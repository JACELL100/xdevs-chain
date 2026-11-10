# xDEVS reference wallet

Vite + React single-page app (no UI kit, hand-written CSS) implementing the Phase 4 user-safety layer (plan §16, §23,
§27): passkey smart accounts with no seed phrase, the Guardian Vault, pre-sign previews, an approvals manager, XNS
sending with poisoning checks, guardian tools, and the EIP-7702 / XIP-0001 EOA-lock path. Testnet reference: testnet
tokens have no value. Spec: [`docs/specs/wallet.md`](../../docs/specs/wallet.md).

## Run against a local dev chain

```bash
# 1. chain + paymaster sponsor + test token (terminal 1)
cd apps/bundler && npm install && npm run devnet          # prints the .env.local below
# 2. dev bundler (terminal 2) — the command is printed by step 1
BUNDLER_PRIVATE_KEY=0x5de4…365a RPC_URL=http://127.0.0.1:8545 npm start
# 3. wallet (terminal 3)
cd apps/wallet && npm install
cp .env.example .env.local    # then paste the values printed by step 1
npm run dev                    # http://localhost:5173 (WebAuthn needs localhost or HTTPS)
```

`npm run devnet` prefers the real node (`xdevs run --dev --genesis deploy/configs/dev-genesis-with-predeploys.json`,
binary via `XDEVS_BIN` or the cargo target dirs) and falls back to `anvil --hardfork osaka --chain-id 7117` with the
same genesis alloc. Against an already-running node: `npm run devnet -- --rpc http://127.0.0.1:8545`.

Configuration (`.env.local`, all public, baked into the bundle): `VITE_RPC_URL`, `VITE_BUNDLER_URL`, `VITE_CHAIN_ID`,
`VITE_API_URL` (explorer API, optional), `VITE_FAUCET_URL`, `VITE_EXPLORER_URL`, `VITE_PAYMASTER_SPONSOR`,
`VITE_KNOWN_TOKENS`, `VITE_DEV_TOOLS`.

## Dev-only parts (never on a public deployment)

- **Virtual passkey**: a WebCrypto P-256 key in `localStorage` that signs exactly like a passkey (for headless tests and
  machines without an authenticator). It is a hot key.
- **Raw-key EIP-7702 panel** and **dev-key guardian**: paste a private key. MetaMask cannot sign a 7702 authorization to
  `0x…1005` today, so the upgrade path needs the raw key on dev chains.
- **`apps/bundler`**: single-operator dev bundler, see its README.

`vite dev` enables these automatically; a production build only with `VITE_DEV_TOOLS=true`.

## Build and deploy

`npm run build` (typecheck + `vite build`) writes `dist/`, a static site (planned host: Cloudflare Pages). A public
deployment needs a production ERC-4337 bundler URL and must keep `VITE_DEV_TOOLS` unset.

## Tests

`npm test` (Node ≥ 22.18, runs TypeScript directly):

- `test/unit.test.ts` — UserOperation packing / hashing helpers, dummy signatures, vault explanation, revoke builders.
- `test/webauthn.test.ts` — WebAuthn signature encoding: RFC 6979 P-256 vector and software-passkey assertions verified
  by the chain's real `P256VERIFY` precompile (`0x100`), byte offsets, and acceptance by `XdevsSmartAccount`'s verifier
  (ERC-1271).
- `test/e2e.test.ts` — the Phase 4 exit criteria headlessly on a real chain with the dev bundler and the paymaster:
  passkey-only sign-up → sponsored deployment → faucet funds; stolen-key large transfer queued → visible in the queue →
  2-of-3 guardian cancel; unlimited approval flagged by the preview; look-alike recipients; approvals list + revoke;
  EIP-7702 upgrade + EOA lock (on the xDEVS node, a raw transfer from the locked key is refused with XIP-0001).

Chain selection: `XDEVS_TEST_CHAIN=xdevs|anvil|auto` (default auto = the xDEVS node if its binary is found). The e2e
test deploys the XDRC-20 test token from `contracts/out-wallet` (built with Foundry on first use).
