# xDEVS reference wallet and dev bundler — Specification

Status: testnet reference implementation (Phase 4). Plan refs: §16 (user-safety layer), §23 (tooling), §27 Phase 4 exit
criteria. Companion specs: [`smart-accounts.md`](smart-accounts.md), [`xip-0001-eoa-lock.md`](xip-0001-eoa-lock.md),
[`xns-and-approvals.md`](xns-and-approvals.md), [`sdk.md`](sdk.md), [`explorer-api.md`](explorer-api.md).

Sources: `apps/wallet/` (Vite + React SPA, logic in `src/lib/`, UI in `src/ui/`), `apps/bundler/` (DEV-ONLY ERC-4337
v0.9 bundler). Tests: `apps/wallet/test/`, `apps/bundler/test/`.

---

## 1. Components

| Piece | Role | Production-ready? |
|---|---|---|
| `apps/wallet` | Static SPA: passkey sign-up, send, Guardian Vault, approvals, guardian tools, EIP-7702 / EOA lock | Reference / testnet. Planned host: Cloudflare Pages (`xdevschain-wallet.xdevs.in`), with a real bundler |
| `apps/wallet/src/lib` | Framework-free logic (also used by the Node tests): UserOp encoding, WebAuthn encoding, account state, vault outcome, preview, recipient checks, approvals, 7702 | — |
| `apps/bundler` | Single-operator ERC-4337 v0.9 bundler for dev chains | **No** (dev only, see §8) |
| `@xdevs/sdk` (`sdk/ts`, linked with `file:`) | `xdevs_*` RPC actions, XNS normalization / namehash, look-alike helpers | — |

Chain-side pieces used: `XdevsSmartAccount` (`0x…1005`), `SmartAccountFactory` (`0x…1006`, also the extension host),
`XdevsPaymaster` (`0x…1007`), EntryPoint v0.9 (`0x4337…D009`), `XNSRegistry` (`0x…1004`), `Permit2Expiring`
(`0x…1008`), `P256VERIFY` (`0x100`), XDRC-20 tokens.

## 2. Configuration

Vite env at build time (all public): `VITE_RPC_URL`, `VITE_BUNDLER_URL`, `VITE_CHAIN_ID` (default 7117),
`VITE_API_URL` (explorer API base, optional), `VITE_FAUCET_URL`, `VITE_EXPLORER_URL`, `VITE_PAYMASTER_SPONSOR`
(XdevsPaymaster sponsor paying the activation op), `VITE_KNOWN_TOKENS` (comma list), `VITE_DEV_TOOLS`
(virtual passkey + raw-key panels; on automatically under `vite dev`, must stay off on public builds).

Persistence: `localStorage["xdevs-wallet:v1"]` holds account records (address, label, kind, passkey credential id +
public key, init params and salt of not-yet-deployed accounts) and a per-account recipient history. Passkey accounts
store **no secret**. Dev-only records (virtual passkey JWK, raw key) do, and are labelled as such.

## 3. Passkey account (no seed phrase)

1. `navigator.credentials.create` with `pubKeyCredParams: [ES256 (-7)]`, `userVerification: "required"`,
   `residentKey: "preferred"`, `attestation: "none"`. The public key comes from
   `AuthenticatorAttestationResponse.getPublicKey()` (SPKI → last 64 bytes = x‖y); the algorithm must be -7.
2. `InitParams` = `{signers: [Passkey(x, y)], guardians, guardianThreshold, recoveryDelay: 0 (48 h), policy, whitelist: [],
   tokenLimits: []}`. Default policy when the user keeps the vault on: `recommendedPolicy` = enabled, block unlimited
   approvals, no instant ERC-1271, 24 h delay, 10 tXDS per transfer, 50 tXDS per day, 1 tXDS daily gas budget (all
   editable at sign-up). Salt: 64 random bits.
3. Address: `factory.getAddressWithConfig(params, salt)` (`eth_call`; the address commits to the full config).
4. The address can receive tXDS right away. "Activate" sends the first UserOperation with
   `factory = 0x…1006`, `factoryData = createAccountWithConfig(params, salt)`, empty `callData`, and — when
   `VITE_PAYMASTER_SPONSOR` is set — `paymaster = 0x…1007`, `paymasterData = sponsor` (open / allowlist mode).
   Without a sponsor the account must be funded first (faucet link `VITE_FAUCET_URL?address=…`).

### 3.1 Owner signature encodings (exactly what `SignatureVerifier` expects)

| Signer | `userOp.signature` |
|---|---|
| Passkey | `0x02 ‖ abi.encode(bytes32 x, bytes32 y, (bytes32 r, bytes32 s, uint256 challengeIndex, uint256 typeIndex, bytes authenticatorData, string clientDataJSON))` |
| secp256k1 (dev key / 7702 EOA key) | raw 65-byte `r‖s‖v` over the 32-byte hash (no EIP-191 prefix) |

Passkey details (`src/lib/webauthn.ts`): the WebAuthn challenge is the 32-byte userOpHash (or the ERC-1271
`replaySafeHash`); the DER signature is parsed and normalized to **low-s** (`s > n/2 → n − s`, the account rejects
high-s); `typeIndex` / `challengeIndex` are **UTF-8 byte offsets** of `"type":"webauthn.get"` and
`"challenge":"<base64url(hash)>"` in `clientDataJSON` (not UTF-16 string indexes). The account checks UP and UV flags,
so creation and assertion both require user verification. Gas estimation uses a dummy signature of the same shape
(`dummyPasskeySignature`), so the user is prompted once per operation.

### 3.2 UserOperation flow (`src/lib/send.ts`)

nonce (`EntryPoint.getNonce(sender, 0)`, 0 for an undeployed account) → fees (`maxFee = 2 × baseFee + tip`) →
`eth_estimateUserOperationGas` with the dummy signature → userOpHash computed locally (EIP-712, domain `ERC4337`/`1`,
v0.9 paymaster-signature rule; equality with `EntryPoint.getUserOpHash` is tested) → one signature → 
`eth_sendUserOperation` → poll `eth_getUserOperationReceipt`. `VaultActionQueued` logs of the sender in the receipt tell
the UI that the op was queued and with which action id. Call data: `execute(target, value, data)` for one call,
`executeBatch(Call[])` for several, or an account function directly (e.g. `setVaultPolicy`, `cancel`): the EntryPoint is
an owner authority after validation.

## 4. Send

1. Recipient: `0x…` (checksummed) or an XNS name. Names go through `normalizeXnsName` (SDK; invalid input such as
   Cyrillic look-alikes is rejected locally), then `xdevs_resolveName`, falling back to
   `XNSRegistry.resolveAddr(namehash)` when the node lacks the method. Expired names do not resolve.
2. Recipient checks: `xdevs_checkRecipient` when available, else local — first-time recipient and look-alike
   (`findLookAlikeAddresses`, same first/last 4 hex chars as a past recipient) from the wallet's own history, code
   check, `XNSRegistry.primaryName` (forward-verified).
3. Vault outcome (§5) and pre-sign preview (§6) are shown before signing. High-severity warnings need an explicit
   acknowledgement; a "rejected" vault outcome disables signing.
4. Modes: native tXDS, ERC-20 transfer (known tokens), raw contract call (what a dApp would ask to sign).

## 5. Guardian Vault in the wallet

- **Will it be queued?** `checkVaultOutcome` runs the account's own dispatcher in an `eth_call` **from the EntryPoint**
  (owner authority): the call returns the queued action id (`> 0` queued, `0` instant) or reverts
  (`UnlimitedApprovalBlocked`, `AccountIsFrozen`). This is exact for deployed accounts. A local mirror
  (`explainVault`: self-call instant list, approvals, transfers, whitelist, threshold / daily limit; per-token limits
  not modelled) supplies the human-readable reason and the answer for undeployed accounts.
- **Policy**: view, and change through `setVaultPolicy` (delayed while the vault is on — the UI says so; disabling the
  vault is delayed too). Whitelist add / remove (`setWhitelisted`, delayed).
- **Queue**: explorer API `/api/v1/addresses/:a/vault-queue` when configured, else `lastActionId()` + `getAction(id)` for
  the last 30 ids, with status (queued / ready / cancelled / executed / expired / invalidated), countdown to
  `executeAfter`, guardian cancel votes (`guardianVotes(2, id)`), owner **Cancel** and **Execute** (when ready). The app
  shell polls the queue every 5 s and shows an alert banner while any action is open ("Not you? Review and cancel").
- **Guardians / freeze / recovery**: guardians and quorum (`getGuardians`), change via `setGuardians` (delayed), freeze
  state (`freezeState`: frozenUntil, duration, anti-hostage cooldown), social-recovery status from the account's
  `RecoveryApproved / Ready / Executed / Cancelled` events (last ~9,000 blocks). While frozen, owner actions are
  disabled and the UI explains that only the guardian quorum can act.
- **Guardian tools** tab: a guardian connects MetaMask (EIP-1193) or a dev key, loads the protected account, and votes
  `cancel(id)`, `freeze()`, `unfreeze()`, `approveRecovery(newSigners)` / `executeRecovery(newSigners)`. Every vote is a
  normal transaction from the guardian; the effect happens at the quorum.

## 6. Pre-sign preview ("advice, not fact")

`previewTransaction` (`src/lib/preview.ts`):

1. `xdevs_simulateTransaction` (SDK) when the node supports it; local recipient warnings are merged in.
2. Otherwise (`-32601` / `XdevsMethodNotAvailableError`) a **local preview**: `eth_call` from the account (success /
   revert reason) and `eth_estimateGas`, plus calldata decoding: ERC-20 `approve` / `increaseAllowance`, XDRC-20
   `approve(spender, value, expiry)` (and the token's `defaultApprovalDuration` for legacy approves), Permit2Expiring
   `approve(token, spender, amount, expiration)`, `setApprovalForAll`, `transfer`, `transferFrom` /
   `safeTransferFrom`, `transferOwnership`, `revokeAll`, native value.
3. Warnings use the SDK catalogue codes: `UNLIMITED_APPROVAL` (high; amount ≥ 2¹⁶⁰−1, the account's own threshold, or
   2²⁵⁶−1), `SET_APPROVAL_FOR_ALL` (high), `OWNERSHIP_TRANSFER` (high), `LOOKALIKE_RECIPIENT` (high),
   `FIRST_TIME_RECIPIENT` (low), `HIGH_VALUE_TRANSFER` (medium, > 50 % of the balance), `UNVERIFIED_CONTRACT` (low,
   undecodable call).
4. Every preview carries a source badge (node simulation vs local) and the disclaimer "Advice, not fact: …".

## 7. Approvals manager

Sources in order: explorer API `GET /api/v1/addresses/:a/approvals` (`VITE_API_URL`), node `xdevs_getApprovals`,
else a local scan: `Approval(owner, spender)` logs of `VITE_KNOWN_TOKENS` and `Approval` / `Permit` logs of
Permit2Expiring over the last ~9,000 blocks (the node caps `eth_getLogs` at 10,000), then live `allowance()` /
`allowanceExpiry()` / Permit2 `allowance()` reads. XDRC-20 is detected with `approvalEpoch(owner)`.

Revoke one: `approve(spender, 0)` or Permit2 `approve(token, spender, 0, 0)` — revocations are always instant under the
vault. Revoke all: with the vault **on**, one revocation per pair (instant); with the vault off, XDRC-20 `revokeAll()`
per token and Permit2Expiring `revokeAll()` (O(1), also kills signed-but-unsubmitted permits). See §10 finding 1.

The tab also has a "grant an approval" builder so users (and the exit-criteria demo) can see the preview flag an
unlimited approval and the vault reject it before anything is signed.

## 8. EIP-7702 and the XIP-0001 EOA lock

- **MetaMask (read-only)**: connect via EIP-1193, read the code (`0xef0100 ‖ 0x…1005` = delegated), vault state and the
  lock (`xdevs_getEoaLock(address, "latest")` → `{active, status, word, delegatedTo}`; on non-xDEVS nodes the raw word
  at slot `0xfd81…41ce`, shown as "enforcement unknown"). The page explains the upgrade and the lock (LOCKED: only
  value-0 self-calls; SEALED while frozen or after a recovery revoked the key) and that MetaMask does not sign 7702
  authorizations to arbitrary contracts.
- **Dev path (raw key, dev chains only)**: one type-4 transaction from the EOA to itself with
  `authorizationList = [auth(0x…1005)]` and `data = initialize(params)` (EOA key as ECDSA signer, vault on, optional
  guardians); then `enableEoaLock()` as a self-call (instant; requires the vault). Self-calls use
  `maxFee = 2 × baseFee`, tip ≤ baseFee and gas ≤ 1,000,000 so they stay valid while LOCKED. The upgraded account can be
  used in the wallet (UserOps signed by the EOA key through the bundler — they never come from the EOA itself).

## 9. Dev bundler (`apps/bundler`, DEV ONLY)

`eth_sendUserOperation`, `eth_estimateUserOperationGas`, `eth_getUserOperationReceipt`, `eth_getUserOperationByHash`,
`eth_supportedEntryPoints`, `eth_chainId`. One op per bundle, bundles strictly sequential, a single operator key
(`BUNDLER_PRIVATE_KEY`), validation by `eth_call` of `handleOps([op])` (EntryPoint `FailedOp` mapped to ERC-7769
codes), no ERC-7562 rules, no mempool, no reputation. Gas estimation without state overrides (the xDEVS node rejects
them): a helper (`XdevsBundlerGasMeter`) executed inside the EntryPoint via `delegateAndRevert` measures deployment,
validation, paymaster validation / postOp and execution, and reverts with the numbers. Details in
`apps/bundler/README.md`.

`npm run devnet` starts the real node with `deploy/configs/dev-genesis-with-predeploys.json` (or anvil
`--hardfork osaka` with the same alloc), configures a paymaster sponsor, stakes the paymaster and deploys an XDRC-20
test token.

## 10. Findings in the chain contracts / node that affect wallets (reported, not changed)

1. **One-shot revocations are queued by the vault.** `CallClassifier` recognises `approve(…, 0)`,
   `setApprovalForAll(…, false)` and Permit2 `approve(…, 0, …)` as revocations, but not XDRC-20 `revokeAll()`,
   Permit2Expiring `revokeAll()` or `lockdown(...)`: they are "generic calls to a non-whitelisted contract" and wait the
   full vault delay. In an emergency ("I signed something bad, kill every approval now") the O(1) path is the slow one.
   The wallet uses per-pair `approve(…, 0)` when the vault is on. Suggested fix: classify these three selectors as
   revocations (they can only reduce allowances).
2. **No sponsor in genesis.** `XdevsPaymaster` is predeployed but unstaked and without any sponsor, so passkey sign-up
   with sponsored activation needs an operator step after genesis (`configure` + `deposit` + `addStake`, done by
   `npm run devnet` on dev chains). Bundlers that enforce ERC-7562 also require the stake.
3. **No enumeration of recovery proposals.** Proposals are keyed by the exact new signer set; there is no view listing
   pending ones, so wallets must scan `RecoveryApproved` / `RecoveryReady` logs (bounded by the node's 10,000-block
   `eth_getLogs` window) or rely on the explorer API.
4. **ERC-1271 with the vault on** accepts only hashes pre-approved through the delayed `approveHash` (by design, against
   off-chain permit bypasses). The wallet therefore does not offer dApp message signing (`personal_sign`,
   `eth_signTypedData`, Permit2 permits) for vault-enabled accounts; it would need an "approve this hash, wait the delay"
   flow.
5. **Passkey restore on a new device** needs the passkey public key. The account exposes `getSigners()`, so a wallet
   could restore from the account address with a discoverable credential, but the reference wallet keeps the key in
   local storage only (export / restore UI not built). Recovery by guardians is the supported path.
6. **Node RPC gaps (Phase 4 items not yet in the node):** `xdevs_simulateTransaction`, `xdevs_checkRecipient`,
   `xdevs_getApprovals`, `xdevs_getVaultQueue`, `xdevs_resolveName`, `xdevs_reverseName` answer `-32601`; the wallet
   falls back to local logic for each. `eth_call` state overrides are rejected (hence the gas-meter design in §9).
   `eth_subscribe` is not used (HTTP polling only).
7. **XIP-0001 fee rule vs. wallets.** A LOCKED EOA's self-call must have `maxFeePerGas ≤ 2 × baseFee`. Wallets that pad
   fees by default (MetaMask often does, especially at the 0.1 gwei base-fee floor where the tip alone can exceed the
   base fee) will see `-32003 XIP0001_*` errors on self-calls; wallets need to special-case locked accounts.
8. **ABI ergonomics.** `InitParams` (tuple with nested tuple arrays) is too deep for viem's type-level human-readable
   ABI parser; the wallet declares it as a JSON ABI fragment. Not a bug, but worth shipping JSON ABIs with the SDK
   (`sdk.md` §5 "contract ABIs").

## 11. Security notes

- No seed phrase; passkey private keys never leave the authenticator. Origin / rpId / counter are not verified
  on-chain (smart-accounts.md §12.8): a passkey is bound to the wallet's domain by the browser, so a different wallet
  origin cannot use it.
- The wallet never signs blind: every send shows recipient checks, the vault outcome and the preview, labelled advice.
- Dev-only features are gated by `VITE_DEV_TOOLS`; the virtual passkey and pasted keys are hot keys in browser storage.
- The dev bundler must not be exposed publicly.

## 12. Tests

| Suite | What it proves |
|---|---|
| `apps/bundler/test/bundler.test.ts` | local userOpHash = `EntryPoint.getUserOpHash` (incl. v0.9 paymaster signature); counterfactual deploy + first op; paymaster-sponsored deploy of an account holding 0 tXDS; AA24 → `-32507`; wrong EntryPoint; execution revert in estimation (`-32521`); receipt / by-hash lookup from logs |
| `apps/wallet/test/unit.test.ts` | packing round-trip, paymasterDataKeccak, dummy signature shapes, vault explanation, revoke builders, wallet / bundler `userop.ts` in sync |
| `apps/wallet/test/webauthn.test.ts` | RFC 6979 P-256 vector and software-passkey WebAuthn assertions verified by the chain's `P256VERIFY` (`0x100`); field byte offsets; `XdevsSmartAccount.isValidSignature` accepts the encoding and rejects a wrong challenge |
| `apps/wallet/test/e2e.test.ts` | Phase 4 exit criteria: (a) passkey-only account → sponsored deployment → faucet funds → instant small payment; (b) stolen key: large transfer queued, visible in the queue, one guardian vote not enough, 2-of-3 cancels, cancelled action cannot execute; vault-off attempt queued and cancelled too; (c) unlimited approval flagged (high, "advice, not fact") and rejected by the vault policy; look-alike recipients; approvals list + revoke-all; EIP-7702 upgrade + EOA lock (on the xDEVS node a raw transfer from the locked key is refused) |

All chain tests run against the real node (`xdevs run --dev` with the predeploy genesis) when its binary is found, and
against `anvil --hardfork osaka` otherwise (`XDEVS_TEST_CHAIN`).
