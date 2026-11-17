// Demo data (NEXT_PUBLIC_DEMO=1): a deterministic, realistic Enclave testnet so the design can be reviewed without a
// live chain. Everything derives from the block number, so any block / tx / address URL resolves consistently, and
// the head advances once per second like the real chain. The UI labels every demo page "Demo data".

import { encodeFunctionData, getAddress, keccak256, parseAbi, toHex, encodeAbiParameters, toEventSelector, pad } from 'viem'
import { predeploys } from '@xdevs/sdk'
import { consensusAddress } from './chainstats.ts'
import type {
  AddressSummary,
  Approval,
  BlockDetail,
  BlockSummary,
  CommitDetail,
  DecodedLog,
  NameInfo,
  TokenPage,
  TokenTransfer,
  TxDetail,
  TxSummary,
  ValidatorRow,
  ValidatorSet,
  VaultAction,
} from './types.ts'

/** Demo genesis: 2026-11-02 00:00:00 UTC. One block per second since then. */
export const DEMO_GENESIS = 1793577600
const E18 = 10n ** 18n
const hx = (v: bigint | number) => `0x${BigInt(v).toString(16)}`

export function demoHead(nowMs = Date.now()): number {
  return Math.max(1, Math.floor(nowMs / 1000) - DEMO_GENESIS)
}

// --------------------------------------------------------------------------------------------------------------
// Deterministic randomness
// --------------------------------------------------------------------------------------------------------------

function rand(seed: string): () => number {
  let h = BigInt(keccak256(toHex(seed)))
  return () => {
    h = (h * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n)
    return Number(h >> 11n) / 2 ** 53
  }
}
const hash = (seed: string) => keccak256(toHex(seed))
const addr = (seed: string) => getAddress(`0x${hash(seed).slice(26)}`)

// Tx hashes carry their (block, index) in the last 14 hex chars, scrambled by an odd multiplier mod 2^56, so a hash
// from a URL maps back to its block without a lookup table. The rest is keccak noise.
const M56 = 1n << 56n
const K = 0x9e3779b97f4a7cn | 1n
function inverseOdd(k: bigint, m: bigint): bigint {
  let [a, b, x0, x1] = [k, m, 1n, 0n]
  while (b) {
    const q = a / b
    ;[a, b] = [b, a - q * b]
    ;[x0, x1] = [x1, x0 - q * x1]
  }
  return ((x0 % m) + m) % m
}
const KINV = inverseOdd(K, M56)

export function demoTxHash(block: number, index: number): string {
  const v = ((BigInt(block) << 16n) | BigInt(index)) * K % M56
  return `${hash(`tx:${block}:${index}`).slice(0, 52)}${v.toString(16).padStart(14, '0')}`
}

export function decodeDemoTxHash(h: string): { block: number; index: number } | null {
  if (!/^0x[0-9a-f]{64}$/.test(h)) return null
  const v = (BigInt(`0x${h.slice(-14)}`) * KINV) % M56
  const block = Number(v >> 16n)
  const index = Number(v & 0xffffn)
  return demoTxHash(block, index) === h ? { block, index } : null
}

// --------------------------------------------------------------------------------------------------------------
// Cast
// --------------------------------------------------------------------------------------------------------------

const VALIDATOR_NAMES = ['aurora', 'basalt', 'cobalt', 'drift', 'ember', 'fjord', 'granite', 'harbor'] as const

export const DEMO_VALIDATORS: ValidatorRow[] = VALIDATOR_NAMES.map((n, i) => {
  const stake = [412_500n, 386_200n, 351_900n, 298_400n, 264_750n, 233_100n, 180_000n, 25_000n][i]! * E18
  const status = i === 6 ? 'jailed' : i === 7 ? 'candidate' : 'active'
  return {
    operator: addr(`validator:${n}`),
    consensusKey: hash(`consensus:${n}`),
    status,
    totalStake: hx(stake),
    selfStake: hx(stake / 4n),
    votingPower: status === 'active' ? hx(stake / E18) : '0x0',
    commissionBps: [500, 700, 500, 1000, 800, 500, 1200, 900][i]!,
    uptimeBps: status === 'active' ? [9998, 9996, 9999, 9991, 9987, 9994, 0, 0][i]! : status === 'jailed' ? 4620 : null,
    jailedUntil: status === 'jailed' ? DEMO_GENESIS + 1_400_000 : null,
    inSigningSet: status === 'active',
    name: `${n}.xdevs`,
  }
})
const ACTIVE = DEMO_VALIDATORS.filter((v) => v.status === 'active')

export const DEMO_TOKENS = {
  eUSD: { address: getAddress('0x7e5f4552091a69125d5dfcb7b8c2659029395bdf'), name: 'Enclave USD', symbol: 'eUSD', decimals: 6, supply: 25_000_000n * 10n ** 6n, xdrc20: true },
  NMB: { address: getAddress('0x2b5ad5c4795c026514f8317c7a215e218dccd6cf'), name: 'Nimbus', symbol: 'NMB', decimals: 18, supply: 100_000_000n * E18, xdrc20: true },
  WXDS: { address: getAddress(predeploys.WXDS), name: 'Wrapped EnclaveXDS', symbol: 'WXDS', decimals: 18, supply: 1_482_330n * E18, xdrc20: false },
} as const
type DemoToken = (typeof DEMO_TOKENS)[keyof typeof DEMO_TOKENS]
const TOKEN_LIST: DemoToken[] = Object.values(DEMO_TOKENS)

/** Named people. `alice` is the showcase smart account: Guardian Vault, EOA lock, guardians, a queued action. */
export const DEMO_PEOPLE = {
  alice: { address: getAddress('0x6813eb9362372eef6200f3b1dbc3f819671cba69'), name: 'alice.xdevs' },
  bob: { address: getAddress('0x1eff47bc3a10a45d4b230b5d10e37751fe6aa718'), name: 'bob.xdevs' },
  faucet: { address: getAddress('0xe1ab8145f7e55dc933d51a18c793f901a3a0b276'), name: 'faucet.xdevs' },
  carol: { address: getAddress('0xe57bfe9f44b819898f47bf37e5af72a0783e1141'), name: null },
  dave: { address: getAddress('0xd41c057fd1c78805aac12b0a94a405c0461a6fbb'), name: null },
} as const
const PEOPLE = Object.values(DEMO_PEOPLE)
const crowd = (r: () => number) => (r() < 0.55 ? PEOPLE[Math.floor(r() * PEOPLE.length)]!.address : addr(`user:${Math.floor(r() * 400)}`))

export const DEMO_NAMES: Record<string, string> = Object.fromEntries([
  ...PEOPLE.filter((p) => p.name).map((p) => [p.address.toLowerCase(), p.name!]),
  ...DEMO_VALIDATORS.map((v) => [v.operator.toLowerCase(), v.name!]),
])

// --------------------------------------------------------------------------------------------------------------
// Transactions
// --------------------------------------------------------------------------------------------------------------

type Kind = 'native' | 'token' | 'wxds' | 'delegate' | 'xns' | 'vault-queue' | 'approve' | 'deploy'

const erc20Abi = parseAbi([
  'function transfer(address to, uint256 value) returns (bool)',
  'function approve(address spender, uint256 value, uint64 expiry) returns (bool)',
])
const wxdsAbi = parseAbi(['function deposit() payable'])
const stakingAbi = parseAbi(['function delegate(address validator) payable'])
const xnsAbi = parseAbi(['function register(string label, address owner, uint256 duration) payable returns (bytes32 node, uint256 expires)'])
const accountAbi = parseAbi(['function execute(address target, uint256 value, bytes data) returns (bytes)'])

const T_TRANSFER = toEventSelector('Transfer(address,address,uint256)')
const T_APPROVAL_EXP = toEventSelector('ApprovalWithExpiry(address,address,uint256,uint64)')
const T_DEPOSIT = toEventSelector('Deposit(address,uint256)')
const T_DELEGATED = toEventSelector('Delegated(address,address,uint256)')
const T_QUEUED = toEventSelector('VaultActionQueued(uint256,address,uint256,bytes,uint64)')
const topicAddr = (a: string) => pad(a as `0x${string}`, { size: 32 }).toLowerCase()

export function demoTxCount(block: number): number {
  const r = rand(`count:${block}`)
  const x = r()
  // Mostly quiet, with bursts: realistic for an early testnet.
  return x < 0.28 ? 0 : x < 0.7 ? 1 + Math.floor(r() * 3) : x < 0.95 ? 3 + Math.floor(r() * 8) : 12 + Math.floor(r() * 30)
}

function kindOf(block: number, index: number, r: () => number): Kind {
  if (index === 0 && block % 97 === 0) return 'vault-queue'
  if (index === 0 && block % 89 === 0) return 'deploy'
  const x = r()
  return x < 0.36 ? 'native' : x < 0.66 ? 'token' : x < 0.76 ? 'wxds' : x < 0.84 ? 'delegate' : x < 0.92 ? 'approve' : 'xns'
}

interface Built {
  detail: TxDetail
}

function buildTx(block: number, index: number): Built {
  const r = rand(`tx:${block}:${index}`)
  const kind = kindOf(block, index, r)
  const ts = DEMO_GENESIS + block
  let from = crowd(r)
  let to: string | null = crowd(r)
  let value = 0n
  let input = '0x'
  let contractCreated: string | null = null
  const logs: DecodedLog[] = []
  const transfers: TokenTransfer[] = []
  const approvals: TxDetail['approvals'] = []
  let gasUsed = 21_000n
  const log = (address: string, topics: string[], data: string, event: string | null, signature: string | null, standard: string | null, args: Record<string, unknown> | null) =>
    logs.push({ logIndex: logs.length, address, topics, data, event, signature, standard, args })

  if (kind === 'native') {
    value = BigInt(Math.floor(r() * 250_000)) * 10n ** 14n + 10n ** 15n
    transfers.push({ blockNumber: block, timestamp: ts, txHash: '', logIndex: null, kind: 'native', token: { address: null, name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18, known: true }, from, to: to!, value: hx(value), spam: false, spamReasons: [], lookAlikeOf: null })
  } else if (kind === 'token' || kind === 'approve') {
    const t = TOKEN_LIST[Math.floor(r() * 2)]!
    const other = crowd(r)
    to = t.address
    const amt = BigInt(Math.floor(r() * 50_000) + 1) * 10n ** BigInt(t.decimals) / 10n
    if (kind === 'token') {
      input = encodeFunctionData({ abi: erc20Abi, functionName: 'transfer', args: [other as `0x${string}`, amt] })
      gasUsed = 51_384n
      log(t.address, [T_TRANSFER, topicAddr(from), topicAddr(other)], hx(amt), 'Transfer', 'Transfer(address,address,uint256)', 'erc20', { from, to: other, value: amt.toString() })
      transfers.push({ blockNumber: block, timestamp: ts, txHash: '', logIndex: 0, kind: 'erc20', token: { address: t.address, name: t.name, symbol: t.symbol, decimals: t.decimals, known: true }, from, to: other, value: hx(amt), spam: false, spamReasons: [], lookAlikeOf: null })
    } else {
      const expiry = BigInt(ts + 30 * 86400)
      input = encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [other as `0x${string}`, amt, expiry] })
      gasUsed = 48_120n
      log(t.address, [T_APPROVAL_EXP, topicAddr(from), topicAddr(other)], encodeAbiParameters([{ type: 'uint256' }, { type: 'uint64' }], [amt, expiry]), 'ApprovalWithExpiry', 'ApprovalWithExpiry(address,address,uint256,uint64)', 'xdrc20', { owner: from, spender: other, value: amt.toString(), expiry: expiry.toString() })
      approvals.push({ kind: 'xdrc20', event: 'ApprovalWithExpiry', token: { address: t.address, name: t.name, symbol: t.symbol, decimals: t.decimals, known: true }, owner: from, spender: other, amount: hx(amt), unlimited: false, expiry: Number(expiry), revokesAll: false })
    }
  } else if (kind === 'wxds') {
    to = DEMO_TOKENS.WXDS.address
    value = BigInt(Math.floor(r() * 900) + 10) * E18 / 10n
    input = encodeFunctionData({ abi: wxdsAbi, functionName: 'deposit' })
    gasUsed = 45_038n
    log(to, [T_DEPOSIT, topicAddr(from)], hx(value), 'Deposit', 'Deposit(address,uint256)', 'wxds', { dst: from, wad: value.toString() })
  } else if (kind === 'delegate') {
    const v = ACTIVE[Math.floor(r() * ACTIVE.length)]!
    to = predeploys.STAKING
    value = BigInt(Math.floor(r() * 20) + 1) * 500n * E18
    input = encodeFunctionData({ abi: stakingAbi, functionName: 'delegate', args: [v.operator as `0x${string}`] })
    gasUsed = 96_412n
    log(to, [T_DELEGATED, topicAddr(from), topicAddr(v.operator)], hx(value), 'Delegated', 'Delegated(address,address,uint256)', null, { delegator: from, validator: v.operator, amount: value.toString() })
  } else if (kind === 'xns') {
    to = predeploys.XNS_REGISTRY
    const label = ['nova', 'lumen', 'quill', 'orbit', 'sable', 'tidal', 'vesper', 'zephyr'][Math.floor(r() * 8)]! + Math.floor(r() * 90 + 10)
    value = 5n * E18
    input = encodeFunctionData({ abi: xnsAbi, functionName: 'register', args: [label, from as `0x${string}`, 31_536_000n] })
    gasUsed = 182_960n
  } else if (kind === 'vault-queue') {
    from = DEMO_PEOPLE.alice.address
    to = DEMO_PEOPLE.alice.address
    const target = DEMO_PEOPLE.dave.address
    value = 2_500n * E18
    input = encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [target, value, '0x'] })
    value = 0n
    gasUsed = 118_744n
    const id = BigInt(Math.floor(block / 97) % 40) + 1n
    const executeAfter = BigInt(ts + 86_400)
    log(DEMO_PEOPLE.alice.address, [T_QUEUED, pad(hx(id) as `0x${string}`, { size: 32 })], encodeAbiParameters([{ type: 'address' }, { type: 'uint256' }, { type: 'bytes' }, { type: 'uint64' }], [target, 2_500n * E18, '0x', executeAfter]), 'VaultActionQueued', 'VaultActionQueued(uint256,address,uint256,bytes,uint64)', 'vault', { id: id.toString(), target, value: (2_500n * E18).toString(), data: '0x', executeAfter: executeAfter.toString() })
  } else if (kind === 'deploy') {
    to = null
    contractCreated = addr(`contract:${block}`)
    input = `0x6080604052${hash(`code:${block}`).slice(2)}${hash(`code2:${block}`).slice(2)}`
    gasUsed = 1_284_331n
  }

  const baseFee = demoBaseFee(block)
  const tip = 1_000_000_000n
  const total = gasUsed * (baseFee + tip)
  const base = gasUsed * baseFee
  const txHash = demoTxHash(block, index)
  for (const t of transfers) t.txHash = txHash
  const detail: TxDetail = {
    hash: txHash,
    blockNumber: block,
    timestamp: ts,
    txIndex: index,
    type: kind === 'vault-queue' ? 4 : 2,
    from,
    to,
    contractCreated,
    value: hx(value),
    status: r() < 0.03 ? 'reverted' : 'success',
    methodId: input.length >= 10 ? input.slice(0, 10) : null,
    fee: hx(total),
    spam: false,
    spamReasons: [],
    blockHash: demoBlockHash(block),
    nonce: Math.floor(r() * 400),
    gas: hx((gasUsed * 13n) / 10n),
    gasUsed: hx(gasUsed),
    gasPrice: hx(baseFee + tip),
    maxFeePerGas: hx(baseFee * 2n + tip),
    maxPriorityFeePerGas: hx(tip),
    effectiveGasPrice: hx(baseFee + tip),
    input,
    authorizationList: kind === 'vault-queue' ? [{ chainId: hx(717171), address: predeploys.SMART_ACCOUNT_IMPL, nonce: '0x0', authority: from }] : null,
    feeBreakdown: { total: hx(total), baseFee: hx(base), burned: hx((base * 8000n) / 10000n), treasury: hx(base - (base * 8000n) / 10000n), priorityFee: hx(total - base), burnBps: 8000 },
    logs,
    tokenTransfers: transfers,
    approvals,
  }
  if (detail.status === 'reverted') {
    detail.logs = []
    detail.tokenTransfers = []
    detail.approvals = []
  }
  return { detail }
}

const summary = (d: TxDetail): TxSummary => ({
  hash: d.hash, blockNumber: d.blockNumber, timestamp: d.timestamp, txIndex: d.txIndex, type: d.type, from: d.from, to: d.to,
  contractCreated: d.contractCreated, value: d.value, status: d.status, methodId: d.methodId, fee: d.fee, spam: d.spam, spamReasons: d.spamReasons,
})

// --------------------------------------------------------------------------------------------------------------
// Blocks
// --------------------------------------------------------------------------------------------------------------

export const demoBlockHash = (n: number) => hash(`block:${n}`)
const demoBaseFee = (n: number) => 1_000_000_000n + BigInt(Math.floor(rand(`fee:${Math.floor(n / 30)}`)() * 400_000_000))
const proposerOf = (n: number) => ACTIVE[n % ACTIVE.length]!

export function demoBlock(n: number): BlockDetail {
  const count = demoTxCount(n)
  const txs = Array.from({ length: count }, (_, i) => buildTx(n, i).detail)
  const gasUsed = txs.reduce((s, t) => s + BigInt(t.gasUsed), 0n)
  const base = gasUsed * demoBaseFee(n)
  const priority = txs.reduce((s, t) => s + BigInt(t.feeBreakdown.priorityFee), 0n)
  const commit = demoCommit(n)
  return {
    number: n,
    hash: demoBlockHash(n),
    parentHash: demoBlockHash(n - 1),
    timestamp: DEMO_GENESIS + n,
    miner: proposerOf(n).operator,
    txCount: count,
    gasUsed: hx(gasUsed),
    gasLimit: hx(60_000_000),
    baseFeePerGas: hx(demoBaseFee(n)),
    size: 620 + count * 148 + Math.floor(rand(`size:${n}`)() * 90),
    fees: { burned: hx((base * 8000n) / 10000n), treasury: hx(base - (base * 8000n) / 10000n), priority: hx(priority) },
    transactions: txs.map(summary),
    commit: { round: commit.round, signatures: commit.signatures.length, validators: commit.signatures.map((s) => s.validator) },
  }
}

export function demoBlockSummary(n: number): BlockSummary {
  const { transactions: _t, commit: _c, ...b } = demoBlock(n)
  return b
}

export function demoCommit(n: number): CommitDetail {
  const r = rand(`commit:${n}`)
  // Usually everyone signs; now and then one validator's precommit arrives too late for the certificate.
  const missing = r() < 0.18 ? Math.floor(r() * ACTIVE.length) : -1
  return {
    height: n,
    round: r() < 0.04 ? 1 : 0,
    blockHash: demoBlockHash(n),
    signatures: ACTIVE.filter((_, i) => i !== missing).map((v) => ({ validator: consensusAddress(v.consensusKey), signature: `${hash(`sig:${n}:${v.operator}`)}${hash(`sig2:${n}:${v.operator}`).slice(2)}` })),
  }
}

export function demoTx(h: string): TxDetail | null {
  const at = decodeDemoTxHash(h.toLowerCase())
  if (!at || at.block > demoHead() || at.index >= demoTxCount(at.block)) return null
  return buildTx(at.block, at.index).detail
}

/** Latest transactions, newest first, scanning back from `head`. */
export function demoLatestTxs(head: number, limit = 10): TxSummary[] {
  const out: TxSummary[] = []
  for (let n = head; n > head - 400 && out.length < limit; n--) {
    const b = demoBlock(n)
    for (let i = b.transactions.length - 1; i >= 0 && out.length < limit; i--) out.push(b.transactions[i]!)
  }
  return out
}

/** A recent block that holds a Guardian Vault queue (for links and screenshots). */
export function demoVaultTxHash(head = demoHead()): string {
  let n = head - (head % 97)
  while (demoTxCount(n) === 0) n -= 97
  return demoTxHash(n, 0)
}

// --------------------------------------------------------------------------------------------------------------
// Addresses, tokens, names, validators
// --------------------------------------------------------------------------------------------------------------

const PREDEPLOY_LABELS: Record<string, string> = {
  [predeploys.STAKING.toLowerCase()]: 'Staking (predeploy)',
  [predeploys.GOVERNANCE.toLowerCase()]: 'Governance (predeploy)',
  [predeploys.TREASURY.toLowerCase()]: 'Treasury (predeploy)',
  [predeploys.FEE_CONFIG.toLowerCase()]: 'FeeConfig (predeploy)',
  [predeploys.XNS_REGISTRY.toLowerCase()]: 'XNS Registry (predeploy)',
  [predeploys.SMART_ACCOUNT_IMPL.toLowerCase()]: 'Smart Account implementation (predeploy)',
  [predeploys.SMART_ACCOUNT_FACTORY.toLowerCase()]: 'Smart Account factory (predeploy)',
  [predeploys.PAYMASTER.toLowerCase()]: 'Paymaster (predeploy)',
  [predeploys.PERMIT2_EXPIRING.toLowerCase()]: 'Permit2Expiring (predeploy)',
  [predeploys.WXDS.toLowerCase()]: 'Wrapped XDS (predeploy)',
}

export function demoAddress(a: string): AddressSummary {
  const address = getAddress(a)
  const low = address.toLowerCase()
  const r = rand(`addr:${low}`)
  const token = TOKEN_LIST.find((t) => t.address.toLowerCase() === low)
  const isAlice = low === DEMO_PEOPLE.alice.address.toLowerCase()
  const predeploy = PREDEPLOY_LABELS[low]
  const validator = DEMO_VALIDATORS.find((v) => v.operator.toLowerCase() === low)
  const name = DEMO_NAMES[low] ?? null
  const isContract = !!token || !!predeploy
  const head = demoHead()
  return {
    address,
    live: true,
    balance: hx(isAlice ? 18_420n * E18 + 3n * 10n ** 17n : predeploy === 'Treasury (predeploy)' ? 1_904_221n * E18 : BigInt(Math.floor(r() * 9_000_000)) * 10n ** 15n),
    nonce: isContract ? 1 : Math.floor(r() * 300),
    kind: isAlice ? 'delegated' : isContract ? 'contract' : 'eoa',
    delegatedTo: isAlice ? predeploys.SMART_ACCOUNT_IMPL : null,
    isContract,
    firstSeenBlock: predeploy || token ? 0 : Math.floor(r() * head * 0.6),
    lastSeenBlock: head - Math.floor(r() * 40),
    txCount: isAlice ? 214 : Math.floor(r() * 900) + 3,
    xns: { primaryName: name, names: name ? [{ name, expires: DEMO_GENESIS + 365 * 86400, status: 'active' }] : [] },
    labels: [...(predeploy ? [predeploy] : []), ...(validator ? ['Validator'] : []), ...(low === DEMO_PEOPLE.faucet.address.toLowerCase() ? ['Faucet'] : [])],
    token: token ? { address: token.address, name: token.name, symbol: token.symbol, decimals: token.decimals, known: true } : null,
    security: {
      accountKind: isAlice ? 'xdevs-7702' : null,
      vaultEnabled: isAlice ? true : null,
      frozen: false,
      frozenUntil: null,
      eoaLock: isAlice ? { status: 'locked', active: true, source: 'rpc' } : { status: 'unlocked', active: true, source: 'rpc' },
      eoaKeyRevoked: false,
      recovery: null,
      guardians: isAlice ? { count: 3, threshold: 2 } : null,
      openVaultActions: isAlice ? 1 : 0,
    },
  }
}

/** Transactions touching `a`: synthesized from recent blocks plus address-specific history. */
export function demoAddressTxs(a: string, limit = 25): TxSummary[] {
  const low = a.toLowerCase()
  const head = demoHead()
  const out: TxSummary[] = []
  for (let n = head; n > head - 600 && out.length < limit; n--) {
    for (const t of demoBlock(n).transactions) if (t.from.toLowerCase() === low || t.to?.toLowerCase() === low) out.push(t)
  }
  if (out.length < 6) {
    // Quiet address: give it a believable sparse history so the page is never empty in demo mode.
    const r = rand(`hist:${low}`)
    for (let i = 0; out.length < 8; i++) {
      const n = head - 600 - Math.floor(r() * 400_000) - i * 977
      const t = { ...buildTx(n, 0).detail }
      out.push(summary({ ...t, from: i % 3 === 0 ? a : t.from, to: i % 3 === 0 ? t.to : a }))
    }
  }
  return out.sort((x, y) => y.blockNumber - x.blockNumber)
}

export function demoVaultQueue(a: string): VaultAction[] {
  if (a.toLowerCase() !== DEMO_PEOPLE.alice.address.toLowerCase()) return []
  const head = demoHead()
  const n = head - (head % 97)
  const now = DEMO_GENESIS + head
  return [
    { account: a, id: '0x7', target: DEMO_PEOPLE.dave.address, value: hx(2_500n * E18), data: '0x', executeAfter: DEMO_GENESIS + n + 86_400, status: 'queued', queuedBlock: n, queuedTx: demoTxHash(n, 0), cancelledBy: null, finalBlock: null, finalTx: null },
    { account: a, id: '0x6', target: DEMO_PEOPLE.carol.address, value: hx(900n * E18), data: '0x', executeAfter: now - 4 * 86_400, status: 'executed', queuedBlock: n - 432_000, queuedTx: demoTxHash(n - 432_000 + 3, 0), cancelledBy: null, finalBlock: n - 340_000, finalTx: demoTxHash(n - 340_000, 0) },
    { account: a, id: '0x5', target: addr('attacker'), value: hx(18_000n * E18), data: '0x', executeAfter: now - 9 * 86_400, status: 'cancelled', queuedBlock: n - 900_000, queuedTx: demoTxHash(n - 900_000, 0), cancelledBy: DEMO_PEOPLE.alice.address, finalBlock: n - 899_200, finalTx: demoTxHash(n - 899_200, 0) },
  ]
}

export function demoApprovals(a: string): Approval[] {
  const r = rand(`appr:${a.toLowerCase()}`)
  const t = DEMO_TOKENS.eUSD
  const now = DEMO_GENESIS + demoHead()
  return [
    { kind: 'xdrc20', token: { address: t.address, name: t.name, symbol: t.symbol, decimals: t.decimals, known: true }, spender: addr('dex-router'), amount: hx(5_000n * 10n ** 6n), unlimited: false, expiry: now + 21 * 86_400, expired: false, remaining: hx(3_120n * 10n ** 6n), blockNumber: demoHead() - 90_000, txHash: demoTxHash(demoHead() - 90_000, 0), revoke: { to: t.address, data: '0x095ea7b3', description: 'approve(spender, 0) on the token' } },
    ...(r() < 0.7 ? [{ kind: 'permit2' as const, token: { address: DEMO_TOKENS.NMB.address, name: 'Nimbus', symbol: 'NMB', decimals: 18, known: true }, spender: addr('aggregator'), amount: hx(250n * E18), unlimited: false, expiry: now + 3 * 3600, expired: false, remaining: hx(250n * E18), blockNumber: demoHead() - 400, txHash: demoTxHash(demoHead() - 400, 0), revoke: { to: predeploys.PERMIT2_EXPIRING, data: '0x87517c45', description: 'approve(token, spender, 0, 0) on Permit2Expiring' } }] : []),
  ]
}

export function demoTokenPage(a: string): TokenPage | null {
  const t = TOKEN_LIST.find((x) => x.address.toLowerCase() === a.toLowerCase())
  if (!t) return null
  const head = demoHead()
  const transfers: TokenTransfer[] = []
  for (let n = head; n > head - 400 && transfers.length < 20; n--)
    for (const tx of demoBlock(n).transactions) {
      if (tx.to?.toLowerCase() !== t.address.toLowerCase()) continue
      const d = demoTx(tx.hash)
      for (const tr of d?.tokenTransfers ?? []) if (tr.kind === 'erc20') transfers.push(tr)
    }
  return {
    token: { address: t.address, name: t.name, symbol: t.symbol, decimals: t.decimals, totalSupply: hx(t.supply), xdrc20: t.xdrc20 ? { defaultApprovalDuration: 30 * 86_400, maxApprovalDuration: 365 * 86_400 } : null, known: true, label: t.symbol === 'WXDS' ? 'Wrapped XDS (predeploy)' : null },
    transfers,
    scannedBlocks: null,
    holdersNote: null,
  }
}

export function demoName(name: string): NameInfo | null {
  const owner = Object.entries(DEMO_NAMES).find(([, n]) => n === name)?.[0]
  if (!owner) return null
  return {
    name, label: name.replace(/\.xdevs$/, ''), node: hash(`node:${name}`), labelhash: hash(`label:${name}`), owner: getAddress(owner), address: getAddress(owner),
    expires: DEMO_GENESIS + 365 * 86_400, graceEndsAt: DEMO_GENESIS + 455 * 86_400, status: 'active', registeredBlock: 1200 + name.length * 31,
    registeredTx: demoTxHash(1200 + name.length * 31, 0), isPrimaryFor: getAddress(owner),
  }
}

export function demoValidatorSet(): ValidatorSet {
  return { validators: DEMO_VALIDATORS, epoch: Math.floor(demoHead() / 3600), via: 'demo' }
}

export const DEMO_STATS = (head: number) => ({ totalTransactions: 1_942_318 + Math.floor(head * 0.37), totalAddresses: 48_211 + Math.floor(head / 90), burnedFees: hx(BigInt(head) * 31n * 10n ** 13n), treasuryFees: hx(BigInt(head) * 775n * 10n ** 11n) })
