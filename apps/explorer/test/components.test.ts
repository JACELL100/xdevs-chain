// Component tests: server-render with react-dom/server (.tsx compiled by test/jsx-hooks.mjs).

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createElement as h } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { ApprovalTable, LookAlikeBanner, SecurityBadges, TransferTable, VaultTable } from '../src/components/tables.tsx'
import { Addr, Pill } from '../src/components/bits.tsx'
import { CommitCertificate } from '../src/components/CommitCert.tsx'
import { NotReachable } from '../src/components/NotReachable.tsx'
import { demoCommit, demoValidatorSet } from '../src/lib/fixtures.ts'
import type { AddressSummary, Approval, TokenTransfer, VaultAction } from '../src/lib/types.ts'

const REAL = '0x1234AaAaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678'
const FAKE = '0x1234bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678'
const NOW = 1_800_000_000
const token = { address: '0x70C0000000000000000000000000000000000002', name: 'Safe X', symbol: 'SAFX', decimals: 18, known: false }

test('status pills carry text, not only colour', () => {
  const html = renderToStaticMarkup(h(Pill, { tone: 'danger', children: 'unlimited' }))
  assert.match(html, /xu-badge is-alert/)
  assert.match(html, />unlimited</)
})

test('full addresses emphasise head and tail and stay accessible', () => {
  const html = renderToStaticMarkup(h(Addr, { address: FAKE, full: true }))
  assert.match(html, /aria-label="0x1234bbbb/)
  assert.match(html, /<b>0x1234<\/b>/)
  assert.match(html, /<b>5678<\/b>/)
  assert.match(html, /href="\/address\/0x1234bbbb/)
  // Predeploys get a readable label.
  assert.match(renderToStaticMarkup(h(Addr, { address: '0x0000000000000000000000000000000000001000' })), />Staking</)
})

test('LookAlikeBanner only renders when there are matches', () => {
  assert.equal(renderToStaticMarkup(h(LookAlikeBanner, { address: FAKE, matches: [] })), '')
  const html = renderToStaticMarkup(h(LookAlikeBanner, { address: FAKE, matches: [{ address: REAL, at: NOW * 1000, source: 'search' }] }))
  assert.match(html, /role="alert"/)
  assert.match(html, /Look-alike address warning/)
  assert.match(html, /searched for/)
  assert.ok(html.includes(REAL))
})

test('ApprovalTable shows unlimited, never-expiring and revoke hints', () => {
  const items: Approval[] = [
    { kind: 'erc20', token, spender: REAL, amount: `0x${'f'.repeat(64)}`, unlimited: true, expiry: null, expired: false, remaining: null, blockNumber: 1, txHash: '0xaa', revoke: { to: token.address, data: '0x095ea7b3', description: 'approve(spender, 0) on the token' } },
    { kind: 'xdrc20', token, spender: FAKE, amount: '0xde0b6b3a7640000', unlimited: false, expiry: NOW + 7200, expired: false, remaining: '0x6f05b59d3b20000', blockNumber: 2, txHash: '0xbb', revoke: { to: token.address, data: '0x095ea7b3', description: 'approve(spender, 0) on the token' } },
  ]
  const html = renderToStaticMarkup(h(ApprovalTable, { items, now: NOW }))
  assert.match(html, /is-alert">unlimited/)
  assert.match(html, /Never expires/)
  assert.match(html, /Expires in 2 h/)
  assert.match(html, /remaining now: 0.5 SAFX/)
  assert.match(html, /How to revoke/)
  assert.match(html, /unverified/)
})

test('TransferTable marks spam rows and names the imitated address', () => {
  const t: TokenTransfer = {
    blockNumber: 3, timestamp: NOW - 60, txHash: `0x${'cd'.repeat(32)}`, logIndex: 0, kind: 'erc20', token, from: REAL, to: FAKE, value: '0x0',
    spam: true, spamReasons: ['zero-value', 'look-alike'], lookAlikeOf: REAL,
  }
  const html = renderToStaticMarkup(h(TransferTable, { transfers: [t], now: NOW, perspective: REAL }))
  assert.match(html, /Zero value/)
  assert.match(html, /Look-alike address/)
  assert.match(html, /imitates/)
  assert.match(html, />out</)
  assert.match(html, /1 min ago/)
})

test('VaultTable and SecurityBadges', () => {
  const v: VaultAction = {
    account: REAL, id: '0x2', target: FAKE, value: '0x3635c9adc5dea00000', data: '0x', executeAfter: NOW + 3600, status: 'queued',
    queuedBlock: 1, queuedTx: `0x${'ee'.repeat(32)}`, cancelledBy: null, finalBlock: null, finalTx: null,
  }
  const vt = renderToStaticMarkup(h(VaultTable, { items: [v], now: NOW }))
  assert.match(vt, /1,000/)
  assert.match(vt, /tXDS/)
  assert.match(vt, /in 1 h/)
  assert.match(vt, /queued/)

  const s: AddressSummary['security'] = {
    accountKind: 'xdevs-7702', vaultEnabled: true, frozen: true, frozenUntil: NOW + 86400,
    eoaLock: { status: 'sealed', active: true, source: 'rpc' }, eoaKeyRevoked: false, recovery: { id: '0x1', approvals: 1, readyAt: null },
    guardians: { count: 3, threshold: 2 }, openVaultActions: 2,
  }
  const html = renderToStaticMarkup(h(SecurityBadges, { s }))
  for (const text of ['xDEVS smart account (EIP-7702)', 'Guardian Vault on', 'frozen until', 'EOA lock: sealed', 'recovery in progress', '2 queued vault actions'])
    assert.ok(html.includes(text), text)
  const none = renderToStaticMarkup(h(SecurityBadges, { s: { ...s, accountKind: null, vaultEnabled: null, frozen: false, eoaLock: { status: 'unknown', active: null, source: 'indexer' }, recovery: null, openVaultActions: 0 } }))
  assert.equal(none, '')
})

test('commit certificate shows signers, power share and the 2/3 line', () => {
  const set = demoValidatorSet()
  const commit = demoCommit(1_000_000)
  const html = renderToStaticMarkup(h(CommitCertificate, { commit, set }))
  assert.match(html, /Final\. Signed by \d of 6 validators/)
  assert.match(html, /2\/3/)
  assert.match(html, /aurora\.xdevs/)
  assert.match(html, /above the 2\/3 threshold/)
  const empty = renderToStaticMarkup(h(CommitCertificate, { commit: null, set: null }))
  assert.match(empty, /no certificate/)
})

test('the "not reachable" state explains itself and offers a retry', () => {
  const html = renderToStaticMarkup(h(NotReachable, { what: 'this block' }))
  assert.match(html, /Network not reachable yet/)
  assert.match(html, /launching soon/)
  assert.match(html, /rpc\.xdevschain\.tech/)
  assert.match(html, /explorer-api\.xdevschain\.tech/)
  assert.match(html, /Check again/)
})
