// Pure tests of the Next.js wallet's own logic (no chain): formatting / strict amount parsing, passkey sign-in
// verification, and the demo backend (vault decisions, look-alike recipients, cancel / revoke).
import assert from 'node:assert/strict'
import { beforeEach, describe, it } from 'node:test'
import { type Address, encodeFunctionData, getAddress, keccak256, parseEther, toHex } from 'viem'
import { DEMO_ACCOUNT, POISON, demoBackend, demoTokens, resetDemo, setDemoSpeed } from '../src/data/demo.ts'
import { erc20Abi } from '../src/lib/abi.ts'
import { cancelCallData } from '../src/lib/vault.ts'
import { fmtAmount, fmtCountdown, fmtDuration, groupAddr, parseAmount, shortAddr } from '../src/lib/format.ts'
import { matchPasskeySigner, rpIdMatchesHost, verifyAssertion } from '../src/lib/passkey-auth.ts'
import { SoftPasskey, passkeySignerStruct } from '../src/lib/signers.ts'
import type { AccountRecord } from '../src/store.ts'

describe('formatting', () => {
  it('amounts: separators, truncation (never rounds up), dust', () => {
    assert.equal(fmtAmount(parseEther('1284.5')), '1,284.5')
    assert.equal(fmtAmount(parseEther('0.99999')), '0.9999')
    assert.equal(fmtAmount(1n), '< 0.0001')
    assert.equal(fmtAmount(0n), '0')
    assert.equal(fmtAmount(3_250_000_000n, 6, 2), '3,250')
  })
  it('strict amount parsing rejects ambiguous input', () => {
    assert.equal(parseAmount('12.5'), parseEther('12.5'))
    assert.equal(parseAmount('.5'), parseEther('0.5'))
    for (const bad of ['', '1,5', '1e3', '-1', '1.2.3', ' . ', '0x10']) assert.throws(() => parseAmount(bad), bad)
    assert.throws(() => parseAmount('1.1234567', 6))
  })
  it('addresses and durations', () => {
    assert.equal(shortAddr('0x5a1E7c0B9d24F6a83E10c4B7d9A2e6F1b3C8a7D4'), '0x5a1E…a7D4')
    assert.equal(groupAddr('0x5a1E7c0B9d24F6a83E10c4B7d9A2e6F1b3C8a7D4').length, 10)
    assert.equal(fmtDuration(86_400), '24 hours')
    assert.equal(fmtDuration(3 * 86_400), '3 days')
    assert.equal(fmtCountdown(17 * 3600 + 12 * 60), '17h 12m')
    assert.equal(fmtCountdown(0), 'now')
  })
})

describe('passkey sign-in verification', () => {
  it('matches an assertion to the right on-chain passkey signer only', async () => {
    const pk = await SoftPasskey.generate('wallet.xdevschain.tech', 'https://wallet.xdevschain.tech')
    const other = await SoftPasskey.generate()
    const a = await pk.assert(keccak256(toHex('challenge')))
    assert.equal(await verifyAssertion(pk.publicKey, a), true)
    assert.equal(await verifyAssertion(other.publicKey, a), false)
    assert.equal(await verifyAssertion(pk.publicKey, { ...a, clientDataJSON: a.clientDataJSON.replace('webauthn', 'webauthm') }), false)
    const signers = [passkeySignerStruct(other.publicKey), { kind: 1, x: pad64('0x01'), y: pad64('0x00') }, passkeySignerStruct(pk.publicKey)]
    assert.deepEqual(await matchPasskeySigner(signers, a), pk.publicKey)
    assert.equal(await matchPasskeySigner([passkeySignerStruct(other.publicKey)], a), null)
  })
  it('RP ID must be the host or a registrable suffix of it', () => {
    assert.equal(rpIdMatchesHost('wallet.xdevschain.tech', 'wallet.xdevschain.tech'), true)
    assert.equal(rpIdMatchesHost('xdevschain.tech', 'wallet.xdevschain.tech'), true)
    assert.equal(rpIdMatchesHost('wallet.xdevschain.tech', 'evil-xdevschain.tech'), false)
    assert.equal(rpIdMatchesHost('localhost', 'localhost'), true)
  })
})

const pad64 = (h: `0x${string}`) => `0x${h.slice(2).padStart(64, '0')}` as `0x${string}`

describe('demo backend', () => {
  const rec: AccountRecord = { address: DEMO_ACCOUNT, label: 'Demo', kind: 'passkey', createdAt: 0 }
  const quiet = () => {}
  beforeEach(() => {
    resetDemo()
    setDemoSpeed(0)
  })

  it('small sends are instant, large ones wait in the Guardian Vault', async () => {
    const st = await demoBackend.readAccount(DEMO_ACCOUNT)
    const to = getAddress('0x00000000000000000000000000000000000000b2')
    const small = await demoBackend.review(st, { target: to, value: parseEther('10'), data: '0x' }, null, null)
    assert.equal(small.vault.kind, 'instant')
    const big = await demoBackend.review(st, { target: to, value: parseEther('500'), data: '0x' }, null, null)
    assert.equal(big.vault.kind, 'queued')
    const before = (await demoBackend.vaultQueue(DEMO_ACCOUNT)).length
    const r = await demoBackend.sendOp(rec, { calls: [big.call], describe: { kind: 'sent', title: 'Send' } }, quiet)
    assert.equal(r.queuedIds.length, 1)
    assert.equal((await demoBackend.vaultQueue(DEMO_ACCOUNT)).length, before + 1)
    assert.equal((await demoBackend.readAccount(DEMO_ACCOUNT)).balance, st.balance, 'nothing left the account yet')
  })

  it('flags the poisoned look-alike of a past recipient', async () => {
    const check = await demoBackend.checkRecipient(DEMO_ACCOUNT, POISON, [])
    assert.equal(check.lookAlikeOf.length, 1)
    const st = await demoBackend.readAccount(DEMO_ACCOUNT)
    const rev = await demoBackend.review(st, { target: POISON, value: parseEther('1'), data: '0x' }, null, check)
    assert.ok(rev.preview.warnings.some((w) => w.code === 'LOOKALIKE_RECIPIENT' && w.severity === 'high'))
  })

  it('rejects homograph names and unlimited approvals', async () => {
    await assert.rejects(demoBackend.resolveRecipient('mаya.xdevs')) // Cyrillic а
    assert.equal((await demoBackend.resolveRecipient('Maya')).name, 'maya.xdevs')
    const st = await demoBackend.readAccount(DEMO_ACCOUNT)
    const approve = { target: demoTokens()[0] as Address, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [POISON, 2n ** 256n - 1n] }) }
    assert.equal((await demoBackend.review(st, approve, null, null)).vault.kind, 'rejected')
    assert.equal((await demoBackend.xnsQuote('nova')).available, true)
    assert.equal((await demoBackend.xnsQuote('rnaya')).available, false, 'rn looks like m: confusable with maya')
    assert.equal((await demoBackend.xnsQuote('ren')).available, false)
    assert.equal((await demoBackend.xnsQuote('f4ucet')).available, true)
    assert.equal((await demoBackend.xnsQuote('fauceт')).available, false)
  })

  it('owner cancel and revocations are instant', async () => {
    const q = await demoBackend.vaultQueue(DEMO_ACCOUNT)
    const open = q.find((a) => a.status === 'queued')
    assert.ok(open)
    await demoBackend.sendOp(rec, { callData: cancelCallData(open.id), describe: { kind: 'cancelled', title: 'Cancel' } }, quiet)
    assert.equal((await demoBackend.vaultQueue(DEMO_ACCOUNT)).find((a) => a.id === open.id)?.status, 'cancelled')
    const before = await demoBackend.approvals(DEMO_ACCOUNT)
    const a = before[0]
    await demoBackend.sendOp(rec, { calls: [{ target: a.token, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [a.spender, 0n] }) }], describe: { kind: 'revoke', title: 'Revoke' } }, quiet)
    assert.equal((await demoBackend.approvals(DEMO_ACCOUNT)).length, before.length - 1)
  })
})
