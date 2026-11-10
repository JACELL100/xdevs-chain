// Environment configuration. Secrets (DATABASE_URL, TELEGRAM_BOT_TOKEN, webhook URLs/secret) come only from env and
// are never logged.

import { getAddress, isAddress, type Address } from 'viem'
import { predeploys } from '@xdevs/sdk'

export interface Config {
  databaseUrl: string
  rpcUrl: string
  /** WebSocket URL for `eth_subscribe("newHeads")`; `null` disables it (polling only). */
  wsUrl: string | null
  mode: 'all' | 'indexer' | 'api'
  port: number
  host: string
  startBlock: number
  batchSize: number
  pollIntervalMs: number
  confirmations: number
  maxReorgDepth: number
  verifyLogs: boolean
  rpcBatch: boolean
  rpcTimeoutMs: number
  corsOrigins: string[]
  rateLimitRps: number
  rateLimitBurst: number
  trustProxy: 'none' | 'cloudflare' | 'xff'
  healthMaxLag: number
  dustNativeWei: bigint
  knownTokens: Address[]
  canonicalTokenFactory: Address | null
  xnsRegistry: Address
  permit2: Address
  wxds: Address
  feeConfig: Address
  alerts: AlertConfig
}

export interface AlertConfig {
  webhookUrls: string[]
  webhookSecret: string | null
  telegramBotToken: string | null
  telegramApiBase: string
  events: string[]
  maxAgeSeconds: number
  unverified: boolean
  explorerUrl: string | null
}

export const ALL_ALERT_EVENTS = [
  'vault.queued',
  'recovery.approved',
  'recovery.ready',
  'recovery.executed',
  'account.frozen',
  'account.unfrozen',
  'eoaLock.changed',
] as const

type Env = Record<string, string | undefined>

const int = (env: Env, key: string, def: number, min = 0): number => {
  const raw = env[key]
  if (raw === undefined || raw === '') return def
  const n = Number(raw)
  if (!Number.isInteger(n) || n < min) throw new Error(`${key} must be an integer >= ${min}, got "${raw}"`)
  return n
}

const bool = (env: Env, key: string, def: boolean): boolean => {
  const raw = env[key]
  if (raw === undefined || raw === '') return def
  if (/^(1|true|yes|on)$/i.test(raw)) return true
  if (/^(0|false|no|off)$/i.test(raw)) return false
  throw new Error(`${key} must be a boolean, got "${raw}"`)
}

const list = (raw: string | undefined): string[] =>
  (raw ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)

const addr = (env: Env, key: string, def: Address | null): Address | null => {
  const raw = env[key]
  if (raw === undefined || raw === '') return def
  if (!isAddress(raw, { strict: false })) throw new Error(`${key} is not an address: "${raw}"`)
  return getAddress(raw)
}

export function deriveWsUrl(rpcUrl: string): string {
  return rpcUrl.replace(/^http(s?):/i, (_m, s: string) => `ws${s}:`)
}

export function loadConfig(env: Env = process.env): Config {
  const rpcUrl = env.RPC_URL || 'http://127.0.0.1:8545'
  const mode = (env.MODE || 'all') as Config['mode']
  if (!['all', 'indexer', 'api'].includes(mode)) throw new Error(`MODE must be all|indexer|api, got "${mode}"`)
  const trustProxy = (env.TRUST_PROXY || 'none') as Config['trustProxy']
  if (!['none', 'cloudflare', 'xff'].includes(trustProxy)) throw new Error('TRUST_PROXY must be none|cloudflare|xff')
  const ws = env.WS_URL
  const events = list(env.ALERT_EVENTS)
  for (const e of events)
    if (!(ALL_ALERT_EVENTS as readonly string[]).includes(e)) throw new Error(`unknown ALERT_EVENTS entry "${e}"`)
  const known = list(env.KNOWN_TOKENS).map((a) => {
    if (!isAddress(a, { strict: false })) throw new Error(`KNOWN_TOKENS entry is not an address: "${a}"`)
    return getAddress(a)
  })
  return {
    databaseUrl: env.DATABASE_URL || 'postgres://postgres:postgres@127.0.0.1:5432/xdevs_explorer',
    rpcUrl,
    wsUrl: ws === 'off' || ws === 'none' ? null : ws || deriveWsUrl(rpcUrl),
    mode,
    port: int(env, 'PORT', 8080),
    host: env.HOST || '0.0.0.0',
    startBlock: int(env, 'START_BLOCK', 0),
    batchSize: int(env, 'BATCH_SIZE', 20, 1),
    pollIntervalMs: int(env, 'POLL_INTERVAL_MS', 1000, 50),
    confirmations: int(env, 'CONFIRMATIONS', 0),
    maxReorgDepth: int(env, 'MAX_REORG_DEPTH', 64, 1),
    verifyLogs: bool(env, 'VERIFY_LOGS', true),
    rpcBatch: bool(env, 'RPC_BATCH', true),
    rpcTimeoutMs: int(env, 'RPC_TIMEOUT_MS', 15000, 100),
    corsOrigins: list(env.CORS_ORIGINS || '*'),
    rateLimitRps: int(env, 'RATE_LIMIT_RPS', 10, 1),
    rateLimitBurst: int(env, 'RATE_LIMIT_BURST', 40, 1),
    trustProxy,
    healthMaxLag: int(env, 'HEALTH_MAX_LAG', 30, 1),
    dustNativeWei: BigInt(env.DUST_NATIVE_WEI || '1000000000000000'),
    knownTokens: known,
    canonicalTokenFactory: addr(env, 'CANONICAL_TOKEN_FACTORY', null),
    xnsRegistry: addr(env, 'XNS_REGISTRY', predeploys.XNS_REGISTRY)!,
    permit2: addr(env, 'PERMIT2_EXPIRING', predeploys.PERMIT2_EXPIRING)!,
    wxds: addr(env, 'WXDS', predeploys.WXDS)!,
    feeConfig: addr(env, 'FEE_CONFIG', predeploys.FEE_CONFIG)!,
    alerts: {
      webhookUrls: list(env.ALERT_WEBHOOK_URLS),
      webhookSecret: env.ALERT_WEBHOOK_SECRET || null,
      telegramBotToken: env.TELEGRAM_BOT_TOKEN || null,
      telegramApiBase: env.TELEGRAM_API_BASE || 'https://api.telegram.org',
      events: events.length ? events : [...ALL_ALERT_EVENTS],
      maxAgeSeconds: int(env, 'ALERT_MAX_AGE_SECONDS', 3600),
      unverified: bool(env, 'ALERT_UNVERIFIED', false),
      explorerUrl: env.EXPLORER_URL ? env.EXPLORER_URL.replace(/\/+$/, '') : null,
    },
  }
}

/** Config for tests: defaults plus overrides, no env. */
export function testConfig(overrides: Partial<Config> = {}): Config {
  return { ...loadConfig({}), wsUrl: null, ...overrides }
}
