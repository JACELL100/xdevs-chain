# xDEVS Wallet (wallet.xdevschain.tech)

Next.js static export (client-side app) built on the shared brand kit `@xdevs/ui`: a passkey smart account with no
seed phrase, the Guardian Vault, guardians with freeze and recovery, expiring approvals, `.xdevs` names, and the
EIP-7702 / XIP-0001 EOA lock for MetaMask addresses. Spec: [`docs/specs/wallet.md`](../../docs/specs/wallet.md).
Testnet: xDEVS Chain **Enclave** (chain 717171, EnclaveXDS / tXDS, no monetary value).

## Screens

| Route | What |
|---|---|
| `/` | Dashboard (balance, quick actions, assets, activity, protection summary, vault alert, activation) or onboarding |
| `/welcome` | Create a wallet with a passkey (vault limits, optional guardians), sign in with a passkey, watch an address |
| `/send` | tXDS / token / contract call; address or `.xdevs` name; look-alike and first-time checks; simulation ("advice, not fact"); Guardian Vault outcome ("will wait N hours"); one passkey confirmation |
| `/receive` | Address grouped for checking, name, QR code (local encoder, `src/lib/qr.ts`), faucet link |
| `/vault` | Queued actions with countdown, cancel (confirm dialog) / execute, history, policy, trusted addresses |
| `/guardians` | Guardians and quorum (add / remove), freeze and recovery state, "act as a guardian" tools (browser wallet) |
| `/approvals` | Live approvals with expiry, revoke / revoke all, approval checker |
| `/names` | Claim a look-alike-proof `.xdevs` name and set it as primary |
| `/eoa-lock` | Connect MetaMask, see delegation / vault / lock state, turn on the XIP-0001 lock (dev raw-key upgrade panel) |
| `/settings` | Endpoints with live status, accounts on this device, export of PUBLIC account info, demo switch |

When the RPC does not answer, every screen shows a "Enclave testnet is launching soon" state with "Preview the
wallet with demo data" (session-only demo).

## Configuration (all public, inlined at build time; defaults = Enclave)

`NEXT_PUBLIC_RPC_URL`, `NEXT_PUBLIC_BUNDLER_URL`, `NEXT_PUBLIC_CHAIN_ID`, `NEXT_PUBLIC_CHAIN_NAME`, `NEXT_PUBLIC_API_URL`
(explorer API, `off` = chain reads only), `NEXT_PUBLIC_EXPLORER_URL`, `NEXT_PUBLIC_FAUCET_URL`,
`NEXT_PUBLIC_PAYMASTER_SPONSOR`, `NEXT_PUBLIC_KNOWN_TOKENS`, `NEXT_PUBLIC_RP_ID` (WebAuthn RP ID; default = page host,
i.e. `wallet.xdevschain.tech`, `localhost` in dev; never change it on a live deployment), `NEXT_PUBLIC_DEMO=1` (demo
build: fixture account and data, every screen labelled "Demo"), `NEXT_PUBLIC_DEV_TOOLS` (virtual passkey and raw-key
panel; on under `next dev`, never on a public build). See `.env.example`.

## Develop, test, build

```bash
npm run build:sdk                      # from the repo root, once
npm run dev -w @xdevs/wallet           # http://localhost:3000 (WebAuthn works on localhost)
npm run typecheck -w @xdevs/wallet
npm test -w @xdevs/wallet              # chain tests start anvil (or the xDEVS node) themselves
npm run build -w @xdevs/wallet         # out/ ; postbuild writes out/_headers (per-page CSP with script hashes)
```

Local chain: `cd apps/bundler && npm run devnet` and the bundler (see its README; on Enclave: `https://bundler.xdevschain.tech`), then `.env.local` with the
local values from `.env.example`.

Code layout: `src/lib` (framework-free logic shared with the tests: WebAuthn encoding, UserOperations, vault,
preview, recipients, approvals, 7702, QR, formatting), `src/data` (`live.ts` = chain + bundler + explorer API,
`demo.ts` = fixtures; both implement `Backend`), `src/wallet` (provider, gate, hooks), `src/components`, `src/views`,
`src/app` (routes).

Tests: `unit` (UserOp packing, vault rules, revoke builders, bundler copy in sync), `webauthn` (P-256 encoding verified
by the chain's `P256VERIFY` and the account's ERC-1271), `e2e` (Phase 4 exit criteria on a real chain + the app's live
data layer and passkey sign-in matching), `app` (formatting, strict amounts, sign-in verification, demo backend
behaviour), `qr` (Reed-Solomon / format / version vectors), `csp` (headers generator).

## Security notes

- No seed phrase; the passkey's private key never leaves the authenticator. Only public data is stored locally.
- Every send shows recipient checks (look-alike / first-time / verified name), the vault outcome and a simulation;
  high-severity warnings and look-alike recipients need an explicit acknowledgement; "rejected" disables signing.
- Strict amount parsing (no guessing of `1,5`), amounts truncated never rounded up, addresses shown in 4-char groups
  with the first and last group highlighted.
- No analytics, no third-party requests, no `eval`; errors never include key material. CSP: inline scripts allowed
  by hash per page, `connect-src` limited to the configured RPC / bundler / explorer API.
