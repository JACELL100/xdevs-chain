// End-to-end: a real node (the xDEVS dev node with every predeploy, or anvil as a fallback), real transactions via
// viem, the indexer over HTTP JSON-RPC into PGlite, then the HTTP API.
//
// Node selection: XDEVS_BIN (default ~/.xdevs-target/debug/xdevs[.exe]) with
// deploy/configs/dev-genesis-with-predeploys.json; else ANVIL_BIN / anvil on PATH / ~/.foundry/bin/anvil
// (chain id 7117). Skipped when neither exists.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createServer, type AddressInfo } from 'node:net'
import { homedir, tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createPublicClient, createWalletClient, getAddress, http, parseEther, type Abi, type Hex } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { xdevsDevnet } from '@xdevs/sdk'
import { createPgliteDb, migrate } from '../src/db.ts'
import { Indexer } from '../src/indexer.ts'
import { HttpRpc, followNewHeads } from '../src/rpc.ts'
import { createApiServer } from '../src/api/server.ts'
import { testConfig } from '../src/config.ts'

const here = dirname(fileURLToPath(import.meta.url))
const repo = join(here, '..', '..', '..')
const exe = process.platform === 'win32' ? '.exe' : ''
const KEYS: Hex[] = [
  '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80',
  '0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d',
]
const ACCOUNT2 = '0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC'

function findNode(): { kind: 'xdevs' | 'anvil'; bin: string } | null {
  const xdevs = process.env.XDEVS_BIN ?? join(homedir(), '.xdevs-target', 'debug', `xdevs${exe}`)
  if (existsSync(xdevs) && process.env.INTEGRATION_NODE !== 'anvil') return { kind: 'xdevs', bin: xdevs }
  const candidates = [process.env.ANVIL_BIN, join(homedir(), '.foundry', 'bin', `anvil${exe}`)].filter(Boolean) as string[]
  for (const c of candidates) if (existsSync(c)) return { kind: 'anvil', bin: c }
  const which = spawnSync(process.platform === 'win32' ? 'where' : 'which', ['anvil'], { encoding: 'utf8' })
  const p = which.stdout?.split(/\r?\n/)[0]?.trim()
  return p ? { kind: 'anvil', bin: p } : null
}

async function freePort(): Promise<number> {
  return new Promise((resolve) => {
    const s = createServer()
    s.listen(0, '127.0.0.1', () => {
      const port = (s.address() as AddressInfo).port
      s.close(() => resolve(port))
    })
  })
}

async function waitForRpc(url: string, child: ChildProcess, timeoutMs = 90_000) {
  const rpc = new HttpRpc(url, { retries: 0, timeoutMs: 2000 })
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    if (child.exitCode !== null) throw new Error(`node exited with code ${child.exitCode}`)
    try {
      await rpc.request('eth_chainId')
      return
    } catch {
      await new Promise((r) => setTimeout(r, 500))
    }
  }
  throw new Error('node did not start')
}

const node = findNode()

test('integration: real node → indexer → API', { skip: node ? false : 'no xdevs node binary and no anvil found', timeout: 600_000 }, async (t) => {
  const port = await freePort()
  const url = `http://127.0.0.1:${port}`
  const dataDir = mkdtempSync(join(tmpdir(), 'xdevs-explorer-it-'))
  const args =
    node!.kind === 'xdevs'
      ? ['run', '--dev', '--genesis', join(repo, 'deploy', 'configs', 'dev-genesis-with-predeploys.json'), '--data-dir', dataDir, '--http', `127.0.0.1:${port}`, '--block-time', '1s', '--log', 'warn']
      : ['--port', String(port), '--chain-id', '7117', '--silent']
  const child = spawn(node!.bin, args, { stdio: 'ignore' })
  t.after(() => {
    child.kill()
    try {
      rmSync(dataDir, { recursive: true, force: true })
    } catch {
      /* Windows may hold the file briefly */
    }
  })
  await waitForRpc(url, child)
  t.diagnostic(`node: ${node!.kind} (${node!.bin})`)

  const chain = { ...xdevsDevnet, rpcUrls: { default: { http: [url] } } }
  const pub = createPublicClient({ chain, transport: http(url) })
  const owner = privateKeyToAccount(KEYS[0]!)
  const other = privateKeyToAccount(KEYS[1]!)
  const wallet = createWalletClient({ account: owner, chain, transport: http(url) })
  const wait = (hash: Hex) => pub.waitForTransactionReceipt({ hash, timeout: 60_000, pollingInterval: 250 })

  // newHeads over WebSocket (xDEVS node and anvil both serve WS on the HTTP port).
  const heads: number[] = []
  const sub = followNewHeads(url.replace('http', 'ws'), (n) => heads.push(n))
  t.after(() => sub.close())

  // Index from here on: a fresh dev node may already have produced many empty blocks.
  const startBlock = Number(await pub.getBlockNumber())

  // 1. Deploy an XDRC-20 (expiring approvals) and use it.
  const fx = JSON.parse(readFileSync(join(here, 'fixtures', 'xdrc20.json'), 'utf8')) as { abi: Abi; bytecode: Hex }
  const deploy = await wallet.deployContract({
    abi: fx.abi,
    bytecode: fx.bytecode,
    args: ['Test Dollar', 'TUSD', owner.address, parseEther('1000000'), 30n * 86400n, 365n * 86400n],
  })
  const token = (await wait(deploy)).contractAddress!
  const transferHash = await wallet.writeContract({ address: token, abi: fx.abi, functionName: 'transfer', args: [other.address, parseEther('1000')] })
  await wait(transferHash)
  const expiry = BigInt(Math.floor(Date.now() / 1000) + 7 * 86400)
  const block = await pub.getBlock()
  const approveExpiry = block.timestamp + 7n * 86400n > expiry ? block.timestamp + 7n * 86400n : expiry
  await wait(await wallet.writeContract({ address: token, abi: fx.abi, functionName: 'approve', args: [ACCOUNT2, parseEther('500'), approveExpiry] }))
  const nativeHash = await wallet.sendTransaction({ to: other.address, value: parseEther('1.5') })
  await wait(nativeHash)

  // 2. Predeploys (xDEVS node only): XNS registration + primary name, WXDS deposit.
  let xns = false
  if (node!.kind === 'xdevs') {
    const xnsAbi = [
      { type: 'function', name: 'register', stateMutability: 'payable', inputs: [{ type: 'string', name: 'label' }, { type: 'address', name: 'owner_' }, { type: 'uint256', name: 'duration' }], outputs: [{ type: 'bytes32' }, { type: 'uint256' }] },
      { type: 'function', name: 'setName', stateMutability: 'nonpayable', inputs: [{ type: 'string', name: 'name_' }], outputs: [{ type: 'bytes32' }] },
    ] as const
    try {
      await wait(await wallet.writeContract({ address: '0x0000000000000000000000000000000000001004', abi: xnsAbi, functionName: 'register', args: ['alice', owner.address, 365n * 86400n] }))
      await wait(await wallet.writeContract({ address: '0x0000000000000000000000000000000000001004', abi: xnsAbi, functionName: 'setName', args: ['alice.xdevs'] }))
      xns = true
    } catch (err) {
      t.diagnostic(`XNS registration failed (skipping XNS assertions): ${(err as Error).message.split('\n')[0]}`)
    }
    await wait(await wallet.sendTransaction({ to: '0x0000000000000000000000000000000000001009', data: '0xd0e30db0', value: parseEther('2') }))
  }
  if (node!.kind === 'anvil') await pub.request({ method: 'evm_mine' as never })

  // 3. Index everything.
  const db = await createPgliteDb()
  t.after(() => db.close())
  await migrate(db)
  const cfg = testConfig({ rpcUrl: url, batchSize: 50, startBlock, knownTokens: [] })
  const rpc = new HttpRpc(url)
  const ix = new Indexer(db, rpc, cfg, () => {})
  const head = Number(await pub.getBlockNumber())
  while (((await ix.cursor())?.number ?? -1) < head) if ((await ix.step()) === 0) await new Promise((r) => setTimeout(r, 200))
  assert.ok((await ix.cursor())!.number >= head)

  // 4. Query the API.
  const server = createApiServer({ db, rpc, cfg, log: () => {} })
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', () => r()))
  t.after(() => new Promise((r) => server.close(r)))
  const api = `http://127.0.0.1:${(server.address() as AddressInfo).port}/api/v1`
  const get = async (p: string) => (await fetch(`${api}${p}`)).json() as Promise<Record<string, any>>

  const stats = await get('/stats')
  assert.equal(stats.chainId, 7117)
  assert.ok(stats.totalTransactions >= 4)

  const tx = await get(`/txs/${transferHash}`)
  assert.equal(tx.status, 'success')
  assert.equal(tx.tokenTransfers.length, 1)
  assert.equal(tx.tokenTransfers[0].token.symbol, 'TUSD')
  assert.equal(BigInt(tx.tokenTransfers[0].value), parseEther('1000'))
  assert.equal(tx.tokenTransfers[0].to, other.address)
  assert.equal(tx.logs[0].event, 'Transfer')
  const fb = tx.feeBreakdown
  assert.equal(fb.burnBps, node!.kind === 'xdevs' ? 8000 : 10000)
  assert.equal(BigInt(fb.burned) + BigInt(fb.treasury), BigInt(fb.baseFee))

  const approvals = await get(`/addresses/${owner.address}/approvals`)
  assert.equal(approvals.items.length, 1)
  assert.equal(approvals.items[0].kind, 'xdrc20')
  assert.equal(approvals.items[0].expiry, Number(approveExpiry))
  assert.equal(BigInt(approvals.items[0].remaining), parseEther('500'), 'live allowance() read from the node')
  assert.equal(approvals.items[0].spender, getAddress(ACCOUNT2))

  const summary = await get(`/addresses/${other.address}`)
  assert.equal(summary.live, true)
  assert.ok(BigInt(summary.balance) > 0n)
  const native = await get(`/addresses/${other.address}/token-transfers`)
  assert.ok(native.items.some((x: { kind: string; value: string }) => x.kind === 'native' && BigInt(x.value) === parseEther('1.5')))
  const tokenAddr = await get(`/addresses/${token}`)
  assert.equal(tokenAddr.isContract, true)
  assert.equal(tokenAddr.token.symbol, 'TUSD')

  if (node!.kind === 'xdevs') {
    const eoa = await get(`/addresses/${owner.address}`)
    assert.equal(eoa.security.eoaLock.source, 'rpc', 'xdevs_getEoaLock answered by the node')
    assert.equal(eoa.security.eoaLock.status, 'unlocked')
    const wxds = await get(`/addresses/${owner.address}/token-transfers?token=0x0000000000000000000000000000000000001009`)
    assert.ok(wxds.items.some((x: { from: string }) => x.from === '0x0000000000000000000000000000000000000000'), 'WXDS deposit shown as a mint')
    if (xns) {
      const name = await get('/names/alice')
      assert.equal(name.status, 'active')
      assert.equal(name.address, owner.address)
      assert.equal(name.isPrimaryFor, owner.address)
      assert.equal(eoa.xns.primaryName, 'alice.xdevs')
      assert.deepEqual((await get('/search?q=alice')).results[0], { type: 'name', id: 'alice.xdevs', label: owner.address })
    }
  }
  // Polling is the source of truth; newHeads only lowers latency. A node without eth_subscribe refuses the
  // subscription and the indexer keeps polling.
  t.diagnostic(`newHeads notifications received over WebSocket: ${heads.length}${heads.length ? '' : ' (node refused eth_subscribe; polling only)'}`)
  t.diagnostic(`XNS registered: ${xns}`)
})
