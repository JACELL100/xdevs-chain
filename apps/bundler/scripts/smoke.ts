// Post-deployment smoke test for a running bundler.
//
//   node scripts/smoke.ts --bundler https://bundler.xdevschain.tech --rpc https://rpc.xdevschain.tech \
//        [--sponsor 0x<XdevsPaymaster sponsor>] [--send]
//
// Read-only by default: checks eth_chainId / eth_supportedEntryPoints / GET /health, builds the activation op of a
// fresh counterfactual account (random throwaway owner key, factory 0x…1006) and asks the bundler to estimate it.
// With --send (and --sponsor, so nobody needs to fund the account) it signs and submits the op and waits for the
// receipt: one real UserOperation, paid by the sponsor's XdevsPaymaster deposit.
import { type Address, type Hex, createPublicClient, encodeFunctionData, http, pad, parseAbi, toHex } from 'viem'
import { generatePrivateKey, privateKeyToAccount } from 'viem/accounts'
import { type UserOperation, fromRpcUserOp, getUserOpHash, toRpcUserOp } from '../src/userop.ts'

const arg = (n: string) => {
  const i = process.argv.indexOf(`--${n}`)
  return i >= 0 ? process.argv[i + 1] : undefined
}
const bundlerUrl = arg('bundler') ?? 'http://127.0.0.1:4337'
const rpcUrl = arg('rpc') ?? 'https://rpc.xdevschain.tech'
const sponsor = arg('sponsor') as Address | undefined
const doSend = process.argv.includes('--send')

const FACTORY: Address = '0x0000000000000000000000000000000000001006'
const PAYMASTER: Address = '0x0000000000000000000000000000000000001007'
const SIGNER = '(uint8 kind, bytes32 x, bytes32 y)'
const factoryAbi = parseAbi([
  `function getAddress(${SIGNER}[] signers, bytes32 salt) view returns (address)`,
  `function createAccount(${SIGNER}[] signers, bytes32 salt) returns (address)`,
])

let id = 0
async function call<T>(method: string, params: unknown[] = []): Promise<T> {
  const res = await fetch(bundlerUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++id, method, params }),
  })
  const j = (await res.json()) as { result?: T; error?: { code: number; message: string } }
  if (j.error) throw new Error(`${method}: ${j.error.code} ${j.error.message}`)
  return j.result as T
}

const ok = (m: string) => console.log(`ok   ${m}`)

const client = createPublicClient({ transport: http(rpcUrl) })
const chainId = await client.getChainId()
const bChain = Number(await call<Hex>('eth_chainId'))
if (bChain !== chainId) throw new Error(`bundler chain ${bChain} != RPC chain ${chainId}`)
ok(`eth_chainId ${chainId}`)
const [entryPoint] = await call<Address[]>('eth_supportedEntryPoints')
ok(`eth_supportedEntryPoints ${entryPoint}`)
const health = await fetch(new URL('/health', bundlerUrl))
const hb = (await health.json()) as Record<string, unknown>
console.log(`${health.ok ? 'ok  ' : 'WARN'} /health ${health.status} ${JSON.stringify(hb)}`)

const owner = privateKeyToAccount(generatePrivateKey()) // throwaway
const signers = [{ kind: 1, x: pad(owner.address), y: pad('0x00') }] as const
const salt = pad(toHex(BigInt(Date.now())))
const sender = await client.readContract({ address: FACTORY, abi: factoryAbi, functionName: 'getAddress', args: [signers, salt] })
const block = await client.getBlock({ blockTag: 'latest' })
const tip = await client.estimateMaxPriorityFeePerGas().catch(() => 1_000_000n)
let op: UserOperation = {
  sender,
  nonce: 0n,
  factory: FACTORY,
  factoryData: encodeFunctionData({ abi: factoryAbi, functionName: 'createAccount', args: [signers, salt] }),
  callData: '0x',
  callGasLimit: 0n,
  verificationGasLimit: 0n,
  preVerificationGas: 0n,
  maxFeePerGas: (block.baseFeePerGas ?? 0n) * 2n + tip,
  maxPriorityFeePerGas: tip,
  paymaster: sponsor ? PAYMASTER : null,
  paymasterData: sponsor ?? null,
  paymasterVerificationGasLimit: null,
  paymasterPostOpGasLimit: null,
  signature: toHex(new Uint8Array(65).fill(0x11)),
}
const est = await call<Record<string, Hex>>('eth_estimateUserOperationGas', [toRpcUserOp(op), entryPoint])
ok(`eth_estimateUserOperationGas ${JSON.stringify(Object.fromEntries(Object.entries(est).map(([k, v]) => [k, Number(v)])))}`)

if (!doSend) {
  console.log('read-only run complete (add --send --sponsor 0x… to submit a sponsored activation op)')
  process.exit(0)
}
if (!sponsor) throw new Error('--send needs --sponsor (an XdevsPaymaster sponsor) so the throwaway account needs no funds')
op = fromRpcUserOp({ ...toRpcUserOp(op), ...est })
op.signature = await owner.sign({ hash: getUserOpHash(op, chainId, entryPoint) })
const hash = await call<Hex>('eth_sendUserOperation', [toRpcUserOp(op), entryPoint])
ok(`eth_sendUserOperation ${hash}`)
for (let i = 0; i < 120; i++) {
  const r = await call<{ success: boolean; actualGasCost: Hex; receipt: { transactionHash: Hex } } | null>('eth_getUserOperationReceipt', [hash])
  if (r) {
    if (!r.success) throw new Error(`op included but reverted (tx ${r.receipt.transactionHash})`)
    ok(`included in ${r.receipt.transactionHash}, account ${sender} deployed, gas cost ${Number(r.actualGasCost) / 1e18} tXDS`)
    process.exit(0)
  }
  await new Promise((res) => setTimeout(res, 1_000))
}
throw new Error('no receipt after 120 s')
