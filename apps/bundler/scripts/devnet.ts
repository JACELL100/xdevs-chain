// Dev-chain bootstrap for the reference wallet, the dev bundler and their tests (DEV ONLY).
//
//   * starts a chain: the real xDEVS node (`xdevs run --dev --genesis deploy/configs/dev-genesis-with-predeploys.json`)
//     or, as a fallback, `anvil --hardfork osaka --chain-id 7117` with the SAME genesis alloc applied (predeploy
//     code + storage, EntryPoint v0.9 and its SenderCreator, XNS, Permit2Expiring, ...);
//   * configures a gas sponsor in the XdevsPaymaster predeploy (0x…1007) and stakes the paymaster;
//   * deploys an XDRC-20 test token (from the Foundry artifacts in contracts/out-wallet, built on demand).
//
// CLI:  node scripts/devnet.ts [--chain xdevs|anvil|auto] [--port 8545] [--rpc http://…(existing chain)]
import { type ChildProcess, execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  type Address,
  type Hex,
  createPublicClient,
  createWalletClient,
  getAddress,
  http,
  parseAbi,
  parseEther,
  toHex,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'

export const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
export const DEV_GENESIS = join(REPO_ROOT, 'deploy', 'configs', 'dev-genesis-with-predeploys.json')
export const CHAIN_ID = 7117

/** The 10 well-known Anvil / Hardhat dev keys (funded by the xDEVS dev genesis too). TEST ONLY. */
export const DEV_KEYS: Hex[] = [
  '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80',
  '0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d',
  '0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a',
  '0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6',
  '0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a',
  '0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba',
  '0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e',
  '0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356',
  '0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97',
  '0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6',
]

/** Role assignment used by the devnet setup and the tests. */
export const ROLES = {
  faucet: 0,
  sponsor: 1,
  bundler: 2,
  guardian1: 3,
  guardian2: 4,
  guardian3: 5,
  eoa7702: 6,
  spender: 7,
  recipient: 8,
} as const

export const PAYMASTER: Address = '0x0000000000000000000000000000000000001007'
export const ENTRY_POINT: Address = '0x433709009B8330FDa32311DF1C2AFA402eD8D009'

export type Chain = { kind: 'xdevs' | 'anvil' | 'external'; rpcUrl: string; stop: () => Promise<void> }

async function rpc(url: string, method: string, params: unknown[] = []): Promise<unknown> {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  })
  const j = (await res.json()) as { result?: unknown; error?: { message: string } }
  if (j.error) throw new Error(`${method}: ${j.error.message}`)
  return j.result
}

async function waitForRpc(url: string, proc: ChildProcess, timeoutMs = 60_000): Promise<void> {
  const until = Date.now() + timeoutMs
  let exited = false
  proc.once('exit', () => {
    exited = true
  })
  while (Date.now() < until) {
    if (exited) throw new Error(`chain process exited early (${url})`)
    try {
      await rpc(url, 'eth_chainId')
      return
    } catch {
      await new Promise((r) => setTimeout(r, 300))
    }
  }
  throw new Error(`chain at ${url} did not come up`)
}

function kill(proc: ChildProcess): Promise<void> {
  return new Promise((resolve) => {
    if (proc.exitCode !== null || proc.killed) return resolve()
    proc.once('exit', () => resolve())
    if (process.platform === 'win32' && proc.pid) {
      try {
        execFileSync('taskkill', ['/F', '/T', '/PID', String(proc.pid)], { stdio: 'ignore' })
      } catch {
        proc.kill()
      }
    } else proc.kill('SIGTERM')
    setTimeout(resolve, 5_000)
  })
}

/** Locates the `xdevs` node binary (env XDEVS_BIN, the cargo target dirs). */
export function findXdevsBinary(): string | null {
  const exe = process.platform === 'win32' ? 'xdevs.exe' : 'xdevs'
  const candidates = [
    process.env.XDEVS_BIN,
    process.env.CARGO_TARGET_DIR && join(process.env.CARGO_TARGET_DIR, 'debug', exe),
    join(REPO_ROOT, 'target', 'release', exe),
    join(REPO_ROOT, 'target', 'debug', exe),
    join(homedir(), '.xdevs-target', 'debug', exe),
    join(homedir(), '.xdevs-target', 'release', exe),
  ].filter((p): p is string => !!p)
  return candidates.find((p) => existsSync(p)) ?? null
}

export async function startXdevsNode(port: number, blockTime = '1s'): Promise<Chain> {
  const bin = findXdevsBinary()
  if (!bin) throw new Error('xdevs binary not found (set XDEVS_BIN)')
  const dataDir = mkdtempSync(join(tmpdir(), 'xdevs-wallet-dev-'))
  const proc = spawn(
    bin,
    ['run', '--dev', '--genesis', DEV_GENESIS, '--data-dir', dataDir, '--http', `127.0.0.1:${port}`, '--block-time', blockTime],
    { stdio: 'ignore' },
  )
  const rpcUrl = `http://127.0.0.1:${port}`
  await waitForRpc(rpcUrl, proc)
  return {
    kind: 'xdevs',
    rpcUrl,
    stop: async () => {
      await kill(proc)
      rmSync(dataDir, { recursive: true, force: true })
    },
  }
}

export async function startAnvil(port: number): Promise<Chain> {
  const proc = spawn(
    'anvil',
    ['--hardfork', 'osaka', '--chain-id', String(CHAIN_ID), '--port', String(port), '--silent', '--base-fee', '1000000000'],
    { stdio: 'ignore' },
  )
  const rpcUrl = `http://127.0.0.1:${port}`
  await waitForRpc(rpcUrl, proc)
  await applyGenesisAlloc(rpcUrl)
  return { kind: 'anvil', rpcUrl, stop: () => kill(proc) }
}

/** Copies every code-bearing account (code + storage) of the xDEVS dev genesis into an anvil chain. */
export async function applyGenesisAlloc(rpcUrl: string, genesisPath = DEV_GENESIS): Promise<void> {
  const g = JSON.parse(readFileSync(genesisPath, 'utf8')) as {
    alloc: Record<string, { code?: Hex; storage?: Record<string, Hex>; balance?: string }>
  }
  for (const [addr, acc] of Object.entries(g.alloc)) {
    if (acc.code) await rpc(rpcUrl, 'anvil_setCode', [addr, acc.code])
    for (const [slot, value] of Object.entries(acc.storage ?? {})) {
      await rpc(rpcUrl, 'anvil_setStorageAt', [addr, toHex(BigInt(slot), { size: 32 }), toHex(BigInt(value), { size: 32 })])
    }
    if (acc.code && acc.balance && BigInt(acc.balance) > 0n) {
      await rpc(rpcUrl, 'anvil_setBalance', [addr, toHex(BigInt(acc.balance))])
    }
  }
}

/** Starts the preferred chain: the xDEVS node if its binary is found (or `kind === 'xdevs'`), else anvil. */
export async function startChain(kind: 'xdevs' | 'anvil' | 'auto', port: number): Promise<Chain> {
  if (kind === 'xdevs' || (kind === 'auto' && findXdevsBinary())) return startXdevsNode(port)
  return startAnvil(port)
}

const paymasterAbi = parseAbi([
  'function configure(bool active, bool openToAll, uint128 maxCostPerOp, uint128 perAccountCap, address signer)',
  'function deposit(address sponsor) payable',
  'function addStake(uint32 unstakeDelaySec) payable',
  'function sponsorInfo(address sponsor) view returns ((uint128 balance, uint128 maxCostPerOp, uint128 perAccountCap, bool active, bool openToAll, address signer))',
])
const epAbi = parseAbi([
  'function getDepositInfo(address account) view returns ((uint256 deposit, bool staked, uint112 stake, uint32 unstakeDelaySec, uint48 withdrawTime) info)',
])

/** Reads (building on demand) a Foundry artifact from contracts/out-wallet. */
export function contractArtifact(file: string, name: string): { abi: unknown[]; bytecode: Hex } {
  const outDir = join(REPO_ROOT, 'contracts', 'out-wallet')
  const p = join(outDir, file, `${name}.json`)
  if (!existsSync(p)) {
    execFileSync('forge', ['build', '--root', join(REPO_ROOT, 'contracts'), '--out', 'out-wallet', '--cache-path', 'cache-wallet'], {
      stdio: 'ignore', // forge prints many lint notes; a failed build still throws
    })
  }
  const a = JSON.parse(readFileSync(p, 'utf8')) as { abi: unknown[]; bytecode: { object: Hex } }
  return { abi: a.abi, bytecode: a.bytecode.object }
}

export type Devnet = {
  rpcUrl: string
  chainId: number
  sponsor: Address
  token: Address
  faucetKey: Hex
}

/** Paymaster sponsor + stake, XDRC-20 test token. Idempotent enough to run against an already-set-up chain. */
export async function setupDevnet(rpcUrl: string, opts: { token?: boolean } = {}): Promise<Devnet> {
  const client = createPublicClient({ transport: http(rpcUrl), pollingInterval: 200 })
  const chainId = await client.getChainId()
  const sponsor = privateKeyToAccount(DEV_KEYS[ROLES.sponsor])
  const faucet = privateKeyToAccount(DEV_KEYS[ROLES.faucet])
  const sw = createWalletClient({ account: sponsor, transport: http(rpcUrl) })
  const fw = createWalletClient({ account: faucet, transport: http(rpcUrl) })
  const wait = async (hash: Hex) => {
    const r = await client.waitForTransactionReceipt({ hash, timeout: 60_000 })
    if (r.status !== 'success') throw new Error(`setup tx ${hash} reverted`)
    return r
  }

  const info = await client.readContract({ address: PAYMASTER, abi: paymasterAbi, functionName: 'sponsorInfo', args: [sponsor.address] })
  if (!info.active) {
    await wait(
      await sw.writeContract({
        chain: null,
        address: PAYMASTER,
        abi: paymasterAbi,
        functionName: 'configure',
        args: [true, true, parseEther('0.05'), parseEther('1'), '0x0000000000000000000000000000000000000000'],
      }),
    )
    await wait(
      await sw.writeContract({ chain: null, address: PAYMASTER, abi: paymasterAbi, functionName: 'deposit', args: [sponsor.address], value: parseEther('20') }),
    )
  }
  const dep = await client.readContract({ address: ENTRY_POINT, abi: epAbi, functionName: 'getDepositInfo', args: [PAYMASTER] })
  if (!dep.staked) {
    await wait(
      await sw.writeContract({ chain: null, address: PAYMASTER, abi: paymasterAbi, functionName: 'addStake', args: [86_400], value: parseEther('1') }),
    )
  }

  let token: Address = '0x0000000000000000000000000000000000000000'
  if (opts.token !== false) {
    const art = contractArtifact('XDRC20.sol', 'XDRC20')
    const hash = await fw.deployContract({
      chain: null,
      abi: art.abi,
      bytecode: art.bytecode,
      args: ['Test USD', 'tUSD', faucet.address, parseEther('1000000'), 30n * 86_400n, 365n * 86_400n],
    })
    token = getAddress((await wait(hash)).contractAddress as Address)
  }
  return { rpcUrl, chainId, sponsor: sponsor.address, token, faucetKey: DEV_KEYS[ROLES.faucet] }
}

// ------------------------------------------------------------------------------------------------ CLI
if (import.meta.main) {
  const arg = (name: string) => {
    const i = process.argv.indexOf(`--${name}`)
    return i >= 0 ? process.argv[i + 1] : undefined
  }
  const port = Number(arg('port') ?? 8545)
  const existing = arg('rpc')
  const kind = (arg('chain') ?? 'auto') as 'xdevs' | 'anvil' | 'auto'
  const chain: Chain = existing
    ? { kind: 'external', rpcUrl: existing, stop: async () => {} }
    : await startChain(kind, port)
  const d = await setupDevnet(chain.rpcUrl)
  const bundlerKey = DEV_KEYS[ROLES.bundler]
  console.log(`
xDEVS dev chain ready (${chain.kind}) at ${chain.rpcUrl}, chain id ${d.chainId}  -- DEV ONLY, keys are public test keys

Paymaster sponsor : ${d.sponsor} (openToAll, 0.05 tXDS max per op)
XDRC-20 test token: ${d.token}

Start the dev bundler (another terminal, apps/bundler):
  BUNDLER_PRIVATE_KEY=${bundlerKey} RPC_URL=${chain.rpcUrl} npm start

apps/wallet/.env.local:
  VITE_RPC_URL=${chain.rpcUrl}
  VITE_BUNDLER_URL=http://127.0.0.1:4337
  VITE_CHAIN_ID=${d.chainId}
  VITE_PAYMASTER_SPONSOR=${d.sponsor}
  VITE_KNOWN_TOKENS=${d.token}
  VITE_DEV_TOOLS=true
`)
  if (chain.kind !== 'external') {
    console.log('Ctrl-C stops the chain.')
    const stop = async () => {
      await chain.stop()
      process.exit(0)
    }
    process.on('SIGINT', stop)
    process.on('SIGTERM', stop)
  }
}
