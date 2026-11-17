// Demo backend (NEXT_PUBLIC_DEMO=1, or "Preview with demo data"): an in-memory account with fixture data so every
// screen can be reviewed without a chain, a bundler or a real passkey. Every screen shows a "Demo" label. Vault
// decisions use the same local rules as the live wallet (`explainVault`), so the demo behaves like the real thing.
import { type Address, decodeFunctionData, encodeFunctionData, formatUnits, getAddress, isAddressEqual, keccak256, maxUint256, parseEther, parseUnits, toFunctionSelector, toHex } from 'viem'
import { checkXnsLabel, findLookAlikeAddresses, normalizeXnsName } from '@xdevs/sdk'
import { config } from '../config.ts'
import { accountAbi, erc20Abi, permit2Abi, xdrc20Abi, xnsAbi } from '../lib/abi.ts'
import { type AccountState, type Call, PERMIT2_EXPIRING, UNLIMITED_APPROVAL, recommendedPolicy } from '../lib/account.ts'
import type { ApprovalItem } from '../lib/approvals.ts'
import { ADVICE_DISCLAIMER, type Preview } from '../lib/preview.ts'
import type { RecipientCheckResult, ResolvedRecipient } from '../lib/recipient.ts'
import { passkeySignerStruct } from '../lib/signers.ts'
import { type QueuedAction, explainVault } from '../lib/vault.ts'
import type { AccountRecord } from '../store.ts'
import type { ActivityItem, Asset, Backend, OpResult, RecoveryStatus, Review, XnsQuote } from './types.ts'

const A = (s: string) => getAddress(s.toLowerCase())

export const DEMO_ACCOUNT = A('0x7a3F9c21D4e5B6a7C8d9E0f1A2b3C4d5E6f70D1e')
export const DEMO_NAME = 'aria.xdevs'
/** An EOA used by the demo "connect MetaMask" flow on the EOA-lock screen. */
export const DEMO_EOA = A('0x3b8e1F0aC94d27E6b5A01c9D2e7F8a6B4c3D2e1F')

const MAYA = A('0x5a1E7c0B9d24F6a83E10c4B7d9A2e6F1b3C8a7D4')
const REN = A('0x9F3c2B1a0E8d7C6b5A4f3E2d1C0b9A8f7E6d5C4B')
const FAUCET = A('0xFa0cE7000000000000000000000000000000fa0c')
/** Address-poisoning fixture: same first and last 4 hex chars as Maya's address. */
export const POISON = A('0x5a1E00000000000000000000000000000000a7D4')
const ROUTER = A('0x2Ee5b6E3c7D4A1F0e9B8c7D6a5F4e3D2c1B0a9F8')
const MARKET = A('0x6C1f0E9d8C7b6A5f4E3d2C1b0A9f8E7d6C5b4A31')
const USDX = A('0x0D5aE1b2C3d4E5f6A7b8C9d0E1f2A3b4C5d6E7F8')
const ENCL = A('0xE4c1A0B9c8D7e6F5a4B3c2D1e0F9a8B7c6D5e4F3')
const WXDS = A('0x0000000000000000000000000000000000001009')
const G1 = A('0x1C0a6E5d4B3c2A1f0E9d8C7b6A5f4E3d2C1b0A91')
const G2 = A('0x8B7a6F5e4D3c2B1a0F9e8D7c6B5a4F3e2D1c0B92')
const G3 = A('0x4D3c2B1a0F9e8D7c6B5a4F3e2D1c0B9a8F7e6D93')

/** Names known to the demo "registry" (forward + reverse). */
const NAMES: Record<string, Address> = {
  'maya.xdevs': MAYA,
  'ren.xdevs': REN,
  'faucet.xdevs': FAUCET,
  'theo.xdevs': G1,
  'lena.xdevs': G2,
  'tidalswap.xdevs': ROUTER,
  [DEMO_NAME]: DEMO_ACCOUNT,
}
const nameOf = (a: Address) => Object.entries(NAMES).find(([, v]) => isAddressEqual(v, a))?.[0] ?? null
export const demoLabel = (a: Address): string | null => {
  if (isAddressEqual(a, ROUTER)) return 'TidalSwap router'
  if (isAddressEqual(a, MARKET)) return 'Lumen Market'
  if (isAddressEqual(a, G3)) return 'Your Ledger'
  if (isAddressEqual(a, FAUCET)) return 'Enclave faucet'
  return nameOf(a)
}
const TOKENS: Record<string, { symbol: string; name: string; decimals: number }> = {
  [USDX]: { symbol: 'USDx', name: 'Bridged USD (test)', decimals: 6 },
  [ENCL]: { symbol: 'ENCL', name: 'Enclave Points', decimals: 18 },
  [WXDS]: { symbol: 'WXDS', name: 'Wrapped tXDS', decimals: 18 },
}

const now = () => Math.floor(Date.now() / 1000)
const EXECUTE_ID_SELECTOR = toFunctionSelector('execute(uint256)')
let speed = 1
/** Test hook: 0 = no simulated latency. */
export function setDemoSpeed(f: number): void {
  speed = f
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms * speed))
const H = 3_600

type DemoDb = {
  state: AccountState
  queue: QueuedAction[]
  approvals: ApprovalItem[]
  activity: ActivityItem[]
  assets: Asset[]
  primaryName: string | null
  history: Address[]
  eoaLocked: boolean
}

function initialDb(): DemoDb {
  const t = now()
  const policy = recommendedPolicy({ nativeThreshold: parseEther('100'), dailyLimit: parseEther('250'), delay: 24n * 3_600n })
  const state: AccountState = {
    address: DEMO_ACCOUNT,
    deployed: true,
    is7702: false,
    balance: parseEther('1284.5'),
    initialized: true,
    policy,
    whitelist: [MAYA],
    spentToday: parseEther('40'),
    guardians: [G1, G2, G3],
    guardianThreshold: 2,
    recoveryDelay: 48n * 3_600n,
    frozen: false,
    frozenUntil: 0n,
    freezeDuration: 30n * 86_400n,
    nextFreezeAllowedAt: 0n,
    lastActionId: 5n,
    signers: [passkeySignerStruct({ x: keccak256(toHex('demo-x')), y: keccak256(toHex('demo-y')) })],
    eoaKeyActive: true,
    eoaLock: null,
  }
  const send = (to: Address, v: string): Call => ({ target: to, value: parseEther(v), data: '0x' })
  const queue: QueuedAction[] = [
    { id: 5n, executeAfter: BigInt(t + 17 * H + 12 * 60), status: 'queued', calls: [send(REN, '600')], cancelVotes: 0, totalValue: parseEther('600') },
    {
      id: 4n,
      executeAfter: BigInt(t + 3 * H + 40 * 60),
      status: 'queued',
      calls: [{ target: USDX, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [MARKET, parseUnits('500', 6)] }) }],
      cancelVotes: 1,
      totalValue: 0n,
    },
    { id: 3n, executeAfter: BigInt(t - 30 * H), status: 'cancelled', calls: [send(POISON, '2000')], cancelVotes: 2, totalValue: parseEther('2000') },
    {
      id: 2n,
      executeAfter: BigInt(t - 4 * 86_400),
      status: 'executed',
      calls: [{ target: DEMO_ACCOUNT, value: 0n, data: encodeFunctionData({ abi: accountAbi, functionName: 'setWhitelisted', args: [MAYA, true] }) }],
      cancelVotes: 0,
      totalValue: 0n,
    },
  ]
  const approvals: ApprovalItem[] = [
    { kind: 'erc20', token: USDX, symbol: 'USDx', spender: ROUTER, amount: maxUint256, unlimited: true, expiry: null, source: 'api' },
    { kind: 'xdrc20', token: ENCL, symbol: 'ENCL', spender: MARKET, amount: parseEther('250'), unlimited: false, expiry: t + 12 * 86_400, source: 'api' },
    { kind: 'permit2', token: USDX, symbol: 'USDx', spender: MARKET, amount: parseUnits('50', 6), unlimited: false, expiry: t + 3 * 86_400 + 5 * H, source: 'api' },
  ]
  const activity: ActivityItem[] = [
    { id: 'a1', kind: 'queued', title: 'Send tXDS (waiting in vault)', counterparty: REN, counterpartyName: 'ren.xdevs', amount: parseEther('600'), symbol: 'tXDS', timestamp: t - 6 * H - 48 * 60, status: 'queued' },
    { id: 'a2', kind: 'sent', title: 'Sent tXDS', counterparty: MAYA, counterpartyName: 'maya.xdevs', amount: parseEther('40'), symbol: 'tXDS', timestamp: t - 9 * H, status: 'confirmed', hash: keccak256(toHex('a2')) },
    { id: 'a3', kind: 'received', title: 'Received USDx', counterparty: REN, counterpartyName: 'ren.xdevs', amount: parseUnits('3250', 6), symbol: 'USDx', decimals: 6, timestamp: t - 26 * H, status: 'confirmed', hash: keccak256(toHex('a3')) },
    { id: 'a4', kind: 'cancelled', title: 'Transfer cancelled by guardians', counterparty: POISON, amount: parseEther('2000'), symbol: 'tXDS', timestamp: t - 30 * H, status: 'cancelled', lookAlikeOf: MAYA },
    { id: 'a5', kind: 'received', title: 'Received tXDS', counterparty: FAUCET, counterpartyName: 'Enclave faucet', amount: parseEther('1000'), symbol: 'tXDS', timestamp: t - 3 * 86_400, status: 'confirmed', hash: keccak256(toHex('a5')) },
    { id: 'a6', kind: 'received', title: 'Received tXDS', counterparty: POISON, amount: 0n, symbol: 'tXDS', timestamp: t - 3 * 86_400 + 600, status: 'confirmed', spam: true, lookAlikeOf: MAYA, hash: keccak256(toHex('a6')) },
    { id: 'a7', kind: 'account', title: 'Account activated (gas sponsored)', timestamp: t - 5 * 86_400, status: 'confirmed', hash: keccak256(toHex('a7')) },
  ]
  const assets: Asset[] = [
    { token: null, symbol: 'tXDS', name: config.coinName, decimals: 18, balance: state.balance },
    { token: USDX, ...TOKENS[USDX], balance: parseUnits('3250', 6) },
    { token: ENCL, ...TOKENS[ENCL], balance: parseEther('1820') },
    { token: WXDS, ...TOKENS[WXDS], balance: parseEther('12') },
  ]
  return { state, queue, approvals, activity, assets, primaryName: DEMO_NAME, history: [MAYA, REN], eoaLocked: false }
}

let db: DemoDb | null = null
const getDb = () => (db ??= initialDb())
/** Test hook. */
export function resetDemo(): void {
  db = null
}

export function demoTokens(): Address[] {
  return [USDX, ENCL]
}

/** Pure preview of one call from fixture metadata (the live wallet uses eth_call + decoding, or the node). */
export function demoPreview(from: Address, call: Call, check: RecipientCheckResult | null, balance: bigint): Preview {
  const warnings: Preview['warnings'] = []
  const summary: string[] = []
  const assetChanges: Preview['assetChanges'] = []
  const approvals: Preview['approvals'] = []
  const label = (a: Address) => demoLabel(a) ?? a
  if (call.value > 0n) {
    summary.push(`Send ${formatUnits(call.value, 18)} tXDS to ${label(call.target)}`)
    assetChanges.push({ token: 'native', symbol: 'tXDS', from, to: call.target, amount: formatUnits(call.value, 18) })
    if (balance > 0n && call.value * 2n > balance) warnings.push({ code: 'HIGH_VALUE_TRANSFER', severity: 'medium', message: 'This sends more than half of your tXDS balance.' })
  }
  if (call.data.length >= 10) {
    try {
      const d = decodeFunctionData({ abi: [...erc20Abi, ...xdrc20Abi, ...permit2Abi, ...xnsAbi], data: call.data })
      const args = (d.args ?? []) as readonly unknown[]
      const meta = TOKENS[call.target] ?? { symbol: 'token', decimals: 18 }
      if (d.functionName === 'transfer') {
        const [to, amt] = args as [Address, bigint]
        summary.push(`Send ${formatUnits(amt, meta.decimals)} ${meta.symbol} to ${label(to)}`)
        assetChanges.push({ token: call.target, symbol: meta.symbol, from, to, amount: formatUnits(amt, meta.decimals) })
      } else if (d.functionName === 'approve' && args.length === 2) {
        const [spender, amt] = args as [Address, bigint]
        const unlimited = amt >= UNLIMITED_APPROVAL
        summary.push(amt === 0n ? `Revoke ${label(spender)}'s ${meta.symbol} allowance` : `Allow ${label(spender)} to spend ${unlimited ? 'ALL your' : formatUnits(amt, meta.decimals)} ${meta.symbol}`)
        approvals.push({ token: call.target, spender, amount: unlimited ? 'unlimited' : formatUnits(amt, meta.decimals), expires: null })
        if (unlimited) warnings.push({ code: 'UNLIMITED_APPROVAL', severity: 'high', message: `This gives ${label(spender)} permission to spend ALL your ${meta.symbol}, forever, until you revoke it.` })
      } else if (d.functionName === 'register') {
        summary.push(`Register ${String(args[0])}.xdevs for 1 year`)
      } else {
        summary.push(`Call ${d.functionName} on ${label(call.target)}`)
      }
    } catch {
      summary.push(`Call ${call.data.slice(0, 10)} on ${label(call.target)} (not decoded)`)
      warnings.push({ code: 'UNVERIFIED_CONTRACT', severity: 'low', message: 'The wallet cannot decode this call. Only continue if you trust the app that built it.' })
    }
  }
  if (check?.lookAlikeOf.length) {
    warnings.push({
      code: 'LOOKALIKE_RECIPIENT',
      severity: 'high',
      message: `This address looks like one you used before (${check.lookAlikeOf.map((a) => demoLabel(a) ?? a).join(', ')}) but is different. This is how address poisoning works.`,
    })
  }
  if (check?.firstTime) warnings.push({ code: 'FIRST_TIME_RECIPIENT', severity: 'low', message: 'You have never sent to this address before.' })
  return { success: true, gasUsed: 52_000n, assetChanges, approvals, warnings, disclaimer: ADVICE_DISCLAIMER, revertReason: null, source: 'local', summary }
}

export const demoBackend: Backend = {
  demo: true,
  async chainStatus() {
    return { kind: 'ok', chainId: config.chainId, block: 1_204_331n + BigInt(Math.floor((Date.now() / 1000) % 100_000)) }
  },
  async readAccount(address) {
    const d = getDb()
    if (isAddressEqual(address, DEMO_ACCOUNT)) return { ...d.state }
    if (isAddressEqual(address, DEMO_EOA))
      return {
        ...d.state,
        address,
        is7702: true,
        balance: parseEther('42.25'),
        guardians: [G1, G2],
        guardianThreshold: 2,
        whitelist: [],
        lastActionId: 0n,
        eoaLock: { status: d.eoaLocked ? 'locked' : 'unlocked', word: d.eoaLocked ? 1n : 0n, protocolActive: true, delegatedTo: A('0x0000000000000000000000000000000000001005'), source: 'node' },
      }
    // Any other address: a plain, un-upgraded EOA.
    return { ...d.state, address, deployed: false, is7702: false, balance: parseEther('42.25'), initialized: false, guardians: [], guardianThreshold: 0, whitelist: [], policy: { ...d.state.policy, enabled: false }, signers: [], lastActionId: 0n }
  },
  async assets() {
    const d = getDb()
    d.assets[0] = { ...d.assets[0], balance: d.state.balance }
    return d.assets.map((a) => ({ ...a }))
  },
  async activity() {
    return [...getDb().activity].sort((a, b) => b.timestamp - a.timestamp)
  },
  async vaultQueue() {
    const t = BigInt(now())
    return getDb().queue.map((q) => (q.status === 'queued' && q.executeAfter <= t ? { ...q, status: 'ready' as const } : { ...q }))
  },
  async approvals() {
    return getDb().approvals.map((a) => ({ ...a }))
  },
  async recoveryStatus(): Promise<RecoveryStatus> {
    return { kind: 'none', text: 'No recovery in progress.' }
  },
  async resolveRecipient(input): Promise<ResolvedRecipient> {
    const s = input.trim()
    if (/^0x/i.test(s)) {
      if (!/^0x[0-9a-fA-F]{40}$/.test(s)) throw new Error('This is not a valid address (0x followed by 40 hex characters).')
      return { address: getAddress(s), name: null, via: 'address' }
    }
    const name = normalizeXnsName(s)
    const a = NAMES[name]
    if (!a) throw new Error(`${name} is not registered (or has expired).`)
    return { address: a, name, via: 'registry' }
  },
  async checkRecipient(_from, to, history): Promise<RecipientCheckResult> {
    const known = [...getDb().history, ...history]
    return {
      firstTime: !known.some((h) => isAddressEqual(h, to)),
      lookAlikeOf: [...new Set(findLookAlikeAddresses(to, known))],
      name: nameOf(to),
      isContract: [ROUTER, MARKET, USDX, ENCL, WXDS].some((c) => isAddressEqual(c, to)),
      source: 'local',
    }
  },
  async review(state, call, recipient, check): Promise<Review> {
    await sleep(450)
    return { call, recipient, check, preview: demoPreview(state.address, call, check, state.balance), vault: explainVault(state, [call]) }
  },
  async tokenDecimals(token) {
    return TOKENS[getAddress(token)]?.decimals ?? 18
  },
  async counterfactualAddress() {
    return DEMO_ACCOUNT
  },
  async primaryName(address) {
    return isAddressEqual(address, DEMO_ACCOUNT) ? getDb().primaryName : nameOf(address)
  },
  async xnsQuote(label): Promise<XnsQuote> {
    await sleep(250)
    const name = `${label}.xdevs`
    const err = checkXnsLabel(label)
    if (err) return { name, available: false, price: 0n, reason: err }
    const taken = Object.keys(NAMES).some((n) => n === name)
    const confusable = ['maya', 'ren', 'faucet', 'theo', 'lena', 'aria', 'tidalswap'].some((l) => l !== label && skeleton(l) === skeleton(label))
    const reserved = ['xdevs', 'the-xdevs', 'official', 'support', 'admin', 'faucet'].includes(label)
    if (taken || reserved) return { name, available: false, price: 0n, reason: reserved ? 'reserved' : 'already registered' }
    if (confusable) return { name, available: false, price: 0n, reason: 'too similar to an existing name (look-alike protection)' }
    return { name, available: true, price: 0n }
  },
  async sendOp(rec: AccountRecord, input, onStatus): Promise<OpResult> {
    const d = getDb()
    if (d.state.frozen) throw new Error('The account is frozen by its guardians.')
    const calls: Call[] = input.calls ?? [{ target: rec.address, value: 0n, data: input.callData ?? '0x' }]
    onStatus('Preparing…')
    await sleep(350)
    onStatus('Confirm with your passkey…')
    await sleep(1_100)
    onStatus('Submitting to the bundler…')
    await sleep(500)
    // execute(uint256) of a ready action is called directly (anyone may), never through the vault dispatcher.
    const executesReady = !!input.callData && input.callData.startsWith(EXECUTE_ID_SELECTOR)
    const outcome = executesReady ? ({ kind: 'instant', reason: 'executes a ready vault action' } as const) : explainVault(d.state, calls)
    if (outcome.kind === 'rejected') throw new Error(`Rejected by your Guardian Vault: ${outcome.reason}.`)
    onStatus('Waiting for inclusion…')
    await sleep(700)
    const t = now()
    const hash = keccak256(toHex(`demo-${t}-${Math.random()}`))
    const queued = outcome.kind === 'queued'
    if (queued) {
      const id = d.state.lastActionId + 1n
      d.state.lastActionId = id
      d.queue.unshift({ id, executeAfter: BigInt(t) + d.state.policy.delay, status: 'queued', calls, cancelVotes: 0, totalValue: calls.reduce((s, c) => s + c.value, 0n) })
      d.activity.unshift({ ...input.describe, id: hash, kind: 'queued', timestamp: t, status: 'queued' })
      return { userOpHash: hash, txHash: hash, queuedIds: [id] }
    }
    for (const c of calls) applyInstant(d, rec.address, c)
    d.activity.unshift({ ...input.describe, id: hash, timestamp: t, status: 'confirmed', hash })
    return { userOpHash: hash, txHash: hash, queuedIds: [] }
  },
}

function skeleton(label: string): string {
  return label.replace(/0/g, 'o').replace(/1/g, 'l').replace(/rn/g, 'm').replace(/vv/g, 'w').replace(/cl/g, 'd')
}

function applyInstant(d: DemoDb, self: Address, c: Call) {
  if (c.value > 0n) {
    d.state.balance -= c.value
    d.state.spentToday += c.value
    if (!d.history.some((h) => isAddressEqual(h, c.target))) d.history.push(c.target)
  }
  if (c.data.length < 10) return
  if (isAddressEqual(c.target, self)) {
    try {
      const f = decodeFunctionData({ abi: accountAbi, data: c.data })
      if (f.functionName === 'cancel') {
        const id = (f.args as readonly [bigint])[0]
        d.queue = d.queue.map((q) => (q.id === id ? { ...q, status: 'cancelled' } : q))
      } else if (f.functionName === 'execute' && (f.args as readonly unknown[]).length === 1) {
        const id = (f.args as readonly [bigint])[0]
        const q = d.queue.find((x) => x.id === id)
        if (q) {
          for (const qc of q.calls) applyInstant(d, self, qc)
          d.queue = d.queue.map((x) => (x.id === id ? { ...x, status: 'executed' } : x))
        }
      }
    } catch {
      /* other self-calls are queued, never instant */
    }
    return
  }
  try {
    const f = decodeFunctionData({ abi: [...erc20Abi, ...permit2Abi], data: c.data })
    const args = (f.args ?? []) as readonly unknown[]
    if (f.functionName === 'approve') {
      const permit = isAddressEqual(c.target, PERMIT2_EXPIRING)
      const token = (permit ? args[0] : c.target) as Address
      const spender = (permit ? args[1] : args[0]) as Address
      const amt = (permit ? args[2] : args[1]) as bigint
      if (amt === 0n) d.approvals = d.approvals.filter((a) => !(isAddressEqual(a.token, token) && isAddressEqual(a.spender, spender) && (a.kind === 'permit2') === permit))
    } else if (f.functionName === 'transfer') {
      const amt = args[1] as bigint
      d.assets = d.assets.map((a) => (a.token && isAddressEqual(a.token, c.target) ? { ...a, balance: a.balance - amt } : a))
    }
  } catch {
    /* not a token call */
  }
}

/** Token metadata of the demo fixtures. */
export function demoTokenMeta(a: Address): { symbol: string; decimals: number } | null {
  return TOKENS[getAddress(a)] ?? null
}

/** Demo EOA-lock flow: "enable" the XIP-0001 lock of the demo MetaMask address. */
export async function demoEnableEoaLock(): Promise<void> {
  await sleep(1_400)
  getDb().eoaLocked = true
}
