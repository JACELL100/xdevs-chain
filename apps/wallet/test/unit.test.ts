// Pure unit tests (no chain): UserOperation packing / hashing helpers, dummy signatures, vault explanation,
// approvals revoke builders.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { type Address, concat, getAddress, decodeFunctionData, encodeFunctionData, keccak256, maxUint256, pad, parseEther, size, toHex } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { erc20Abi, permit2Abi, xdrc20Abi } from '../src/lib/abi.ts'
import { PERMIT2_EXPIRING, VAULT_OFF, recommendedPolicy } from '../src/lib/account.ts'
import { type ApprovalItem, revokeAllCalls, revokeCall } from '../src/lib/approvals.ts'
import { SoftPasskey, ecdsaSigner } from '../src/lib/signers.ts'
import { PAYMASTER_SIG_MAGIC, type UserOperation, packUserOp, paymasterDataKeccak, unpackUserOp } from '../src/lib/userop.ts'
import { explainVault } from '../src/lib/vault.ts'

const A = getAddress('0x00000000000000000000000000000000000000a1')
const B = getAddress('0x00000000000000000000000000000000000000b2')
const SELF = getAddress('0x00000000000000000000000000000000000000cc')

describe('UserOperation encoding', () => {
  it('the wallet copy of userop.ts matches the bundler (one implementation, two apps)', () => {
    const strip = (s: string) => s.split('\n').filter((l) => !l.startsWith('// Copied from')).join('\n')
    const w = readFileSync(new URL('../src/lib/userop.ts', import.meta.url), 'utf8')
    const b = readFileSync(new URL('../../bundler/src/userop.ts', import.meta.url), 'utf8')
    assert.equal(strip(w), strip(b))
  })

  it('packs gas fields as uint128 pairs and round-trips', () => {
    const op: UserOperation = {
      sender: A, nonce: 1n, factory: B, factoryData: '0xabcd', callData: '0x01',
      callGasLimit: 2n, verificationGasLimit: 3n, preVerificationGas: 4n, maxFeePerGas: 5n, maxPriorityFeePerGas: 6n,
      paymaster: B, paymasterVerificationGasLimit: 7n, paymasterPostOpGasLimit: 8n, paymasterData: '0x99', signature: '0x',
    }
    const p = packUserOp(op)
    assert.equal(p.accountGasLimits, concat([pad(toHex(3n), { size: 16 }), pad(toHex(2n), { size: 16 })]))
    assert.equal(p.gasFees, concat([pad(toHex(6n), { size: 16 }), pad(toHex(5n), { size: 16 })]))
    assert.equal(p.initCode, concat([B, '0xabcd']))
    assert.equal(size(p.paymasterAndData), 20 + 16 + 16 + 1)
    assert.deepEqual(unpackUserOp(p), op)
  })

  it('paymasterDataKeccak drops a v0.9 paymaster signature but keeps the magic', () => {
    const head = concat([B, pad('0x01', { size: 16 }), pad('0x02', { size: 16 }), A])
    const sig = toHex(new Uint8Array(65).fill(9))
    const withSig = concat([head, sig, '0x0041', PAYMASTER_SIG_MAGIC])
    assert.equal(paymasterDataKeccak(withSig), keccak256(concat([head, PAYMASTER_SIG_MAGIC])))
    assert.equal(paymasterDataKeccak(head), keccak256(head))
  })

  it('dummy signatures have the shape of real ones', async () => {
    const pk = await SoftPasskey.generate()
    const dummy = pk.signer().dummySignature()
    const real = await pk.signer().signHash(keccak256('0x01'))
    assert.equal(dummy.slice(0, 4), '0x02')
    assert.ok(Math.abs(size(dummy) - size(real)) <= 64, `${size(dummy)} vs ${size(real)}`)
    const e = ecdsaSigner(privateKeyToAccount(keccak256('0x02')))
    assert.equal(size(e.dummySignature()), 65)
    assert.equal(size(await e.signHash(keccak256('0x03'))), 65)
  })
})

describe('Guardian Vault explanation (local mirror)', () => {
  const st = { address: SELF, policy: recommendedPolicy({ nativeThreshold: parseEther('1'), dailyLimit: parseEther('2') }), whitelist: [] as Address[], spentToday: 0n }
  it('vault off => instant', () => {
    assert.equal(explainVault({ ...st, policy: VAULT_OFF }, [{ target: A, value: parseEther('100'), data: '0x' }]).kind, 'instant')
  })
  it('native: threshold, daily limit, whitelist', () => {
    assert.equal(explainVault(st, [{ target: A, value: parseEther('0.5'), data: '0x' }]).kind, 'instant')
    assert.equal(explainVault(st, [{ target: A, value: parseEther('1.5'), data: '0x' }]).kind, 'queued')
    assert.equal(explainVault({ ...st, spentToday: parseEther('1.8') }, [{ target: A, value: parseEther('0.5'), data: '0x' }]).kind, 'queued')
    assert.equal(explainVault({ ...st, whitelist: [A] }, [{ target: A, value: parseEther('50'), data: '0x' }]).kind, 'instant')
  })
  it('approvals: unlimited rejected, limited queued, revocation instant; self-calls', () => {
    const approve = (amt: bigint) => ({ target: B, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [A, amt] }) })
    assert.equal(explainVault(st, [approve(maxUint256)]).kind, 'rejected')
    assert.equal(explainVault({ ...st, policy: { ...st.policy, blockUnlimitedApprovals: false } }, [approve(maxUint256)]).kind, 'queued')
    assert.equal(explainVault(st, [approve(5n)]).kind, 'queued')
    assert.equal(explainVault(st, [approve(0n)]).kind, 'instant')
    const cancel = { target: SELF, value: 0n, data: encodeFunctionData({ abi: [{ type: 'function', name: 'cancel', inputs: [{ type: 'uint256', name: 'id' }], outputs: [], stateMutability: 'nonpayable' }], functionName: 'cancel', args: [1n] }) }
    assert.equal(explainVault(st, [cancel]).kind, 'instant')
    assert.equal(explainVault(st, [{ target: SELF, value: 0n, data: '0x12345678' }]).kind, 'queued')
  })
})

describe('approvals revoke builders', () => {
  const items: ApprovalItem[] = [
    { kind: 'erc20', token: A, symbol: 'A', spender: B, amount: 1n, unlimited: false, expiry: null, source: 'local' },
    { kind: 'xdrc20', token: B, symbol: 'B', spender: A, amount: 1n, unlimited: false, expiry: 1, source: 'local' },
    { kind: 'permit2', token: A, symbol: 'A', spender: A, amount: 1n, unlimited: false, expiry: 1, source: 'local' },
  ]
  it('single revocations are approve(spender, 0) / Permit2 approve(token, spender, 0, 0)', () => {
    const c = revokeCall(items[2])
    assert.equal(c.target, PERMIT2_EXPIRING)
    assert.deepEqual(decodeFunctionData({ abi: permit2Abi, data: c.data }).args, [A, A, 0n, 0])
    assert.deepEqual(decodeFunctionData({ abi: erc20Abi, data: revokeCall(items[0]).data }).args, [B, 0n])
  })
  it('revoke-all: per-pair with the vault on (instant path), epoch bumps with the vault off', () => {
    assert.equal(revokeAllCalls(items, true).length, 3)
    const off = revokeAllCalls(items, false)
    assert.equal(off.length, 3)
    assert.equal(decodeFunctionData({ abi: xdrc20Abi, data: off[1].data }).functionName, 'revokeAll')
    assert.equal(off[2].target, PERMIT2_EXPIRING)
  })
})
