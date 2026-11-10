// Read-only handlers for the explorer API (docs/specs/explorer-api.md). Every handler returns plain JSON-safe data.

import { encodeFunctionData, decodeFunctionResult, getAddress, isAddress, type Hex } from 'viem'
import { normalizeXnsName, isValidXnsLabel } from '@xdevs/sdk'
import type { Config } from '../config.ts'
import type { Queryable } from '../db.ts'
import { decodeLog, toJsonSafe, MAX_UINT160, MAX_UINT256 } from '../decode.ts'
import { codeKind, delegationTarget, PREDEPLOY_LABELS, toHex, type RpcBlock } from '../chain.ts'
import { RpcError, type Rpc } from '../rpc.ts'
import { EOA_LOCK_STATES, pgArray } from '../indexer.ts'

export class HttpError extends Error {
  readonly status: number
  readonly code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}
export const badRequest = (m: string) => new HttpError(400, 'BAD_REQUEST', m)
export const notFound = (m: string) => new HttpError(404, 'NOT_FOUND', m)

export interface ApiContext {
  db: Queryable
  rpc: Rpc
  cfg: Config
  now: () => number
}

export const GRACE_SECONDS = 90 * 86_400
export const VAULT_EXECUTION_WINDOW = 14 * 86_400
const NATIVE = { address: null, name: 'Testnet XDS', symbol: 'tXDS', decimals: 18, known: true }

// ---------------------------------------------------------------- helpers

const hx = (v: unknown): string | null => (v === null || v === undefined ? null : `0x${BigInt(String(v)).toString(16)}`)
const num = (v: unknown): number | null => (v === null || v === undefined ? null : Number(v))
const cs = (a: unknown): string | null => (a ? getAddress(String(a)) : null)

export function parseLimit(raw: string | null): number {
  if (raw === null || raw === '') return 25
  const n = Number(raw)
  if (!Number.isInteger(n) || n < 1 || n > 100) throw badRequest('limit must be an integer in 1..100')
  return n
}

export function parseBool(raw: string | null, def: boolean): boolean {
  if (raw === null || raw === '') return def
  if (raw === 'true' || raw === '1') return true
  if (raw === 'false' || raw === '0') return false
  throw badRequest(`invalid boolean "${raw}"`)
}

export const encodeCursor = (parts: (number | string)[]): string => Buffer.from(JSON.stringify(parts)).toString('base64url')
export function decodeCursor(raw: string | null, shape: ('n' | 's')[]): (number | string)[] | null {
  if (!raw) return null
  try {
    const parts = JSON.parse(Buffer.from(raw, 'base64url').toString('utf8')) as unknown
    if (!Array.isArray(parts) || parts.length !== shape.length) throw new Error()
    parts.forEach((p, i) => {
      if (shape[i] === 'n' ? !Number.isSafeInteger(p) : typeof p !== 'string' || !/^\d+$/.test(p)) throw new Error()
    })
    return parts
  } catch {
    throw badRequest('invalid cursor')
  }
}

export function parseAddress(raw: string): string {
  if (!isAddress(raw, { strict: false })) throw badRequest(`not an address: ${raw}`)
  return raw.toLowerCase()
}

function page<T>(rows: T[], limit: number, key: (r: T) => (number | string)[]) {
  const more = rows.length > limit
  const items = more ? rows.slice(0, limit) : rows
  return { items, nextCursor: more ? encodeCursor(key(items[items.length - 1]!)) : null }
}

function knownTokens(cfg: Config): string[] {
  return [cfg.wxds, ...cfg.knownTokens].map((a) => a.toLowerCase())
}

interface TokenRow {
  t_address: string | null
  t_name: string | null
  t_symbol: string | null
  t_decimals: number | null
  t_canonical: boolean | null
}
const TOKEN_COLS = 'tk.address AS t_address, tk.name AS t_name, tk.symbol AS t_symbol, tk.decimals AS t_decimals, tk.is_canonical AS t_canonical'

function tokenRef(cfg: Config, address: string | null, r: Partial<TokenRow>) {
  if (!address) return NATIVE
  return {
    address: cs(address),
    name: r.t_name ?? null,
    symbol: r.t_symbol ?? null,
    decimals: num(r.t_decimals),
    known: knownTokens(cfg).includes(address) || !!r.t_canonical,
  }
}

async function rpcSafe<T>(p: Promise<T>): Promise<T | null> {
  try {
    return await p
  } catch {
    return null
  }
}

// ---------------------------------------------------------------- mappers

type Row = Record<string, unknown>

export function blockSummary(r: Row) {
  return {
    number: num(r.number)!,
    hash: r.hash as string,
    parentHash: r.parent_hash as string,
    timestamp: num(r.timestamp)!,
    miner: cs(r.miner)!,
    txCount: num(r.tx_count)!,
    gasUsed: hx(r.gas_used)!,
    gasLimit: hx(r.gas_limit)!,
    baseFeePerGas: hx(r.base_fee),
    size: num(r.size),
    fees: { burned: hx(r.burned_fees)!, treasury: hx(r.treasury_fees)!, priority: hx(r.priority_fees)! },
  }
}

const reasons = (v: unknown): string[] => (typeof v === 'string' ? (JSON.parse(v) as string[]) : ((v as string[]) ?? []))

export function txSummary(r: Row) {
  return {
    hash: r.hash as string,
    blockNumber: num(r.block_number)!,
    timestamp: num(r.timestamp)!,
    txIndex: num(r.tx_index)!,
    type: num(r.type)!,
    from: cs(r.from_address)!,
    to: cs(r.to_address),
    contractCreated: cs(r.contract_address),
    value: hx(r.value)!,
    status: Number(r.status) === 1 ? 'success' : 'reverted',
    methodId: (r.method_id as string | null) ?? null,
    fee: hx(r.fee)!,
    spam: !!r.spam,
    spamReasons: reasons(r.spam_reasons),
  }
}

function transfer(cfg: Config, r: Row) {
  return {
    blockNumber: num(r.block_number)!,
    timestamp: num(r.timestamp)!,
    txHash: r.tx_hash as string,
    logIndex: num(r.log_index),
    kind: r.kind as string,
    token: tokenRef(cfg, (r.token as string | null) ?? null, r as Partial<TokenRow>),
    from: cs(r.from_address)!,
    to: cs(r.to_address)!,
    value: hx(r.value)!,
    spam: !!r.spam,
    spamReasons: reasons(r.spam_reasons),
    lookAlikeOf: cs(r.look_alike_of),
  }
}

// ---------------------------------------------------------------- endpoints

export async function health(ctx: ApiContext) {
  const cur = (await ctx.db.query<Row>('SELECT block_number FROM indexer_cursor WHERE id = $1', ['main'])).rows[0]
  const info = (await ctx.db.query<Row>('SELECT chain_id FROM chain_info WHERE id = 1')).rows[0]
  const head = await rpcSafe(ctx.rpc.request<Hex>('eth_blockNumber'))
  const indexed = cur ? num(cur.block_number)! : null
  const headN = head ? Number(BigInt(head)) : null
  const lag = headN !== null && indexed !== null ? Math.max(0, headN - indexed) : null
  return { ok: lag !== null && lag <= ctx.cfg.healthMaxLag, chainId: num(info?.chain_id), indexedBlock: indexed, headBlock: headN, lagBlocks: lag }
}

export async function stats(ctx: ApiContext) {
  const [h, s, last, span] = await Promise.all([
    health(ctx),
    ctx.db.query<Row>('SELECT * FROM chain_stats WHERE id = 1'),
    ctx.db.query<Row>('SELECT base_fee FROM blocks ORDER BY number DESC LIMIT 1'),
    ctx.db.query<Row>('SELECT min(timestamp) AS lo, max(timestamp) AS hi, count(*) AS n FROM (SELECT timestamp FROM blocks WHERE number > 0 ORDER BY number DESC LIMIT 100) x'),
  ])
  const st = s.rows[0] ?? {}
  const sp = span.rows[0] ?? {}
  const n = Number(sp.n ?? 0)
  return {
    chainId: h.chainId,
    latestBlock: h.headBlock,
    indexedBlock: h.indexedBlock,
    lagBlocks: h.lagBlocks,
    avgBlockTimeMs: n > 1 ? Math.round(((Number(sp.hi) - Number(sp.lo)) / (n - 1)) * 1000) : null,
    totalTransactions: Number(st.tx_count ?? 0),
    totalAddresses: Number(st.address_count ?? 0),
    totalTokenTransfers: Number(st.transfer_count ?? 0),
    burnedFees: hx(st.burned ?? 0)!,
    treasuryFees: hx(st.treasury ?? 0)!,
    latestBaseFee: hx(last.rows[0]?.base_fee),
    nativeCurrency: { name: NATIVE.name, symbol: NATIVE.symbol, decimals: 18 },
  }
}

export async function listBlocks(ctx: ApiContext, qs: URLSearchParams) {
  const limit = parseLimit(qs.get('limit'))
  const cur = decodeCursor(qs.get('cursor'), ['n'])
  const rows = (
    await ctx.db.query<Row>(`SELECT * FROM blocks ${cur ? 'WHERE number < $2' : ''} ORDER BY number DESC LIMIT $1`, cur ? [limit + 1, cur[0]] : [limit + 1])
  ).rows
  const p = page(rows, limit, (r) => [num(r.number)!])
  return { items: p.items.map(blockSummary), nextCursor: p.nextCursor }
}

export async function getBlock(ctx: ApiContext, id: string) {
  let row: Row | undefined
  if (/^0x[0-9a-fA-F]{64}$/.test(id)) row = (await ctx.db.query<Row>('SELECT * FROM blocks WHERE hash = $1', [id.toLowerCase()])).rows[0]
  else if (id === 'latest') row = (await ctx.db.query<Row>('SELECT * FROM blocks ORDER BY number DESC LIMIT 1')).rows[0]
  else {
    let n: number
    if (/^\d{1,15}$/.test(id)) n = Number(id)
    else if (/^0x[0-9a-fA-F]{1,13}$/.test(id)) n = Number(BigInt(id))
    else throw badRequest('expected a block number, a 0x hex number, a block hash or "latest"')
    row = (await ctx.db.query<Row>('SELECT * FROM blocks WHERE number = $1', [n])).rows[0]
  }
  if (!row) throw notFound('block not indexed')
  const txs = (await ctx.db.query<Row>('SELECT * FROM transactions WHERE block_number = $1 ORDER BY tx_index LIMIT 1000', [row.number])).rows
  return { ...blockSummary(row), transactions: txs.map(txSummary), commit: await commitOf(ctx, num(row.number)!, row.hash as string) }
}

/** `xdevs_getCommitCertificate`: who finalized the block (Phase 2 BFT). `null` on dev blocks or other nodes. */
async function commitOf(ctx: ApiContext, n: number, hash: string) {
  const c = await rpcSafe(
    ctx.rpc.request<{ height: string; round: string; blockHash: string; signatures: { validator: string }[] } | null>('xdevs_getCommitCertificate', [toHex(n)]),
  )
  if (!c || c.blockHash?.toLowerCase() !== hash) return null
  return { round: Number(BigInt(c.round)), signatures: c.signatures.length, validators: c.signatures.map((s) => s.validator) }
}

export async function listTxs(ctx: ApiContext, qs: URLSearchParams) {
  const limit = parseLimit(qs.get('limit'))
  const cur = decodeCursor(qs.get('cursor'), ['n', 'n'])
  const rows = (
    await ctx.db.query<Row>(
      `SELECT * FROM transactions ${cur ? 'WHERE (block_number, tx_index) < ($2, $3)' : ''} ORDER BY block_number DESC, tx_index DESC LIMIT $1`,
      cur ? [limit + 1, ...cur] : [limit + 1],
    )
  ).rows
  const p = page(rows, limit, (r) => [num(r.block_number)!, num(r.tx_index)!])
  return { items: p.items.map(txSummary), nextCursor: p.nextCursor }
}

export async function getTx(ctx: ApiContext, hash: string) {
  if (!/^0x[0-9a-fA-F]{64}$/.test(hash)) throw badRequest('expected a 32-byte transaction hash')
  const h = hash.toLowerCase()
  const row = (await ctx.db.query<Row>('SELECT t.*, b.hash AS block_hash, b.base_fee AS block_base_fee, b.burn_bps FROM transactions t JOIN blocks b ON b.number = t.block_number WHERE t.hash = $1', [h])).rows[0]
  if (!row) throw notFound('transaction not indexed')
  const [logs, transfers] = await Promise.all([
    ctx.db.query<Row>('SELECT * FROM logs WHERE tx_hash = $1 ORDER BY block_number, log_index', [h]),
    ctx.db.query<Row>(`SELECT tt.*, ${TOKEN_COLS} FROM token_transfers tt LEFT JOIN tokens tk ON tk.address = tt.token WHERE tt.tx_hash = $1 ORDER BY tt.seq`, [h]),
  ])
  const decoded = logs.rows.map((l) => {
    const topics = [l.topic0, l.topic1, l.topic2, l.topic3].filter((t): t is string => typeof t === 'string')
    const ev = decodeLog({ address: l.address as string, topics, data: l.data as string })
    return {
      logIndex: num(l.log_index)!,
      address: cs(l.address)!,
      topics,
      data: l.data as string,
      event: ev?.event ?? null,
      signature: ev?.signature ?? null,
      standard: ev?.standard ?? null,
      args: ev ? (toJsonSafe(ev.args) as Record<string, unknown>) : null,
      _ev: ev,
    }
  })
  const tokenAddrs = new Set<string>()
  const approvals: Record<string, unknown>[] = []
  const permit2 = ctx.cfg.permit2.toLowerCase()
  for (const d of decoded) {
    const ev = d._ev
    if (!ev) continue
    const a = ev.args
    const emitter = d.address.toLowerCase()
    const base = { event: ev.event, owner: cs(a.owner), revokesAll: false }
    if (ev.standard === 'erc20' && ev.event === 'Approval') {
      // XDRC-20 also emits ApprovalWithExpiry for the same write: report that one only.
      const twin = decoded.some((x) => x._ev?.event === 'ApprovalWithExpiry' && x.address === d.address && x.logIndex === d.logIndex + 1)
      if (twin) continue
      const amt = a.value as bigint
      approvals.push({ kind: 'erc20', ...base, token: emitter, spender: cs(a.spender), amount: hx(amt), unlimited: amt === MAX_UINT256, expiry: null })
    } else if (ev.standard === 'xdrc20' && ev.event === 'ApprovalWithExpiry') {
      const amt = a.value as bigint
      approvals.push({ kind: 'xdrc20', ...base, token: emitter, spender: cs(a.spender), amount: hx(amt), unlimited: amt === MAX_UINT256, expiry: amt === 0n ? null : Number(a.expiry) })
    } else if (ev.standard === 'xdrc20' && ev.event === 'AllApprovalsRevoked') {
      approvals.push({ kind: 'xdrc20', ...base, token: emitter, spender: null, amount: null, unlimited: false, expiry: null, revokesAll: true })
    } else if (ev.standard === 'permit2' && emitter === permit2) {
      if (ev.event === 'Approval' || ev.event === 'Permit') {
        const amt = a.amount as bigint
        approvals.push({ kind: 'permit2', ...base, token: String(a.token).toLowerCase(), spender: cs(a.spender), amount: hx(amt), unlimited: amt === MAX_UINT160, expiry: amt === 0n ? null : Number(a.expiration) })
      } else if (ev.event === 'Lockdown') {
        approvals.push({ kind: 'permit2', ...base, token: String(a.token).toLowerCase(), spender: cs(a.spender), amount: '0x0', unlimited: false, expiry: null })
      } else if (ev.event === 'AllRevoked') {
        approvals.push({ kind: 'permit2', ...base, token: null, spender: null, amount: null, unlimited: false, expiry: null, revokesAll: true })
      }
    }
  }
  for (const ap of approvals) if (ap.token) tokenAddrs.add(ap.token as string)
  const toks = tokenAddrs.size
    ? (await ctx.db.query<Row>(`SELECT ${TOKEN_COLS} FROM tokens tk WHERE tk.address = ANY($1::text[])`, [pgArray([...tokenAddrs])])).rows
    : []
  const tokMap = new Map(toks.map((t) => [t.t_address as string, t]))

  const s = txSummary(row)
  const gasUsed = BigInt(String(row.gas_used))
  return {
    ...s,
    blockHash: row.block_hash as string,
    nonce: num(row.nonce)!,
    gas: hx(row.gas)!,
    gasUsed: hx(gasUsed)!,
    gasPrice: hx(row.gas_price),
    maxFeePerGas: hx(row.max_fee_per_gas),
    maxPriorityFeePerGas: hx(row.max_priority_fee_per_gas),
    effectiveGasPrice: hx(row.effective_gas_price)!,
    input: row.input as string,
    authorizationList: row.authorization_list
      ? ((typeof row.authorization_list === 'string' ? JSON.parse(row.authorization_list) : row.authorization_list) as { chainId: string; address: string; nonce: string }[]).map((x) => ({
          chainId: x.chainId, address: cs(x.address), nonce: x.nonce, authority: null,
        }))
      : null,
    feeBreakdown: {
      total: hx(row.fee)!,
      baseFee: hx(row.base_fee_part)!,
      burned: hx(row.burned)!,
      treasury: hx(row.treasury)!,
      priorityFee: hx(row.priority_fee)!,
      burnBps: num(row.burn_bps)!,
    },
    logs: decoded.map(({ _ev, ...rest }) => rest),
    tokenTransfers: transfers.rows.map((r) => transfer(ctx.cfg, r)),
    approvals: approvals.map((ap) => ({ ...ap, token: ap.token ? tokenRef(ctx.cfg, ap.token as string, tokMap.get(ap.token as string) ?? {}) : null })),
  }
}

export async function getAddressSummary(ctx: ApiContext, raw: string) {
  const a = parseAddress(raw)
  const live = await ctx.rpc
    .batch([
      { method: 'eth_getBalance', params: [a, 'latest'] },
      { method: 'eth_getTransactionCount', params: [a, 'latest'] },
      { method: 'eth_getCode', params: [a, 'latest'] },
      { method: 'xdevs_getEoaLock', params: [a, 'latest'] },
    ])
    .catch(() => [] as unknown[])
  const ok = (v: unknown) => v !== undefined && !(v instanceof RpcError)
  const balance = ok(live[0]) ? hx(live[0]) : null
  const nonce = ok(live[1]) ? Number(BigInt(live[1] as string)) : null
  const code = ok(live[2]) ? (live[2] as string) : null
  const lock = ok(live[3]) && live[3] ? (live[3] as { active: boolean; status: string; delegatedTo: string | null }) : null

  const [row, sec, open, names, rev, label, token] = await Promise.all([
    ctx.db.query<Row>('SELECT * FROM addresses WHERE address = $1', [a]),
    ctx.db.query<Row>('SELECT * FROM account_security WHERE account = $1', [a]),
    ctx.db.query<Row>(`SELECT count(*) AS n FROM vault_actions WHERE account = $1 AND status = 'queued' AND execute_after + $2 >= $3`, [a, VAULT_EXECUTION_WINDOW, ctx.now()]),
    ctx.db.query<Row>('SELECT name, expires FROM xns_names WHERE owner = $1 OR addr = $1 ORDER BY expires DESC LIMIT 20', [a]),
    ctx.db.query<Row>('SELECT r.name, n.addr, n.expires FROM xns_reverse r LEFT JOIN xns_names n ON n.name = r.name WHERE r.address = $1', [a]),
    ctx.db.query<Row>('SELECT label FROM labels WHERE address = $1', [a]),
    ctx.db.query<Row>(`SELECT ${TOKEN_COLS} FROM tokens tk WHERE tk.address = $1`, [a]),
  ])
  const r = row.rows[0]
  const s = sec.rows[0]
  const now = ctx.now()
  const rv = rev.rows[0]
  const primaryName = rv && rv.name && rv.addr === a && Number(rv.expires) > now ? (rv.name as string) : null
  const kind = code !== null ? codeKind(code) : null
  const frozenUntil = s ? num(s.frozen_until) : null
  const eoaLock = lock
    ? { status: lock.status, active: lock.active, source: 'rpc' as const }
    : { status: s && s.eoa_lock_state !== null ? (EOA_LOCK_STATES[Number(s.eoa_lock_state)] ?? 'locked') : 'unknown', active: null, source: 'indexer' as const }
  const labels = [PREDEPLOY_LABELS[a], ...label.rows.map((l) => l.label as string)].filter(Boolean) as string[]
  const t = token.rows[0]
  return {
    address: getAddress(a),
    live: balance !== null,
    balance,
    nonce,
    kind,
    delegatedTo: cs(delegationTarget(code) ?? lock?.delegatedTo ?? null),
    isContract: kind !== null ? kind === 'contract' : !!r?.is_contract,
    firstSeenBlock: r ? num(r.first_seen_block) : null,
    lastSeenBlock: r ? num(r.last_seen_block) : null,
    txCount: r ? num(r.tx_count)! : 0,
    xns: {
      primaryName,
      names: names.rows.map((n) => ({ name: n.name as string, expires: num(n.expires)!, status: nameStatus(Number(n.expires), now) })),
    },
    labels,
    token: t ? tokenRef(ctx.cfg, a, t) : null,
    security: {
      accountKind: s ? (s.account_kind as string) : null,
      vaultEnabled: s ? ((s.vault_enabled as boolean | null) ?? null) : null,
      frozen: frozenUntil !== null && frozenUntil > now,
      frozenUntil: frozenUntil !== null && frozenUntil > now ? frozenUntil : null,
      eoaLock,
      eoaKeyRevoked: !!s?.eoa_key_revoked,
      recovery: s?.recovery_id ? { id: s.recovery_id as string, approvals: num(s.recovery_approvals), readyAt: num(s.recovery_ready_at) } : null,
      guardians: s && s.guardian_count !== null ? { count: num(s.guardian_count)!, threshold: num(s.guardian_threshold) } : null,
      openVaultActions: Number(open.rows[0]?.n ?? 0),
    },
  }
}

export function nameStatus(expires: number, now: number): 'active' | 'grace' | 'released' {
  if (now < expires) return 'active'
  if (now < expires + GRACE_SECONDS) return 'grace'
  return 'released'
}

export async function addressTxs(ctx: ApiContext, raw: string, qs: URLSearchParams) {
  const a = parseAddress(raw)
  const limit = parseLimit(qs.get('limit'))
  const cur = decodeCursor(qs.get('cursor'), ['n', 'n'])
  const includeSpam = parseBool(qs.get('includeSpam'), false)
  const who = '(from_address = $1 OR to_address = $1 OR contract_address = $1)'
  const spam = includeSpam ? '' : 'AND (NOT spam OR from_address = $1)'
  const params: unknown[] = [a, limit + 1]
  if (cur) params.push(...cur)
  const rows = (
    await ctx.db.query<Row>(
      `SELECT * FROM transactions WHERE ${who} ${spam} ${cur ? 'AND (block_number, tx_index) < ($3, $4)' : ''} ORDER BY block_number DESC, tx_index DESC LIMIT $2`,
      params,
    )
  ).rows
  const p = page(rows, limit, (r) => [num(r.block_number)!, num(r.tx_index)!])
  const hidden = includeSpam || cur
    ? null
    : Number((await ctx.db.query<Row>(`SELECT count(*) AS n FROM (SELECT 1 FROM transactions WHERE ${who} AND spam AND from_address <> $1 LIMIT 1000) x`, [a])).rows[0]?.n ?? 0)
  return { items: p.items.map(txSummary), nextCursor: p.nextCursor, hiddenSpam: hidden }
}

export async function addressTransfers(ctx: ApiContext, raw: string, qs: URLSearchParams) {
  const a = parseAddress(raw)
  const limit = parseLimit(qs.get('limit'))
  const cur = decodeCursor(qs.get('cursor'), ['n', 'n'])
  const includeSpam = parseBool(qs.get('includeSpam'), false)
  const tokenFilter = qs.get('token')
  const params: unknown[] = [a, limit + 1]
  let where = '(tt.from_address = $1 OR tt.to_address = $1)'
  if (tokenFilter) {
    params.push(parseAddress(tokenFilter))
    where += ` AND tt.token = $${params.length}`
  }
  const spamWhere = includeSpam ? '' : ' AND (NOT tt.spam OR tt.tx_from = $1)'
  let curWhere = ''
  if (cur) {
    params.push(...cur)
    curWhere = ` AND (tt.block_number, tt.seq) < ($${params.length - 1}, $${params.length})`
  }
  const rows = (
    await ctx.db.query<Row>(
      `SELECT tt.*, ${TOKEN_COLS} FROM token_transfers tt LEFT JOIN tokens tk ON tk.address = tt.token
       WHERE ${where}${spamWhere}${curWhere} ORDER BY tt.block_number DESC, tt.seq DESC LIMIT $2`,
      params,
    )
  ).rows
  const p = page(rows, limit, (r) => [num(r.block_number)!, num(r.seq)!])
  const hidden = includeSpam || cur
    ? null
    : Number(
        (
          await ctx.db.query<Row>(
            `SELECT count(*) AS n FROM (SELECT 1 FROM token_transfers tt
               WHERE (tt.from_address = $1 OR tt.to_address = $1) ${tokenFilter ? 'AND tt.token = $2' : ''} AND tt.spam AND tt.tx_from <> $1 LIMIT 1000) x`,
            tokenFilter ? [a, params[2]] : [a],
          )
        ).rows[0]?.n ?? 0,
      )
  return { items: p.items.map((r) => transfer(ctx.cfg, r)), nextCursor: p.nextCursor, hiddenSpam: hidden }
}

const erc20AllowanceAbi = [
  { type: 'function', name: 'allowance', stateMutability: 'view', inputs: [{ type: 'address', name: 'owner' }, { type: 'address', name: 'spender' }], outputs: [{ type: 'uint256' }] },
  { type: 'function', name: 'approve', stateMutability: 'nonpayable', inputs: [{ type: 'address', name: 'spender' }, { type: 'uint256', name: 'value' }], outputs: [{ type: 'bool' }] },
] as const
const permit2Abi = [
  {
    type: 'function', name: 'allowance', stateMutability: 'view',
    inputs: [{ type: 'address', name: 'user' }, { type: 'address', name: 'token' }, { type: 'address', name: 'spender' }],
    outputs: [{ type: 'uint160', name: 'amount' }, { type: 'uint48', name: 'expiration' }, { type: 'uint48', name: 'nonce' }],
  },
  {
    type: 'function', name: 'lockdown', stateMutability: 'nonpayable',
    inputs: [{ type: 'tuple[]', name: 'approvals', components: [{ type: 'address', name: 'token' }, { type: 'address', name: 'spender' }] }],
    outputs: [],
  },
] as const

export async function addressApprovals(ctx: ApiContext, raw: string, qs: URLSearchParams) {
  const a = parseAddress(raw)
  const includeExpired = parseBool(qs.get('includeExpired'), false)
  const now = ctx.now()
  const rows = (
    await ctx.db.query<Row>(
      `SELECT ap.*, ${TOKEN_COLS} FROM approvals ap LEFT JOIN tokens tk ON tk.address = ap.token
       WHERE ap.owner = $1 AND NOT ap.revoked AND ap.amount > 0 ${includeExpired ? '' : 'AND (ap.expiry IS NULL OR ap.expiry > $2)'}
       ORDER BY ap.block_number DESC, ap.log_index DESC LIMIT 500`,
      includeExpired ? [a] : [a, now],
    )
  ).rows
  const permit2 = ctx.cfg.permit2.toLowerCase()
  const calls = rows.map((r) =>
    r.scope === 'permit2'
      ? { method: 'eth_call', params: [{ to: permit2, data: encodeFunctionData({ abi: permit2Abi, functionName: 'allowance', args: [a as Hex, r.token as Hex, r.spender as Hex] }) }, 'latest'] }
      : { method: 'eth_call', params: [{ to: r.token, data: encodeFunctionData({ abi: erc20AllowanceAbi, functionName: 'allowance', args: [a as Hex, r.spender as Hex] }) }, 'latest'] },
  )
  const res = calls.length ? await ctx.rpc.batch(calls).catch(() => [] as unknown[]) : []
  const live = calls.length === 0 || res.length === calls.length
  const items = rows.map((r, i) => {
    const amount = BigInt(String(r.amount))
    const permit = r.scope === 'permit2'
    let remaining: string | null = null
    const out = res[i]
    if (typeof out === 'string' && out !== '0x') {
      try {
        remaining = permit
          ? hx((decodeFunctionResult({ abi: permit2Abi, functionName: 'allowance', data: out as Hex }) as readonly [bigint, number, number])[0])
          : hx(decodeFunctionResult({ abi: erc20AllowanceAbi, functionName: 'allowance', data: out as Hex }) as bigint)
      } catch {
        remaining = null
      }
    }
    const expiry = num(r.expiry)
    return {
      kind: r.kind as string,
      token: tokenRef(ctx.cfg, r.token as string, r),
      spender: cs(r.spender)!,
      amount: hx(amount)!,
      unlimited: permit ? amount === MAX_UINT160 : amount === MAX_UINT256,
      expiry,
      expired: expiry !== null && expiry <= now,
      remaining,
      blockNumber: num(r.block_number)!,
      txHash: r.tx_hash as string,
      revoke: permit
        ? {
            to: getAddress(permit2),
            data: encodeFunctionData({ abi: permit2Abi, functionName: 'lockdown', args: [[{ token: r.token as Hex, spender: r.spender as Hex }]] }),
            description: 'lockdown([(token, spender)]) on Permit2Expiring',
          }
        : {
            to: getAddress(r.token as string),
            data: encodeFunctionData({ abi: erc20AllowanceAbi, functionName: 'approve', args: [r.spender as Hex, 0n] }),
            description: 'approve(spender, 0) on the token',
          },
    }
  })
  return { items, live }
}

export function vaultStatus(status: string, executeAfter: number, now: number): string {
  if (status !== 'queued') return status
  if (now > executeAfter + VAULT_EXECUTION_WINDOW) return 'expired'
  if (now >= executeAfter) return 'ready'
  return 'queued'
}

export async function vaultQueue(ctx: ApiContext, raw: string, qs: URLSearchParams) {
  const a = parseAddress(raw)
  const limit = parseLimit(qs.get('limit'))
  const status = qs.get('status') ?? 'open'
  if (!['open', 'all'].includes(status)) throw badRequest('status must be open or all')
  const cur = decodeCursor(qs.get('cursor'), ['n', 's'])
  const now = ctx.now()
  const params: unknown[] = [a, limit + 1]
  let where = 'account = $1'
  if (status === 'open') {
    params.push(now - VAULT_EXECUTION_WINDOW)
    where += ` AND status = 'queued' AND execute_after >= $${params.length}`
  }
  if (cur) {
    params.push(...cur)
    where += ` AND (queued_block, action_id) < ($${params.length - 1}, $${params.length}::numeric)`
  }
  const rows = (await ctx.db.query<Row>(`SELECT * FROM vault_actions WHERE ${where} ORDER BY queued_block DESC, action_id DESC LIMIT $2`, params)).rows
  const p = page(rows, limit, (r) => [num(r.queued_block)!, String(r.action_id)])
  return {
    items: p.items.map((r) => ({
      account: cs(r.account)!,
      id: hx(r.action_id)!,
      target: cs(r.target)!,
      value: hx(r.value)!,
      data: r.data as string,
      executeAfter: num(r.execute_after)!,
      status: vaultStatus(r.status as string, Number(r.execute_after), now),
      queuedBlock: num(r.queued_block)!,
      queuedTx: r.queued_tx as string,
      cancelledBy: cs(r.cancelled_by),
      finalBlock: num(r.final_block),
      finalTx: (r.final_tx as string | null) ?? null,
    })),
    nextCursor: p.nextCursor,
  }
}

export async function getName(ctx: ApiContext, raw: string) {
  let name: string
  try {
    name = normalizeXnsName(raw)
  } catch (e) {
    throw badRequest((e as Error).message)
  }
  const r = (await ctx.db.query<Row>('SELECT * FROM xns_names WHERE name = $1', [name])).rows[0]
  if (!r) throw notFound(`${name} has never been registered`)
  const now = ctx.now()
  const expires = Number(r.expires)
  const status = nameStatus(expires, now)
  const primary = r.addr
    ? (await ctx.db.query<Row>('SELECT address FROM xns_reverse WHERE name = $1 AND address = $2', [name, r.addr])).rows[0]
    : undefined
  return {
    name,
    label: r.label as string,
    node: r.node as string,
    labelhash: r.labelhash as string,
    owner: cs(r.owner)!,
    address: status === 'active' ? cs(r.addr) : null,
    expires,
    graceEndsAt: expires + GRACE_SECONDS,
    status,
    registeredBlock: num(r.registered_block)!,
    registeredTx: r.registered_tx as string,
    isPrimaryFor: status === 'active' && primary ? cs(primary.address) : null,
  }
}

export async function search(ctx: ApiContext, qs: URLSearchParams) {
  const q = (qs.get('q') ?? '').trim()
  if (q.length > 200) throw badRequest('query too long')
  const results: { type: string; id: string; label: string | null }[] = []
  if (!q) return { query: q, results }
  if (/^\d{1,15}$/.test(q)) {
    const b = (await ctx.db.query<Row>('SELECT number FROM blocks WHERE number = $1', [Number(q)])).rows[0]
    if (b) results.push({ type: 'block', id: q, label: `Block ${q}` })
  } else if (/^0x[0-9a-fA-F]{64}$/.test(q)) {
    const h = q.toLowerCase()
    const t = (await ctx.db.query<Row>('SELECT hash FROM transactions WHERE hash = $1', [h])).rows[0]
    if (t) results.push({ type: 'tx', id: h, label: 'Transaction' })
    const b = (await ctx.db.query<Row>('SELECT number FROM blocks WHERE hash = $1', [h])).rows[0]
    if (b) results.push({ type: 'block', id: String(b.number), label: `Block ${b.number}` })
  } else if (isAddress(q, { strict: false })) {
    const a = q.toLowerCase()
    const n = (await ctx.db.query<Row>('SELECT name FROM xns_reverse WHERE address = $1', [a])).rows[0]
    results.push({ type: 'address', id: getAddress(a), label: (n?.name as string | undefined) ?? PREDEPLOY_LABELS[a] ?? null })
  } else {
    const lowerQ = q.toLowerCase()
    let exact: string | null = null
    try {
      exact = normalizeXnsName(lowerQ)
    } catch {
      exact = null
    }
    if (exact) {
      const r = (await ctx.db.query<Row>('SELECT name, addr, expires FROM xns_names WHERE name = $1', [exact])).rows[0]
      if (r) results.push({ type: 'name', id: exact, label: r.addr && Number(r.expires) > ctx.now() ? getAddress(r.addr as string) : 'not resolving' })
    }
    const prefix = lowerQ.replace(/\.xdevs$/, '')
    if (/^[a-z0-9-]{1,63}$/.test(prefix) && (prefix.length < 3 || isValidXnsLabel(prefix) || /^[a-z0-9-]+$/.test(prefix))) {
      const rows = (await ctx.db.query<Row>(`SELECT name FROM xns_names WHERE label LIKE $1 ORDER BY label LIMIT 10`, [`${prefix}%`])).rows
      for (const r of rows) if (r.name !== exact) results.push({ type: 'name', id: r.name as string, label: null })
    }
  }
  return { query: q, results }
}

export type { RpcBlock }
