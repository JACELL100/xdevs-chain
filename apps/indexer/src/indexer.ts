// The block follower: fetches blocks + receipts over JSON-RPC, verifies the hash chain, writes Postgres in one
// transaction per batch (data + cursor together, so a crash never leaves half a batch), and derives explorer state.

import { formatEther, type Hex } from 'viem'
import { predeploys, xnsNamehash } from '@xdevs/sdk'
import type { Config } from './config.ts'
import { insertMany, type Db, type Queryable } from './db.ts'
import { decodeLog, toJsonSafe, type DecodedEvent } from './decode.ts'
import { isMethodNotFound, RpcError, sleep, unwrapAll, followNewHeads, type Rpc } from './rpc.ts'
import {
  accountKinds,
  codeKind,
  fetchTokenMeta,
  hexToBig,
  hexToNum,
  lower,
  toHex,
  ZERO_ADDRESS,
  type AccountKind,
  type RpcBlock,
  type RpcLog,
  type RpcReceipt,
  type RpcTx,
  type TokenMeta,
} from './chain.ts'
import { classifyTransfer, findLookAlike, fingerprint, isPoisonCandidate, NATIVE_SYMBOLS, type TokenInfo } from './spam.ts'

export class FatalIndexerError extends Error {}
class RetryableError extends Error {}

export interface FetchedBlock {
  block: RpcBlock
  receipts: RpcReceipt[]
}

const CURSOR_ID = 'main'
export const pgArray = (xs: readonly string[]): string => `{${xs.map((x) => `"${x}"`).join(',')}}`

/** Events whose emitter is a smart account (vault queue, recovery, freeze, EOA lock). */
const SECURITY_STANDARDS = new Set(['vault', 'account'])

const ALERT_TYPES: Record<string, string> = {
  VaultActionQueued: 'vault.queued',
  RecoveryApproved: 'recovery.approved',
  RecoveryReady: 'recovery.ready',
  RecoveryExecuted: 'recovery.executed',
  AccountFrozen: 'account.frozen',
  AccountUnfrozen: 'account.unfrozen',
  EoaLockUpdated: 'eoaLock.changed',
}

export const EOA_LOCK_STATES = ['unlocked', 'locked', 'sealed'] as const

interface EventCtx {
  blockNumber: number
  blockHash: string
  timestamp: number
  logIndex: number
  txHash: string
  emitter: string
}

type Logger = (msg: string) => void

export class Indexer {
  readonly db: Db
  readonly rpc: Rpc
  readonly cfg: Config
  private readonly log: Logger
  private chainId = 0
  private burnBps = 10_000
  private wake: (() => void) | null = null
  private receiptsByBlock = true
  /** Wall clock used for alert freshness; injectable for tests. */
  now: () => number = () => Math.floor(Date.now() / 1000)
  /** Called after each committed batch (the alert dispatcher hooks in here). */
  onCommit: (() => void) | null = null

  constructor(db: Db, rpc: Rpc, cfg: Config, log: Logger = (m) => console.log(`[indexer] ${m}`)) {
    this.db = db
    this.rpc = rpc
    this.cfg = cfg
    this.log = log
  }

  async init(): Promise<void> {
    this.chainId = hexToNum(await this.rpc.request<Hex>('eth_chainId'))
    const stored = (await this.db.query<{ chain_id: number; burn_bps: number }>('SELECT chain_id, burn_bps FROM chain_info WHERE id = 1')).rows[0]
    if (stored && Number(stored.chain_id) !== this.chainId)
      throw new FatalIndexerError(`database was indexed for chain ${stored.chain_id}, RPC is chain ${this.chainId}`)
    this.burnBps = await this.detectBurnBps()
    await this.db.query(
      `INSERT INTO chain_info (id, chain_id, burn_bps) VALUES (1, $1, $2)
       ON CONFLICT (id) DO UPDATE SET burn_bps = EXCLUDED.burn_bps`,
      [this.chainId, this.burnBps],
    )
  }

  /** `BURN_BPS` env, else 8000 on an xDEVS node (chain config, plan §15.1) and 10000 (all burned) elsewhere. */
  private async detectBurnBps(): Promise<number> {
    const env = process.env.BURN_BPS
    if (env !== undefined && env !== '') {
      const n = Number(env)
      if (!Number.isInteger(n) || n < 0 || n > 10_000) throw new FatalIndexerError('BURN_BPS must be 0..10000')
      return n
    }
    try {
      const v = await this.rpc.request<string>('web3_clientVersion')
      return /xdevs/i.test(v) ? 8000 : 10_000
    } catch {
      return 10_000
    }
  }

  async cursor(q: Queryable = this.db): Promise<{ number: number; hash: string } | null> {
    const r = (await q.query<{ block_number: number; block_hash: string }>('SELECT block_number, block_hash FROM indexer_cursor WHERE id = $1', [CURSOR_ID])).rows[0]
    return r ? { number: Number(r.block_number), hash: r.block_hash } : null
  }

  /** Follows the chain until `signal` aborts. Uses newHeads over WebSocket when available, polling otherwise. */
  async run(signal: AbortSignal): Promise<void> {
    const sub = this.cfg.wsUrl ? followNewHeads(this.cfg.wsUrl, () => this.wake?.(), this.log) : null
    signal.addEventListener('abort', () => this.wake?.(), { once: true })
    let backoff = 1000
    try {
      while (!signal.aborted) {
        try {
          const n = await this.step()
          backoff = 1000
          if (n > 0) continue
        } catch (err) {
          if (err instanceof FatalIndexerError) throw err
          this.log(`step failed (retrying in ${backoff} ms): ${(err as Error).message}`)
          await sleep(backoff)
          backoff = Math.min(backoff * 2, 30_000)
          continue
        }
        await new Promise<void>((resolve) => {
          const t = setTimeout(resolve, this.cfg.pollIntervalMs)
          this.wake = () => {
            clearTimeout(t)
            resolve()
          }
        })
        this.wake = null
      }
    } finally {
      sub?.close()
    }
  }

  /** Indexes one batch. Returns the number of blocks written (0 when caught up or after a rollback). */
  async step(): Promise<number> {
    if (!this.chainId) await this.init()
    const cur = await this.cursor()
    const head = hexToNum(await this.rpc.request<Hex>('eth_blockNumber')) - this.cfg.confirmations
    const from = cur ? cur.number + 1 : this.cfg.startBlock
    if (from > head) return 0
    const to = Math.min(head, from + this.cfg.batchSize - 1)
    const fetched = await this.fetchRange(from, to)

    if (cur && fetched[0]!.block.parentHash.toLowerCase() !== cur.hash) {
      await this.handleDivergence(cur)
      return 0
    }
    for (let i = 1; i < fetched.length; i++)
      if (fetched[i]!.block.parentHash.toLowerCase() !== fetched[i - 1]!.block.hash.toLowerCase())
        throw new RetryableError(`hash chain broken inside batch at block ${from + i}`)

    const prep = await this.prepare(fetched, toHex(to))
    await this.db.transaction((q) => this.write(q, fetched, prep))
    this.log(`indexed ${from}..${to} (${fetched.reduce((s, f) => s + f.block.transactions.length, 0)} txs)`)
    this.onCommit?.()
    return fetched.length
  }

  async fetchRange(from: number, to: number): Promise<FetchedBlock[]> {
    const nums = Array.from({ length: to - from + 1 }, (_, i) => from + i)
    const blocks = unwrapAll<RpcBlock | null>(
      await this.rpc.batch(nums.map((n) => ({ method: 'eth_getBlockByNumber', params: [toHex(n), true] }))),
    )
    blocks.forEach((b, i) => {
      if (!b) throw new RetryableError(`block ${nums[i]} not available yet`)
      if (hexToNum(b.number) !== nums[i]) throw new RetryableError(`node returned block ${b.number} for ${nums[i]}`)
    })
    const out: FetchedBlock[] = []
    let receiptLists: (RpcReceipt[] | null)[] = []
    if (this.receiptsByBlock) {
      const res = await this.rpc.batch(nums.map((n) => ({ method: 'eth_getBlockReceipts', params: [toHex(n)] })))
      if (res.some((r) => isMethodNotFound(r))) this.receiptsByBlock = false
      else receiptLists = unwrapAll<RpcReceipt[] | null>(res)
    }
    for (let i = 0; i < blocks.length; i++) {
      const block = blocks[i]!
      let receipts = receiptLists[i] ?? null
      if (!this.receiptsByBlock) {
        receipts = unwrapAll<RpcReceipt>(
          await this.rpc.batch(block.transactions.map((t) => ({ method: 'eth_getTransactionReceipt', params: [t.hash] }))),
        )
      }
      if (!receipts) throw new RetryableError(`receipts of block ${nums[i]} not available`)
      if (receipts.length !== block.transactions.length)
        throw new RetryableError(`block ${nums[i]}: ${receipts.length} receipts for ${block.transactions.length} txs`)
      receipts.forEach((r, j) => {
        if (!r || r.blockHash.toLowerCase() !== block.hash.toLowerCase() || r.transactionHash !== block.transactions[j]!.hash)
          throw new RetryableError(`block ${nums[i]}: receipt ${j} does not match the block (head moved?)`)
      })
      out.push({ block, receipts })
    }
    if (this.cfg.verifyLogs) {
      const logs = await this.rpc.request<RpcLog[]>('eth_getLogs', [{ fromBlock: toHex(from), toBlock: toHex(to) }])
      const expected = out.reduce((s, f) => s + f.receipts.reduce((t, r) => t + r.logs.length, 0), 0)
      if (logs.length !== expected) throw new RetryableError(`eth_getLogs returned ${logs.length} logs for ${from}..${to}, receipts have ${expected}`)
      const hashes = new Map(out.map((f) => [hexToNum(f.block.number), f.block.hash.toLowerCase()]))
      for (const l of logs)
        if (hashes.get(hexToNum(l.blockNumber)) !== l.blockHash.toLowerCase())
          throw new RetryableError(`eth_getLogs block hash mismatch at ${hexToNum(l.blockNumber)}`)
    }
    return out
  }

  /** RPC work done before the DB transaction: code of new addresses, token metadata, smart-account detection. */
  private async prepare(fetched: FetchedBlock[], at: Hex) {
    const addrs = new Set<string>()
    const tokenAddrs = new Set<string>()
    const securityEmitters = new Set<string>()
    const created = new Set<string>()
    for (const { block, receipts } of fetched) {
      block.transactions.forEach((tx, i) => {
        addrs.add(tx.from.toLowerCase())
        if (tx.to) addrs.add(tx.to.toLowerCase())
        const c = lower(receipts[i]!.contractAddress)
        if (c) {
          addrs.add(c)
          created.add(c)
        }
        for (const l of receipts[i]!.logs) {
          const ev = decodeLog(l)
          if (!ev) continue
          const em = l.address.toLowerCase()
          if (ev.standard === 'erc20' || ev.standard === 'xdrc20' || ev.standard === 'wxds') tokenAddrs.add(em)
          if (ev.standard === 'erc20' && ev.event === 'Transfer') {
            addrs.add(String(ev.args.from).toLowerCase())
            addrs.add(String(ev.args.to).toLowerCase())
          }
          if (ev.standard === 'permit2' && em === this.cfg.permit2.toLowerCase() && ev.args.token) tokenAddrs.add(String(ev.args.token).toLowerCase())
          if (SECURITY_STANDARDS.has(ev.standard)) securityEmitters.add(em)
        }
      })
    }
    addrs.delete(ZERO_ADDRESS)

    const known = async (table: string, col: string, list: Set<string>) =>
      new Set(list.size ? (await this.db.query<{ a: string }>(`SELECT ${col} AS a FROM ${table} WHERE ${col} = ANY($1::text[])`, [pgArray([...list])])).rows.map((r) => r.a) : [])

    const seenAddrs = await known('addresses', 'address', addrs)
    const newAddrs = [...addrs].filter((a) => !seenAddrs.has(a))
    const isContract = new Map<string, boolean>()
    const needCode = newAddrs.filter((a) => !created.has(a))
    for (const a of created) isContract.set(a, true)
    if (needCode.length) {
      const codes = await this.rpc.batch(needCode.map((a) => ({ method: 'eth_getCode', params: [a, at] })))
      needCode.forEach((a, i) => {
        const c = codes[i]
        isContract.set(a, typeof c === 'string' && codeKind(c) === 'contract')
      })
    }

    const seenTokens = await known('tokens', 'address', tokenAddrs)
    const newTokens = await fetchTokenMeta(this.rpc, [...tokenAddrs].filter((t) => !seenTokens.has(t)), at)

    const seenAccounts = await known('account_security', 'account', securityEmitters)
    const kinds = await accountKinds(this.rpc, [...securityEmitters].filter((a) => !seenAccounts.has(a)), at)
    return { isContract, newTokens, kinds }
  }

  private async write(q: Queryable, fetched: FetchedBlock[], prep: Awaited<ReturnType<Indexer['prepare']>>): Promise<void> {
    const bps = BigInt(this.burnBps)
    const blockRows: unknown[][] = []
    const txRows: unknown[][] = []
    const logRows: unknown[][] = []
    const txFrom = new Map<string, string>()
    let burnedTotal = 0n
    let treasuryTotal = 0n

    for (const { block, receipts } of fetched) {
      const n = hexToNum(block.number)
      const ts = hexToNum(block.timestamp)
      const baseFee = block.baseFeePerGas ? hexToBig(block.baseFeePerGas) : 0n
      let burned = 0n
      let treasury = 0n
      let priority = 0n
      block.transactions.forEach((tx, i) => {
        const r = receipts[i]!
        const gasUsed = hexToBig(r.gasUsed)
        const egp = r.effectiveGasPrice ? hexToBig(r.effectiveGasPrice) : hexToBig(tx.gasPrice)
        const fee = gasUsed * egp
        const basePart = gasUsed * (baseFee < egp ? baseFee : egp)
        const b = (basePart * bps) / 10_000n
        burned += b
        treasury += basePart - b
        priority += fee - basePart
        txFrom.set(tx.hash.toLowerCase(), tx.from.toLowerCase())
        const input = tx.input ?? '0x'
        txRows.push([
          tx.hash.toLowerCase(), n, hexToNum(tx.transactionIndex), ts, tx.from.toLowerCase(), lower(tx.to) ?? null,
          lower(r.contractAddress) ?? null, hexToBig(tx.value).toString(), hexToNum(tx.nonce), hexToNum(tx.type ?? '0x0'),
          hexToBig(tx.gas).toString(), tx.gasPrice ? hexToBig(tx.gasPrice).toString() : null,
          tx.maxFeePerGas ? hexToBig(tx.maxFeePerGas).toString() : null,
          tx.maxPriorityFeePerGas ? hexToBig(tx.maxPriorityFeePerGas).toString() : null, egp.toString(), gasUsed.toString(),
          r.status === undefined ? 1 : hexToNum(r.status), input.toLowerCase(), input.length >= 10 ? input.slice(0, 10).toLowerCase() : null,
          fee.toString(), basePart.toString(), b.toString(), (basePart - b).toString(), (fee - basePart).toString(),
          tx.authorizationList ? JSON.stringify(tx.authorizationList.map((a) => ({ chainId: a.chainId, address: lower(a.address), nonce: a.nonce }))) : null,
        ])
        for (const l of r.logs)
          logRows.push([n, hexToNum(l.logIndex), l.transactionHash.toLowerCase(), hexToNum(l.transactionIndex), l.address.toLowerCase(),
            lower(l.topics[0]) ?? null, lower(l.topics[1]) ?? null, lower(l.topics[2]) ?? null, lower(l.topics[3]) ?? null, l.data.toLowerCase()])
      })
      burnedTotal += burned
      treasuryTotal += treasury
      blockRows.push([n, block.hash.toLowerCase(), block.parentHash.toLowerCase(), ts, block.miner.toLowerCase(), hexToBig(block.gasUsed).toString(),
        hexToBig(block.gasLimit).toString(), block.baseFeePerGas ? baseFee.toString() : null, block.transactions.length,
        block.size ? hexToNum(block.size) : null, burned.toString(), treasury.toString(), priority.toString(), this.burnBps])
    }

    const newBlocks = new Set(
      (await insertMany<{ number: number }>(q, 'blocks',
        ['number', 'hash', 'parent_hash', 'timestamp', 'miner', 'gas_used', 'gas_limit', 'base_fee', 'tx_count', 'size', 'burned_fees', 'treasury_fees', 'priority_fees', 'burn_bps'],
        blockRows, 'ON CONFLICT DO NOTHING RETURNING number')).map((r) => Number(r.number)),
    )
    const newTx = new Set(
      (await insertMany<{ hash: string }>(q, 'transactions',
        ['hash', 'block_number', 'tx_index', 'timestamp', 'from_address', 'to_address', 'contract_address', 'value', 'nonce', 'type', 'gas', 'gas_price',
          'max_fee_per_gas', 'max_priority_fee_per_gas', 'effective_gas_price', 'gas_used', 'status', 'input', 'method_id', 'fee', 'base_fee_part',
          'burned', 'treasury', 'priority_fee', 'authorization_list'],
        txRows, 'ON CONFLICT DO NOTHING RETURNING hash', { authorization_list: 'jsonb' })).map((r) => r.hash),
    )
    await insertMany(q, 'logs', ['block_number', 'log_index', 'tx_hash', 'tx_index', 'address', 'topic0', 'topic1', 'topic2', 'topic3', 'data'], logRows, 'ON CONFLICT DO NOTHING')

    const firstBlock = hexToNum(fetched[0]!.block.number)
    await insertMany(q, 'tokens', ['address', 'name', 'symbol', 'decimals', 'first_seen_block'],
      prep.newTokens.map((t: TokenMeta) => [t.address, t.name, t.symbol, t.decimals, firstBlock]), 'ON CONFLICT DO NOTHING')
    for (const [account, kind] of prep.kinds)
      await q.query(`INSERT INTO account_security (account, account_kind, updated_block) VALUES ($1, $2, $3) ON CONFLICT (account) DO NOTHING`, [account, kind, firstBlock])

    const tokens = new TokenCache(q, this.cfg)
    const history = new History(q)
    const transferRows: unknown[][] = []
    const nativeSpam = new Map<string, { spam: boolean; reasons: string[] }>()
    const touched = new Map<string, { first: number; last: number; txs: number }>()
    const touch = (a: string | null | undefined, n: number, tx: boolean) => {
      if (!a || a === ZERO_ADDRESS) return
      const t = touched.get(a) ?? { first: n, last: n, txs: 0 }
      t.first = Math.min(t.first, n)
      t.last = Math.max(t.last, n)
      if (tx) t.txs++
      touched.set(a, t)
    }

    for (const { block, receipts } of fetched) {
      const n = hexToNum(block.number)
      const ts = hexToNum(block.timestamp)
      for (let i = 0; i < block.transactions.length; i++) {
        const tx = block.transactions[i]!
        const r = receipts[i]!
        const hash = tx.hash.toLowerCase()
        const from = tx.from.toLowerCase()
        const to = lower(tx.to) ?? null
        const isNew = newTx.has(hash)
        if (isNew) {
          const parties = new Set([from, to, lower(r.contractAddress)].filter(Boolean) as string[])
          for (const p of parties) touch(p, n, true)
        }
        const ok = r.status === undefined || hexToNum(r.status) === 1
        const value = hexToBig(tx.value)
        const plain = !tx.input || tx.input === '0x'
        if (ok && to && (value > 0n || plain)) {
          const res = await this.classify(history, tokens, 'native', null, from, to, value)
          nativeSpam.set(hash, res)
          transferRows.push([n, -(i + 1), hash, null, ts, from, 'native', null, from, to, value.toString(), res.spam, JSON.stringify(res.reasons), res.lookAlikeOf])
          if (!res.spam) history.add(from, to, n)
        } else if (ok && to && !nativeSpam.has(hash)) {
          history.add(from, to, n)
        }
        if (!ok) continue
        for (const l of r.logs) {
          const ev = decodeLog(l)
          if (!ev) continue
          const ctx: EventCtx = { blockNumber: n, blockHash: block.hash.toLowerCase(), timestamp: ts, logIndex: hexToNum(l.logIndex), txHash: hash, emitter: l.address.toLowerCase() }
          const tr = await this.transferOf(ev, ctx)
          if (tr) {
            const res = await this.classify(history, tokens, 'erc20', tr.token, tr.from, tr.to, tr.value)
            transferRows.push([n, ctx.logIndex, hash, ctx.logIndex, ts, from, 'erc20', tr.token, tr.from, tr.to, tr.value.toString(), res.spam, JSON.stringify(res.reasons), res.lookAlikeOf])
            touch(tr.from, n, false)
            touch(tr.to, n, false)
            if (!res.spam) history.add(tr.from, tr.to, n)
          }
          await applyEvent(q, this.cfg, ev, ctx)
          if (SECURITY_STANDARDS.has(ev.standard)) await this.queueAlert(q, ev, ctx)
        }
      }
    }

    const newTransfers = await insertMany(q, 'token_transfers',
      ['block_number', 'seq', 'tx_hash', 'log_index', 'timestamp', 'tx_from', 'kind', 'token', 'from_address', 'to_address', 'value', 'spam', 'spam_reasons', 'look_alike_of'],
      transferRows, 'ON CONFLICT DO NOTHING RETURNING seq', { spam_reasons: 'jsonb' })
    for (const [hash, s] of nativeSpam)
      if (s.spam || s.reasons.length) await q.query('UPDATE transactions SET spam = $2, spam_reasons = $3::jsonb WHERE hash = $1', [hash, s.spam, JSON.stringify(s.reasons)])

    const addrRows = [...touched].map(([a, t]) => [a, t.first, t.last, prep.isContract.get(a) ?? false, t.txs])
    const inserted = await insertMany<{ inserted: boolean }>(q, 'addresses', ['address', 'first_seen_block', 'last_seen_block', 'is_contract', 'tx_count'], addrRows,
      `ON CONFLICT (address) DO UPDATE SET first_seen_block = LEAST(addresses.first_seen_block, EXCLUDED.first_seen_block),
         last_seen_block = GREATEST(addresses.last_seen_block, EXCLUDED.last_seen_block),
         is_contract = addresses.is_contract OR EXCLUDED.is_contract, tx_count = addresses.tx_count + EXCLUDED.tx_count
       RETURNING (xmax = 0) AS inserted`)
    await history.flush()

    const blocksAdded = fetched.filter((f) => newBlocks.has(hexToNum(f.block.number)))
    const burnedNew = blocksAdded.length === fetched.length ? burnedTotal : 0n
    const treasuryNew = blocksAdded.length === fetched.length ? treasuryTotal : 0n
    await q.query(
      `UPDATE chain_stats SET tx_count = tx_count + $1, address_count = address_count + $2, transfer_count = transfer_count + $3,
         burned = burned + $4::numeric, treasury = treasury + $5::numeric WHERE id = 1`,
      [newTx.size, inserted.filter((r) => r.inserted).length, newTransfers.length, burnedNew.toString(), treasuryNew.toString()],
    )
    const last = fetched[fetched.length - 1]!.block
    await q.query(
      `INSERT INTO indexer_cursor (id, block_number, block_hash, updated_at) VALUES ($1, $2, $3, now())
       ON CONFLICT (id) DO UPDATE SET block_number = EXCLUDED.block_number, block_hash = EXCLUDED.block_hash, updated_at = now()`,
      [CURSOR_ID, hexToNum(last.number), last.hash.toLowerCase()],
    )
    if (hexToNum(fetched[0]!.block.number) === 0) await q.query('UPDATE chain_info SET genesis_hash = $1 WHERE id = 1', [fetched[0]!.block.hash.toLowerCase()])
  }

  private async transferOf(ev: DecodedEvent, ctx: EventCtx): Promise<{ token: string; from: string; to: string; value: bigint } | null> {
    if (ev.standard === 'erc20' && ev.event === 'Transfer')
      return { token: ctx.emitter, from: String(ev.args.from).toLowerCase(), to: String(ev.args.to).toLowerCase(), value: ev.args.value as bigint }
    if (ev.standard === 'wxds' && ctx.emitter === this.cfg.wxds.toLowerCase()) {
      if (ev.event === 'Deposit') return { token: ctx.emitter, from: ZERO_ADDRESS, to: String(ev.args.dst).toLowerCase(), value: ev.args.wad as bigint }
      if (ev.event === 'Withdrawal') return { token: ctx.emitter, from: String(ev.args.src).toLowerCase(), to: ZERO_ADDRESS, value: ev.args.wad as bigint }
    }
    return null
  }

  private async classify(history: History, tokens: TokenCache, kind: 'native' | 'erc20', token: string | null, from: string, to: string, value: bigint) {
    const info: TokenInfo | null = token ? await tokens.info(token) : null
    let lookAlikeOf: string | null = null
    const base = { kind, value, token: info, dustNativeWei: this.cfg.dustNativeWei }
    if (isPoisonCandidate(base) && from !== ZERO_ADDRESS && to !== ZERO_ADDRESS)
      lookAlikeOf = (await history.lookAlike(from, to)) ?? (await history.lookAlike(to, from))
    const res = classifyTransfer({ ...base, lookAlikeOf, protectedSymbols: await tokens.protectedSymbols() })
    return { ...res, reasons: res.reasons as string[], lookAlikeOf: res.reasons.includes('look-alike') ? lookAlikeOf : null }
  }

  private async queueAlert(q: Queryable, ev: DecodedEvent, ctx: EventCtx): Promise<void> {
    const type = ALERT_TYPES[ev.event]
    const a = this.cfg.alerts
    if (!type || !a.events.includes(type)) return
    if (ctx.timestamp < this.now() - a.maxAgeSeconds) return
    const sec = (await q.query<{ account_kind: AccountKind }>('SELECT account_kind FROM account_security WHERE account = $1', [ctx.emitter])).rows[0]
    const verified = !!sec && sec.account_kind !== 'unknown'
    if (!verified && !a.unverified) return
    const details = alertDetails(ev)
    const payload = {
      type, chainId: this.chainId, account: ctx.emitter, blockNumber: ctx.blockNumber, txHash: ctx.txHash, timestamp: ctx.timestamp, verified, details,
      explorerUrl: a.explorerUrl ? `${a.explorerUrl}/address/${ctx.emitter}` : null,
    }
    const destinations = a.webhookUrls.map((_u, i) => `webhook:${i}`)
    if (a.telegramBotToken) {
      const subs = await q.query<{ target: string }>(`SELECT target FROM alert_subscriptions WHERE account = $1 AND channel = 'telegram'`, [ctx.emitter])
      for (const s of subs.rows) destinations.push(`telegram:${s.target}`)
    }
    for (const d of destinations)
      await q.query(
        `INSERT INTO alert_outbox (dedupe_key, event, account, destination, payload) VALUES ($1, $2, $3, $4, $5::jsonb) ON CONFLICT (dedupe_key) DO NOTHING`,
        [`${ctx.blockHash}:${ctx.logIndex}:${d}`, type, ctx.emitter, d, JSON.stringify(payload)],
      )
  }

  /** Parent-hash mismatch: find the common ancestor, roll back, rebuild derived tables (never expected under BFT). */
  async handleDivergence(cur: { number: number; hash: string }): Promise<void> {
    let n = cur.number
    let ancestor: number | null = null
    for (let depth = 0; depth <= this.cfg.maxReorgDepth && n >= 0; depth++, n--) {
      const dbHash = (await this.db.query<{ hash: string }>('SELECT hash FROM blocks WHERE number = $1', [n])).rows[0]?.hash
      const rpcBlock = await this.rpc.request<RpcBlock | null>('eth_getBlockByNumber', [toHex(n), false])
      if (dbHash && rpcBlock && rpcBlock.hash.toLowerCase() === dbHash) {
        ancestor = n
        break
      }
    }
    if (ancestor === null)
      throw new FatalIndexerError(
        `chain diverged more than ${this.cfg.maxReorgDepth} blocks below block ${cur.number} (restarted dev chain?). Reset the database: npm run cli -- reset`,
      )
    this.log(`parent hash mismatch at ${cur.number + 1}: rolling back to common ancestor ${ancestor}`)
    await this.db.transaction(async (q) => {
      await rollbackTo(q, this.cfg, ancestor)
      await q.query('UPDATE indexer_cursor SET block_number = $2, block_hash = (SELECT hash FROM blocks WHERE number = $2), updated_at = now() WHERE id = $1', [CURSOR_ID, ancestor])
    })
  }
}

function alertDetails(ev: DecodedEvent): Record<string, unknown> {
  const a = ev.args
  switch (ev.event) {
    case 'VaultActionQueued':
      return { id: toJsonSafe(a.id), target: a.target, value: toJsonSafe(a.value), valueFormatted: `${formatEther(a.value as bigint)} tXDS`,
        data: String(a.data).slice(0, 202), executeAfter: Number(a.executeAfter) }
    case 'RecoveryApproved':
      return { id: a.id, guardian: a.guardian, approvals: Number(a.approvals) }
    case 'RecoveryReady':
      return { id: a.id, readyAt: Number(a.readyAt) }
    case 'RecoveryExecuted':
      return { id: a.id }
    case 'AccountFrozen':
      return { frozenUntil: Number(a.frozenUntil) }
    case 'EoaLockUpdated': {
      const s = Number(a.state)
      return { state: s, status: EOA_LOCK_STATES[s] ?? 'locked' }
    }
    default:
      return {}
  }
}

/** Token metadata + "known" status for the classification, cached per batch. */
class TokenCache {
  private readonly q: Queryable
  private readonly cfg: Config
  private readonly cache = new Map<string, TokenInfo>()
  private symbols: Set<string> | null = null
  constructor(q: Queryable, cfg: Config) {
    this.q = q
    this.cfg = cfg
  }
  private knownList(): string[] {
    return [this.cfg.wxds, ...this.cfg.knownTokens].map((a) => a.toLowerCase())
  }
  async info(token: string): Promise<TokenInfo> {
    const c = this.cache.get(token)
    if (c) return c
    const r = (await this.q.query<{ symbol: string | null; decimals: number | null; is_canonical: boolean }>('SELECT symbol, decimals, is_canonical FROM tokens WHERE address = $1', [token])).rows[0]
    const info = { known: this.knownList().includes(token) || !!r?.is_canonical, symbol: r?.symbol ?? null, decimals: r?.decimals ?? null }
    this.cache.set(token, info)
    return info
  }
  async protectedSymbols(): Promise<Set<string>> {
    if (this.symbols) return this.symbols
    const rows = await this.q.query<{ symbol: string | null }>(`SELECT symbol FROM tokens WHERE address = ANY($1::text[]) OR is_canonical`, [pgArray(this.knownList())])
    this.symbols = new Set([...NATIVE_SYMBOLS, ...rows.rows.map((r) => r.symbol?.trim().toUpperCase()).filter(Boolean) as string[]])
    return this.symbols
  }
}

/** Non-spam counterparties (DB + this batch) for the look-alike check. */
class History {
  private readonly q: Queryable
  private readonly pending = new Map<string, Map<string, number>>()
  constructor(q: Queryable) {
    this.q = q
  }
  add(a: string, b: string, block: number) {
    if (!a || !b || a === b || a === ZERO_ADDRESS || b === ZERO_ADDRESS) return
    for (const [x, y] of [[a, b], [b, a]] as const) {
      const m = this.pending.get(x) ?? new Map<string, number>()
      if (!m.has(y)) m.set(y, block)
      this.pending.set(x, m)
    }
  }
  /** A counterparty of `owner` that `candidate` imitates. */
  async lookAlike(owner: string, candidate: string): Promise<string | null> {
    const { prefix, suffix } = fingerprint(candidate)
    const rows = await this.q.query<{ counterparty: string }>(
      `SELECT counterparty FROM counterparties WHERE owner = $1 AND counterparty LIKE $2 AND right(counterparty, 4) = $3 AND counterparty <> $4 LIMIT 5`,
      [owner, `${prefix}%`, suffix, candidate],
    )
    const list = [...rows.rows.map((r) => r.counterparty), ...(this.pending.get(owner)?.keys() ?? [])]
    return findLookAlike(candidate, list)
  }
  async flush() {
    const rows: unknown[][] = []
    for (const [owner, m] of this.pending) for (const [cp, b] of m) rows.push([owner, cp, b])
    await insertMany(this.q, 'counterparties', ['owner', 'counterparty', 'first_block'], rows, 'ON CONFLICT DO NOTHING')
    this.pending.clear()
  }
}

/** Applies one decoded event to the derived tables (approvals, XNS, vault queue, account security). Idempotent. */
export async function applyEvent(q: Queryable, cfg: Config, ev: DecodedEvent, ctx: EventCtx): Promise<void> {
  const a = ev.args
  const s = (v: unknown) => String(v).toLowerCase()
  const pos = [ctx.blockNumber, ctx.logIndex, ctx.txHash] as const
  const upsertApproval = (scope: string, kind: string, token: string, owner: string, spender: string, amount: bigint, expiry: number | null) =>
    q.query(
      `INSERT INTO approvals (scope, kind, token, owner, spender, amount, expiry, revoked, block_number, log_index, tx_hash)
       VALUES ($1, $2, $3, $4, $5, $6::numeric, $7, false, $8, $9, $10)
       ON CONFLICT (scope, token, owner, spender) DO UPDATE SET kind = EXCLUDED.kind, amount = EXCLUDED.amount, expiry = EXCLUDED.expiry,
         revoked = false, block_number = EXCLUDED.block_number, log_index = EXCLUDED.log_index, tx_hash = EXCLUDED.tx_hash
       WHERE (approvals.block_number, approvals.log_index) <= (EXCLUDED.block_number, EXCLUDED.log_index)`,
      [scope, kind, token, owner, spender, amount.toString(), expiry, ...pos],
    )

  switch (ev.standard) {
    case 'erc20':
      if (ev.event === 'Approval') {
        const t = (await q.query<{ is_xdrc20: boolean }>('SELECT is_xdrc20 FROM tokens WHERE address = $1', [ctx.emitter])).rows[0]
        if (t?.is_xdrc20) return // the ApprovalWithExpiry emitted right after carries amount + expiry
        await upsertApproval('token', 'erc20', ctx.emitter, s(a.owner), s(a.spender), a.value as bigint, null)
      }
      return
    case 'xdrc20':
      if (ev.event === 'ApprovalWithExpiry') {
        await q.query('UPDATE tokens SET is_xdrc20 = true WHERE address = $1', [ctx.emitter])
        await upsertApproval('token', 'xdrc20', ctx.emitter, s(a.owner), s(a.spender), a.value as bigint, Number(a.expiry))
      } else if (ev.event === 'AllApprovalsRevoked') {
        await q.query(
          `UPDATE approvals SET revoked = true WHERE scope = 'token' AND token = $1 AND owner = $2 AND (block_number, log_index) < ($3, $4)`,
          [ctx.emitter, s(a.owner), ctx.blockNumber, ctx.logIndex],
        )
      }
      return
    case 'permit2':
      if (ctx.emitter !== cfg.permit2.toLowerCase()) return
      if (ev.event === 'Approval' || ev.event === 'Permit')
        await upsertApproval('permit2', 'permit2', s(a.token), s(a.owner), s(a.spender), a.amount as bigint, Number(a.expiration))
      else if (ev.event === 'Lockdown') await upsertApproval('permit2', 'permit2', s(a.token), s(a.owner), s(a.spender), 0n, null)
      else if (ev.event === 'AllRevoked')
        await q.query(
          `UPDATE approvals SET revoked = true WHERE scope = 'permit2' AND owner = $1 AND (block_number, log_index) < ($2, $3)`,
          [s(a.owner), ctx.blockNumber, ctx.logIndex],
        )
      return
    case 'xns':
      if (ctx.emitter !== cfg.xnsRegistry.toLowerCase()) return
      return applyXns(q, ev, ctx)
    case 'vault':
    case 'account':
      return applySecurity(q, ev, ctx)
    case 'factory':
      if (ev.event === 'AccountCreated' && ctx.emitter === predeploys.SMART_ACCOUNT_FACTORY.toLowerCase())
        await q.query(
          `INSERT INTO account_security (account, account_kind, updated_block) VALUES ($1, 'xdevs-proxy', $2)
           ON CONFLICT (account) DO UPDATE SET account_kind = 'xdevs-proxy'`,
          [s(a.account), ctx.blockNumber],
        )
      if (ev.event === 'CanonicalTokenCreated' && cfg.canonicalTokenFactory && ctx.emitter === cfg.canonicalTokenFactory.toLowerCase())
        await q.query(
          `INSERT INTO tokens (address, name, symbol, decimals, is_xdrc20, is_canonical, first_seen_block) VALUES ($1, $2, $3, $4, true, true, $5)
           ON CONFLICT (address) DO UPDATE SET is_canonical = true, is_xdrc20 = true`,
          [s(a.token), a.name, a.symbol, Number(a.decimals), ctx.blockNumber],
        )
      return
    default:
      return
  }
}

async function applyXns(q: Queryable, ev: DecodedEvent, ctx: EventCtx): Promise<void> {
  const a = ev.args
  const s = (v: unknown) => String(v).toLowerCase()
  switch (ev.event) {
    case 'NameRegistered': {
      const label = String(a.label)
      const name = `${label}.xdevs`
      let node: string
      try {
        node = xnsNamehash(name)
      } catch {
        return // the registry validates labels; ignore anything the SDK cannot normalize
      }
      await q.query(
        `INSERT INTO xns_names (labelhash, label, name, node, owner, addr, expires, registered_block, registered_tx, updated_block)
         VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8, $7)
         ON CONFLICT (labelhash) DO UPDATE SET owner = EXCLUDED.owner, addr = EXCLUDED.addr, expires = EXCLUDED.expires,
           registered_block = EXCLUDED.registered_block, registered_tx = EXCLUDED.registered_tx, updated_block = EXCLUDED.updated_block`,
        [s(a.labelhash), label, name, node, s(a.owner), Number(a.expires), ctx.blockNumber, ctx.txHash],
      )
      return
    }
    case 'NameRenewed':
      await q.query('UPDATE xns_names SET expires = $2, updated_block = $3 WHERE labelhash = $1', [s(a.labelhash), Number(a.expires), ctx.blockNumber])
      return
    case 'Transfer':
      await q.query('UPDATE xns_names SET owner = $2, updated_block = $3 WHERE node = $1', [s(a.node), s(a.owner), ctx.blockNumber])
      return
    case 'AddrChanged':
      await q.query('UPDATE xns_names SET addr = $2, updated_block = $3 WHERE node = $1', [s(a.node), s(a.a) === ZERO_ADDRESS ? null : s(a.a), ctx.blockNumber])
      return
    case 'VersionChanged':
      await q.query('UPDATE xns_names SET addr = NULL, updated_block = $2 WHERE node = $1', [s(a.node), ctx.blockNumber])
      await q.query('UPDATE xns_reverse SET name = NULL, updated_block = $2 WHERE node = $1', [s(a.node), ctx.blockNumber])
      return
    case 'ReverseClaimed':
      await q.query(
        `INSERT INTO xns_reverse (address, node, updated_block) VALUES ($1, $2, $3)
         ON CONFLICT (address) DO UPDATE SET node = EXCLUDED.node, updated_block = EXCLUDED.updated_block`,
        [s(a.addr), s(a.node), ctx.blockNumber],
      )
      return
    case 'NameChanged':
      await q.query('UPDATE xns_reverse SET name = $2, updated_block = $3 WHERE node = $1', [s(a.node), String(a.name).slice(0, 300) || null, ctx.blockNumber])
      return
    default:
      return
  }
}

async function applySecurity(q: Queryable, ev: DecodedEvent, ctx: EventCtx): Promise<void> {
  const a = ev.args
  const acct = ctx.emitter
  await q.query(
    `INSERT INTO security_events (block_number, log_index, tx_hash, account, event, args) VALUES ($1, $2, $3, $4, $5, $6::jsonb) ON CONFLICT DO NOTHING`,
    [ctx.blockNumber, ctx.logIndex, ctx.txHash, acct, ev.event, JSON.stringify(toJsonSafe(a))],
  )
  await q.query(
    `INSERT INTO account_security (account, updated_block) VALUES ($1, $2) ON CONFLICT (account) DO UPDATE SET updated_block = GREATEST(account_security.updated_block, EXCLUDED.updated_block)`,
    [acct, ctx.blockNumber],
  )
  const set = (sql: string, params: unknown[]) => q.query(`UPDATE account_security SET ${sql} WHERE account = $1`, [acct, ...params])
  const invalidateQueued = () =>
    q.query(`UPDATE vault_actions SET status = 'invalidated', final_block = $2, final_tx = $3 WHERE account = $1 AND status = 'queued'`, [acct, ctx.blockNumber, ctx.txHash])
  switch (ev.event) {
    case 'VaultActionQueued':
      await q.query(
        `INSERT INTO vault_actions (account, action_id, target, value, data, execute_after, status, queued_block, queued_tx)
         VALUES ($1, $2::numeric, $3, $4::numeric, $5, $6, 'queued', $7, $8)
         ON CONFLICT (account, action_id) DO UPDATE SET target = EXCLUDED.target, value = EXCLUDED.value, data = EXCLUDED.data,
           execute_after = EXCLUDED.execute_after, status = 'queued', queued_block = EXCLUDED.queued_block, queued_tx = EXCLUDED.queued_tx,
           cancelled_by = NULL, final_block = NULL, final_tx = NULL
         WHERE vault_actions.queued_block <= EXCLUDED.queued_block`,
        [acct, (a.id as bigint).toString(), String(a.target).toLowerCase(), (a.value as bigint).toString(), String(a.data).toLowerCase(), Number(a.executeAfter), ctx.blockNumber, ctx.txHash],
      )
      return
    case 'VaultActionCancelled':
      await q.query(
        `UPDATE vault_actions SET status = 'cancelled', cancelled_by = $3, final_block = $4, final_tx = $5 WHERE account = $1 AND action_id = $2::numeric`,
        [acct, (a.id as bigint).toString(), String(a.by).toLowerCase(), ctx.blockNumber, ctx.txHash],
      )
      return
    case 'VaultActionExecuted':
      await q.query(`UPDATE vault_actions SET status = 'executed', final_block = $3, final_tx = $4 WHERE account = $1 AND action_id = $2::numeric`, [
        acct, (a.id as bigint).toString(), ctx.blockNumber, ctx.txHash,
      ])
      return
    case 'VaultEpochBumped':
      await invalidateQueued()
      return
    case 'AccountInitialized':
      await set('vault_enabled = $2, guardian_count = $3', [a.vaultEnabled as boolean, Number(a.guardians)])
      return
    case 'VaultPolicyUpdated':
      await set('vault_enabled = $2', [(a.policy as { enabled: boolean }).enabled])
      return
    case 'GuardiansUpdated':
      await set('guardian_count = $2, guardian_threshold = $3', [(a.guardians as unknown[]).length, Number(a.threshold)])
      return
    case 'RecoveryApproved':
      await set('recovery_id = $2, recovery_approvals = $3', [String(a.id), Number(a.approvals)])
      return
    case 'RecoveryReady':
      await set('recovery_id = $2, recovery_ready_at = $3', [String(a.id), Number(a.readyAt)])
      return
    case 'RecoveryExecuted':
      await set('recovery_id = NULL, recovery_approvals = NULL, recovery_ready_at = NULL, frozen_until = NULL', [])
      await invalidateQueued()
      return
    case 'RecoveryCancelled':
      await set('recovery_id = NULL, recovery_approvals = NULL, recovery_ready_at = NULL', [])
      return
    case 'AccountFrozen':
      await set('frozen_until = $2', [Number(a.frozenUntil)])
      return
    case 'AccountUnfrozen':
      await set('frozen_until = NULL', [])
      return
    case 'EoaLockUpdated':
      await set('eoa_lock_state = $2', [Number(a.state)])
      return
    case 'EoaKeyRevoked':
      await set('eoa_key_revoked = true', [])
      return
    default:
      return
  }
}

/**
 * Deletes everything above `ancestor` and rebuilds the derived tables from the stored logs. Slow but exact;
 * only used after a parent-hash mismatch (BFT finality makes this a dev-chain-only path).
 */
export async function rollbackTo(q: Queryable, cfg: Config, ancestor: number): Promise<void> {
  await q.query('DELETE FROM blocks WHERE number > $1', [ancestor]) // cascades: transactions, logs, token_transfers, security_events
  const kinds = (await q.query<{ account: string; account_kind: string }>('SELECT account, account_kind FROM account_security')).rows
  for (const t of ['approvals', 'xns_names', 'xns_reverse', 'vault_actions', 'account_security', 'counterparties']) await q.query(`DELETE FROM ${t}`)
  await insertMany(q, 'account_security', ['account', 'account_kind', 'updated_block'], kinds.map((k) => [k.account, k.account_kind, 0]))
  await q.query('DELETE FROM tokens WHERE first_seen_block > $1', [ancestor])
  await q.query('UPDATE tokens SET is_xdrc20 = false')

  const PAGE = 2000
  let after: [number, number] = [-1, -1]
  for (;;) {
    const rows = (
      await q.query<{ block_number: number; log_index: number; tx_hash: string; address: string; topic0: string | null; topic1: string | null; topic2: string | null; topic3: string | null; data: string; block_hash: string; timestamp: number }>(
        `SELECT l.*, b.hash AS block_hash, b.timestamp FROM logs l JOIN blocks b ON b.number = l.block_number
         WHERE (l.block_number, l.log_index) > ($1, $2) ORDER BY l.block_number, l.log_index LIMIT ${PAGE}`,
        after,
      )
    ).rows
    for (const r of rows) {
      const topics = [r.topic0, r.topic1, r.topic2, r.topic3].filter((t): t is string => !!t)
      const ev = decodeLog({ address: r.address, topics, data: r.data })
      if (!ev) continue
      await applyEvent(q, cfg, ev, {
        blockNumber: Number(r.block_number), blockHash: r.block_hash, timestamp: Number(r.timestamp), logIndex: Number(r.log_index), txHash: r.tx_hash, emitter: r.address,
      })
    }
    if (rows.length < PAGE) break
    const last = rows[rows.length - 1]!
    after = [Number(last.block_number), Number(last.log_index)]
  }

  await q.query(
    `INSERT INTO counterparties (owner, counterparty, first_block)
     SELECT x.o, x.c, min(x.b) FROM (
       SELECT from_address AS o, to_address AS c, block_number AS b FROM transactions WHERE to_address IS NOT NULL AND status = 1 AND NOT spam
       UNION ALL SELECT to_address, from_address, block_number FROM transactions WHERE to_address IS NOT NULL AND status = 1 AND NOT spam
       UNION ALL SELECT from_address, to_address, block_number FROM token_transfers WHERE NOT spam
       UNION ALL SELECT to_address, from_address, block_number FROM token_transfers WHERE NOT spam
     ) x WHERE x.o <> x.c AND x.o <> '${ZERO_ADDRESS}' AND x.c <> '${ZERO_ADDRESS}' GROUP BY x.o, x.c`,
  )
  await q.query('DELETE FROM addresses WHERE first_seen_block > $1', [ancestor])
  await q.query(
    `UPDATE addresses a SET last_seen_block = $1,
       tx_count = (SELECT count(*) FROM transactions t WHERE t.from_address = a.address OR t.to_address = a.address OR t.contract_address = a.address)
     WHERE a.last_seen_block > $1`,
    [ancestor],
  )
  await q.query(
    `UPDATE chain_stats SET tx_count = (SELECT count(*) FROM transactions), address_count = (SELECT count(*) FROM addresses),
       transfer_count = (SELECT count(*) FROM token_transfers), burned = (SELECT coalesce(sum(burned_fees), 0) FROM blocks),
       treasury = (SELECT coalesce(sum(treasury_fees), 0) FROM blocks) WHERE id = 1`,
  )
}

export { RpcError }
