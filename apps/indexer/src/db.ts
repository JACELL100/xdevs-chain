// Small database abstraction so the same code runs on node-postgres (production: Neon / Supabase / any Postgres)
// and PGlite (in-process Postgres for tests).

import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

export interface Queryable {
  query<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<{ rows: T[] }>
}

export interface Db extends Queryable {
  /** Runs `fn` in one transaction (BEGIN / COMMIT, ROLLBACK on throw). */
  transaction<T>(fn: (q: Queryable) => Promise<T>): Promise<T>
  /** Runs a multi-statement SQL script (no parameters). */
  exec(sql: string): Promise<void>
  close(): Promise<void>
}

export async function createPgDb(connectionString: string, max = 5): Promise<Db> {
  const pg = await import('pg')
  // int8 (bigint) as JS number is fine: block numbers and timestamps stay far below 2^53. NUMERIC stays a string.
  pg.default.types.setTypeParser(20, (v: string) => Number(v))
  const ssl = /sslmode=(require|verify)/.test(connectionString) || /neon\.tech|supabase\.co/.test(connectionString)
  const pool = new pg.default.Pool({
    connectionString,
    max,
    ssl: ssl ? { rejectUnauthorized: !/sslmode=no-verify/.test(connectionString) } : undefined,
    idleTimeoutMillis: 30_000,
  })
  pool.on('error', (err) => console.error('[db] idle client error:', err.message))
  return {
    query: (sql, params) => pool.query(sql, params as unknown[]) as never,
    async transaction(fn) {
      const client = await pool.connect()
      try {
        await client.query('BEGIN')
        const out = await fn({ query: (sql, params) => client.query(sql, params as unknown[]) as never })
        await client.query('COMMIT')
        return out
      } catch (err) {
        await client.query('ROLLBACK').catch(() => {})
        throw err
      } finally {
        client.release()
      }
    },
    async exec(sql) {
      await pool.query(sql)
    },
    close: () => pool.end(),
  }
}

/** In-process Postgres (WASM). Used by the tests; also handy for a quick local demo (`DATABASE_URL=pglite://`). */
export async function createPgliteDb(dataDir?: string): Promise<Db> {
  const { PGlite, types } = await import('@electric-sql/pglite')
  const pg = new PGlite(dataDir, {
    parsers: { [types.INT8]: (v: string) => Number(v), [types.NUMERIC]: (v: string) => v },
  })
  await pg.waitReady
  // PGlite runs one query at a time; serialize transactions so concurrent API reads never interleave with them.
  let chain: Promise<unknown> = Promise.resolve()
  const serial = <T>(fn: () => Promise<T>): Promise<T> => {
    const run = chain.then(fn, fn)
    chain = run.catch(() => {})
    return run
  }
  return {
    query: (sql, params) => serial(() => pg.query(sql, params)) as never,
    transaction: (fn) => serial(() => pg.transaction((tx) => fn({ query: (sql, params) => tx.query(sql, params) as never }))),
    exec: (sql) => serial(async () => void (await pg.exec(sql))),
    close: () => pg.close(),
  }
}

export async function createDb(url: string): Promise<Db> {
  if (url.startsWith('pglite://')) return createPgliteDb(url.slice('pglite://'.length) || undefined)
  return createPgDb(url)
}

const MIGRATIONS_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', 'migrations')

/** Applies `migrations/*.sql` in name order, each once, recorded in `schema_migrations`. */
export async function migrate(db: Db, dir = MIGRATIONS_DIR): Promise<string[]> {
  await db.exec(
    'CREATE TABLE IF NOT EXISTS schema_migrations (version text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())',
  )
  const done = new Set((await db.query<{ version: string }>('SELECT version FROM schema_migrations')).rows.map((r) => r.version))
  const applied: string[] = []
  for (const file of readdirSync(dir).filter((f) => f.endsWith('.sql')).sort()) {
    if (done.has(file)) continue
    const sql = readFileSync(join(dir, file), 'utf8')
    await db.transaction(async (q) => {
      // node-postgres runs multi-statement text when there are no parameters; PGlite needs exec, which the
      // transaction handle lacks, so split on statement boundaries (the migration files keep one statement per ';').
      for (const stmt of splitSql(sql)) await q.query(stmt)
      await q.query('INSERT INTO schema_migrations (version) VALUES ($1)', [file])
    })
    applied.push(file)
  }
  return applied
}

export function splitSql(sql: string): string[] {
  return sql
    .replace(/--.*$/gm, '')
    .split(/;\s*(?:\r?\n|$)/)
    .map((s) => s.trim())
    .filter(Boolean)
}

/**
 * Multi-row INSERT in chunks (Postgres allows 65535 bind parameters per statement).
 * `suffix` is appended verbatim (e.g. `ON CONFLICT DO NOTHING RETURNING hash`). Returns all returned rows.
 */
export async function insertMany<T = Record<string, unknown>>(
  q: Queryable,
  table: string,
  columns: readonly string[],
  rows: readonly unknown[][],
  suffix = '',
  casts: Record<string, string> = {},
): Promise<T[]> {
  const out: T[] = []
  if (!rows.length) return out
  const perChunk = Math.max(1, Math.floor(30_000 / columns.length))
  for (let i = 0; i < rows.length; i += perChunk) {
    const chunk = rows.slice(i, i + perChunk)
    const params: unknown[] = []
    const values = chunk.map((row) => {
      const ph = row.map((v, j) => {
        params.push(v)
        const cast = casts[columns[j]!]
        return `$${params.length}${cast ? `::${cast}` : ''}`
      })
      return `(${ph.join(',')})`
    })
    const res = await q.query<T>(`INSERT INTO ${table} (${columns.join(',')}) VALUES ${values.join(',')} ${suffix}`, params)
    out.push(...res.rows)
  }
  return out
}
