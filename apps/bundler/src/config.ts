// Bundler configuration: environment variables -> typed config (with safe defaults), operator key loading.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { type Address, type Hex, getAddress, isAddress, parseEther } from 'viem'
import { decryptKeystore } from './keystore.ts'
import { ENTRY_POINT_V09 } from './userop.ts'

export type TrustProxy = 'none' | 'cloudflare' | 'xff'
export type SponsorPolicyMode = 'off' | 'activation'

export type BundlerConfig = {
  // --- chain
  entryPoint: Address
  /** Refuse to start when the RPC reports another chain id (0 = do not check). */
  expectedChainId: number
  /** Receives the bundle fees (default: the operator). */
  beneficiary?: Address
  // --- operator
  /** Refuse to submit bundles (and new ops) below this operator balance (wei). */
  operatorMinBalance: bigint
  /** `bundler_operator_balance_low` alarm threshold (wei). */
  operatorLowBalance: bigint
  // --- op validation
  minPriorityFeePerGas: bigint
  /** maxFeePerGas must be >= baseFee * pct / 100 + minPriorityFeePerGas. */
  baseFeeMarginPct: bigint
  maxOpBytes: number
  maxVerificationGas: bigint
  maxCallGas: bigint
  /** An op with validUntil closer than this (seconds) is rejected; at bundle time half of it. */
  minValidUntilSeconds: number
  requirePaymasterStake: boolean
  minStake: bigint
  minUnstakeDelay: number
  // --- mempool / ERC-7562 limits
  maxMempoolOps: number
  sameSenderMempoolCount: number
  sameUnstakedEntityMempoolCount: number
  throttledEntityMempoolCount: number
  throttledEntityBundleCount: number
  throttledEntityLiveBlocks: number
  /** Pending ops older than this are dropped (seconds). */
  mempoolTtlSeconds: number
  // --- reputation (ERC-7562 §reputation)
  minInclusionRateDenominator: number
  throttlingSlack: number
  banSlack: number
  /** Entities that are never throttled / banned (e.g. a paymaster the operator runs). */
  reputationAllowlist: Address[]
  // --- bundling
  /** Background bundling period (ms); 0 = only `bundleNow()` (tests). */
  bundleIntervalMs: number
  /** Delay between the first new op and the bundle attempt, to batch ops arriving together (ms). */
  bundleCollectMs: number
  maxBundleOps: number
  maxBundleGas: bigint
  maxInflightBundles: number
  receiptTimeoutMs: number
  // --- sponsorship policy (XdevsPaymaster open / allowlist mode)
  sponsorPolicy: SponsorPolicyMode
  sponsorPaymaster: Address
  /** Sponsors the policy applies to (empty = every sponsor of `sponsorPaymaster`). */
  sponsorPolicySponsors: Address[]
  /** Sponsored ops per account through this bundler; the first one must deploy the account. */
  sponsorMaxOpsPerSender: number
  /** Sponsored ops per client IP per hour (0 = unlimited). */
  sponsoredOpsPerIpPerHour: number
  // --- HTTP
  trustProxy: TrustProxy
  /** Peers allowed to set CF-Connecting-IP / X-Forwarded-For (cloudflared runs on localhost). */
  trustedProxies: string[]
  corsOrigins: string[]
  maxBodyBytes: number
  maxBatch: number
  rateLimitRps: number
  rateLimitBurst: number
  sendPerMinute: number
  estimatePerMinute: number
  maxConcurrentSimulations: number
  // --- persistence / logs
  dataDir?: string
  logLevel: 'debug' | 'info' | 'warn' | 'error' | 'silent'
}

export const PREDEPLOYED_PAYMASTER: Address = '0x0000000000000000000000000000000000001007'

export const DEFAULT_CONFIG: BundlerConfig = {
  entryPoint: ENTRY_POINT_V09,
  expectedChainId: 0,
  operatorMinBalance: parseEther('0.01'),
  operatorLowBalance: parseEther('0.5'),
  minPriorityFeePerGas: 1_000_000n,
  baseFeeMarginPct: 100n,
  maxOpBytes: 16_384,
  maxVerificationGas: 3_000_000n,
  maxCallGas: 10_000_000n,
  minValidUntilSeconds: 30,
  requirePaymasterStake: true,
  minStake: parseEther('1'),
  minUnstakeDelay: 86_400,
  maxMempoolOps: 1_000,
  sameSenderMempoolCount: 4,
  sameUnstakedEntityMempoolCount: 10,
  throttledEntityMempoolCount: 4,
  throttledEntityBundleCount: 4,
  throttledEntityLiveBlocks: 10,
  mempoolTtlSeconds: 1_800,
  minInclusionRateDenominator: 10,
  throttlingSlack: 10,
  banSlack: 50,
  reputationAllowlist: [],
  bundleIntervalMs: 1_000,
  bundleCollectMs: 150,
  maxBundleOps: 16,
  maxBundleGas: 15_000_000n,
  maxInflightBundles: 2,
  receiptTimeoutMs: 120_000,
  sponsorPolicy: 'activation',
  sponsorPaymaster: PREDEPLOYED_PAYMASTER,
  sponsorPolicySponsors: [],
  sponsorMaxOpsPerSender: 1,
  sponsoredOpsPerIpPerHour: 30,
  trustProxy: 'none',
  trustedProxies: ['127.0.0.1', '::1', '::ffff:127.0.0.1'],
  corsOrigins: ['*'],
  maxBodyBytes: 131_072,
  maxBatch: 10,
  rateLimitRps: 20,
  rateLimitBurst: 60,
  sendPerMinute: 30,
  estimatePerMinute: 60,
  maxConcurrentSimulations: 4,
  logLevel: 'info',
}

type Env = Record<string, string | undefined>

const int = (env: Env, name: string, def: number, min = 0): number => {
  const v = env[name]
  if (v === undefined || v === '') return def
  const n = Number(v)
  if (!Number.isInteger(n) || n < min) throw new Error(`${name} must be an integer >= ${min}`)
  return n
}
const big = (env: Env, name: string, def: bigint): bigint => {
  const v = env[name]
  if (v === undefined || v === '') return def
  if (!/^\d+$/.test(v)) throw new Error(`${name} must be a non-negative integer (wei)`)
  return BigInt(v)
}
/** Amounts in tXDS (decimal), e.g. `0.5`. */
const ether = (env: Env, name: string, def: bigint): bigint => {
  const v = env[name]
  if (v === undefined || v === '') return def
  if (!/^\d+(\.\d{1,18})?$/.test(v)) throw new Error(`${name} must be a decimal tXDS amount, e.g. 0.5`)
  return parseEther(v)
}
const bool = (env: Env, name: string, def: boolean): boolean => {
  const v = env[name]?.toLowerCase()
  if (v === undefined || v === '') return def
  if (['1', 'true', 'yes', 'on'].includes(v)) return true
  if (['0', 'false', 'no', 'off'].includes(v)) return false
  throw new Error(`${name} must be true or false`)
}
const addr = (v: string, name: string): Address => {
  if (!isAddress(v, { strict: false })) throw new Error(`${name}: invalid address ${v}`)
  return getAddress(v)
}
const list = (env: Env, name: string): string[] | undefined => {
  const v = env[name]
  if (v === undefined) return undefined
  return v
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
}

/** Parses the bundler settings from the environment (see README "Configuration"). Throws on invalid values. */
export function loadConfig(env: Env): BundlerConfig {
  const d = DEFAULT_CONFIG
  const trustProxy = (env.TRUST_PROXY || 'none') as TrustProxy
  if (!['none', 'cloudflare', 'xff'].includes(trustProxy)) throw new Error('TRUST_PROXY must be none|cloudflare|xff')
  const sponsorPolicy = (env.SPONSOR_POLICY || d.sponsorPolicy) as SponsorPolicyMode
  if (!['off', 'activation'].includes(sponsorPolicy)) throw new Error('SPONSOR_POLICY must be off|activation')
  const logLevel = (env.LOG_LEVEL || (env.QUIET ? 'silent' : d.logLevel)) as BundlerConfig['logLevel']
  if (!['debug', 'info', 'warn', 'error', 'silent'].includes(logLevel)) throw new Error('LOG_LEVEL must be debug|info|warn|error|silent')
  const cfg: BundlerConfig = {
    entryPoint: env.ENTRY_POINT ? addr(env.ENTRY_POINT, 'ENTRY_POINT') : d.entryPoint,
    expectedChainId: int(env, 'CHAIN_ID', d.expectedChainId),
    beneficiary: env.BENEFICIARY ? addr(env.BENEFICIARY, 'BENEFICIARY') : undefined,
    operatorMinBalance: ether(env, 'OPERATOR_MIN_BALANCE', d.operatorMinBalance),
    operatorLowBalance: ether(env, 'OPERATOR_LOW_BALANCE', d.operatorLowBalance),
    minPriorityFeePerGas: big(env, 'MIN_PRIORITY_FEE_PER_GAS', d.minPriorityFeePerGas),
    baseFeeMarginPct: BigInt(int(env, 'BASE_FEE_MARGIN_PCT', Number(d.baseFeeMarginPct), 100)),
    maxOpBytes: int(env, 'MAX_OP_BYTES', d.maxOpBytes, 1_024),
    maxVerificationGas: big(env, 'MAX_VERIFICATION_GAS', d.maxVerificationGas),
    maxCallGas: big(env, 'MAX_CALL_GAS', d.maxCallGas),
    minValidUntilSeconds: int(env, 'MIN_VALID_UNTIL_SECONDS', d.minValidUntilSeconds),
    requirePaymasterStake: bool(env, 'REQUIRE_PAYMASTER_STAKE', d.requirePaymasterStake),
    minStake: ether(env, 'MIN_STAKE', d.minStake),
    minUnstakeDelay: int(env, 'MIN_UNSTAKE_DELAY', d.minUnstakeDelay),
    maxMempoolOps: int(env, 'MAX_MEMPOOL_OPS', d.maxMempoolOps, 1),
    sameSenderMempoolCount: int(env, 'SAME_SENDER_MEMPOOL_COUNT', d.sameSenderMempoolCount, 1),
    sameUnstakedEntityMempoolCount: int(env, 'SAME_UNSTAKED_ENTITY_MEMPOOL_COUNT', d.sameUnstakedEntityMempoolCount, 1),
    throttledEntityMempoolCount: int(env, 'THROTTLED_ENTITY_MEMPOOL_COUNT', d.throttledEntityMempoolCount, 1),
    throttledEntityBundleCount: int(env, 'THROTTLED_ENTITY_BUNDLE_COUNT', d.throttledEntityBundleCount, 1),
    throttledEntityLiveBlocks: int(env, 'THROTTLED_ENTITY_LIVE_BLOCKS', d.throttledEntityLiveBlocks, 1),
    mempoolTtlSeconds: int(env, 'MEMPOOL_TTL_SECONDS', d.mempoolTtlSeconds, 10),
    minInclusionRateDenominator: int(env, 'MIN_INCLUSION_RATE_DENOMINATOR', d.minInclusionRateDenominator, 1),
    throttlingSlack: int(env, 'THROTTLING_SLACK', d.throttlingSlack),
    banSlack: int(env, 'BAN_SLACK', d.banSlack),
    reputationAllowlist: (list(env, 'REPUTATION_ALLOWLIST') ?? []).map((a) => addr(a, 'REPUTATION_ALLOWLIST')),
    bundleIntervalMs: int(env, 'BUNDLE_INTERVAL_MS', d.bundleIntervalMs),
    bundleCollectMs: int(env, 'BUNDLE_COLLECT_MS', d.bundleCollectMs),
    maxBundleOps: int(env, 'MAX_BUNDLE_OPS', d.maxBundleOps, 1),
    maxBundleGas: big(env, 'MAX_BUNDLE_GAS', d.maxBundleGas),
    maxInflightBundles: int(env, 'MAX_INFLIGHT_BUNDLES', d.maxInflightBundles, 1),
    receiptTimeoutMs: int(env, 'RECEIPT_TIMEOUT_MS', d.receiptTimeoutMs, 1_000),
    sponsorPolicy,
    sponsorPaymaster: env.SPONSOR_PAYMASTER ? addr(env.SPONSOR_PAYMASTER, 'SPONSOR_PAYMASTER') : d.sponsorPaymaster,
    sponsorPolicySponsors: (list(env, 'SPONSOR_POLICY_SPONSORS') ?? []).map((a) => addr(a, 'SPONSOR_POLICY_SPONSORS')),
    sponsorMaxOpsPerSender: int(env, 'SPONSOR_MAX_OPS_PER_SENDER', d.sponsorMaxOpsPerSender, 1),
    sponsoredOpsPerIpPerHour: int(env, 'SPONSORED_OPS_PER_IP_PER_HOUR', d.sponsoredOpsPerIpPerHour),
    trustProxy,
    trustedProxies: list(env, 'TRUSTED_PROXIES') ?? d.trustedProxies,
    corsOrigins: list(env, 'CORS_ORIGINS') ?? d.corsOrigins,
    maxBodyBytes: int(env, 'MAX_BODY_BYTES', d.maxBodyBytes, 1_024),
    maxBatch: int(env, 'MAX_BATCH', d.maxBatch, 1),
    rateLimitRps: int(env, 'RATE_LIMIT_RPS', d.rateLimitRps, 1),
    rateLimitBurst: int(env, 'RATE_LIMIT_BURST', d.rateLimitBurst, 1),
    sendPerMinute: int(env, 'SEND_PER_MINUTE', d.sendPerMinute, 1),
    estimatePerMinute: int(env, 'ESTIMATE_PER_MINUTE', d.estimatePerMinute, 1),
    maxConcurrentSimulations: int(env, 'MAX_CONCURRENT_SIMULATIONS', d.maxConcurrentSimulations, 1),
    dataDir: env.DATA_DIR || undefined,
    logLevel,
  }
  if (cfg.banSlack < cfg.throttlingSlack) throw new Error('BAN_SLACK must be >= THROTTLING_SLACK')
  if (cfg.operatorLowBalance < cfg.operatorMinBalance) throw new Error('OPERATOR_LOW_BALANCE must be >= OPERATOR_MIN_BALANCE')
  return cfg
}

const KEY_RE = /^0x[0-9a-fA-F]{64}$/

/**
 * Operator key, from (first match):
 *   BUNDLER_PRIVATE_KEY_FILE   file holding `0x…` (e.g. a systemd credential: `$CREDENTIALS_DIRECTORY/operator-key`)
 *   BUNDLER_KEYSTORE           Web3 Secret Storage v3 JSON file + BUNDLER_KEYSTORE_PASSWORD_FILE (or _PASSWORD)
 *   BUNDLER_PRIVATE_KEY        the key itself (dev / tests; it stays in the process environment)
 * The key is never logged; error messages never contain it.
 */
export function loadOperatorKey(env: Env): Hex {
  const credDir = env.CREDENTIALS_DIRECTORY
  const keyFile = env.BUNDLER_PRIVATE_KEY_FILE || (credDir ? existing(join(credDir, 'operator-key')) : undefined)
  if (keyFile) {
    const k = readFileSync(keyFile, 'utf8').trim()
    if (!KEY_RE.test(k)) throw new Error(`${keyFile} does not contain a 0x-prefixed 32-byte hex key`)
    return k as Hex
  }
  if (env.BUNDLER_KEYSTORE) {
    const pwFile = env.BUNDLER_KEYSTORE_PASSWORD_FILE || (credDir ? existing(join(credDir, 'keystore-password')) : undefined)
    const password = pwFile ? readFileSync(pwFile, 'utf8').replace(/\r?\n$/, '') : env.BUNDLER_KEYSTORE_PASSWORD
    if (password === undefined) throw new Error('BUNDLER_KEYSTORE needs BUNDLER_KEYSTORE_PASSWORD_FILE (or BUNDLER_KEYSTORE_PASSWORD)')
    return decryptKeystore(JSON.parse(readFileSync(env.BUNDLER_KEYSTORE, 'utf8')), password)
  }
  const k = env.BUNDLER_PRIVATE_KEY
  if (!k || !KEY_RE.test(k)) {
    throw new Error('operator key missing: set BUNDLER_PRIVATE_KEY_FILE, BUNDLER_KEYSTORE or BUNDLER_PRIVATE_KEY (0x + 64 hex)')
  }
  return k as Hex
}

function existing(p: string): string | undefined {
  try {
    readFileSync(p)
    return p
  } catch {
    return undefined
  }
}
