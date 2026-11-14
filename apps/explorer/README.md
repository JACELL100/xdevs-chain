# xDEVS Explorer (UI)

Static block explorer for xDEVS Chain: Vite + React + TypeScript, hand-written CSS (light and dark), no UI kit. It
reads the [explorer API](../../docs/specs/explorer-api.md) served by [`apps/indexer`](../indexer/README.md) and, for a
network check, the node's JSON-RPC.

## Pages

| Path | What it shows |
|---|---|
| `/` | Network stats (head, block time, totals, base fee, burned / Treasury fees), latest blocks and transactions (auto-refresh). |
| `/blocks`, `/txs` | Paginated lists. |
| `/block/:numberOrHash` | Block details, fee split, "finalized by N validators" from `xdevs_getCommitCertificate` when the node serves it. |
| `/tx/:hash` | Status, fee breakdown (burned / Treasury / tip), transfers with spam flags, approvals with **Unlimited** and expiry badges, EIP-7702 authorizations, decoded logs, input. |
| `/address/:address` | Balance, nonce, XNS name, labels, security badges (smart account, Guardian Vault, freeze, XIP-0001 EOA lock, recovery), tabs for transactions and transfers (**spam hidden by default**, toggle), live approvals with a **revoke hint** (ready-made calldata), and the vault queue. |
| `/name/:name` | `.xdevs` name: resolution, owner, expiry, grace period, primary name. |
| `/search?q=` | Block number, hash, address or name. |

**Look-alike warning** (plan §16.3): the explorer remembers the last 50 addresses you viewed or searched in this
browser (localStorage only, never sent anywhere). If you open an address whose first 4 and last 4 hex chars match a
different address from that history, a red banner explains address poisoning and shows both addresses. Addresses are
always shown with their first and last 4 characters emphasised, and short forms keep 6 + 6 characters so 4/4
look-alikes still differ visibly.

## Configuration

Build-time variables (Vite inlines them; they are public):

| Variable | Default | Meaning |
|---|---|---|
| `VITE_API_URL` | `http://127.0.0.1:8080` | Explorer API origin (no `/api/v1`). |
| `VITE_RPC_URL` | `http://127.0.0.1:8545` | Node JSON-RPC, used for the footer network check. |
| `VITE_CHAIN_ID` | `7117` | Expected chain id; a mismatch with the RPC or the API shows a warning. |
| `VITE_NETWORK_NAME` | derived | Footer label. |
| `VITE_NATIVE_SYMBOL` | `tXDS` | Native coin symbol. |

Copy `.env.example` to `.env.local` to change them locally.

## Run locally

```bash
# node + indexer/API first (see apps/indexer/README.md), e.g.
#   xdevs run --dev --genesis ../../deploy/configs/dev-genesis-with-predeploys.json --block-time 1s
#   (cd ../indexer && DATABASE_URL=pglite://./data npm start)
cd apps/explorer
npm ci
npm run dev        # http://localhost:5173
npm run build      # typecheck + static build into dist/
npm run preview    # serve dist/
npm test           # node --test: formatting, look-alike logic, router, server-rendered components
```

`@xdevs/sdk` is linked from `sdk/ts` (`file:../../sdk/ts`); build the SDK once (`cd sdk/ts && npm ci && npm run build`)
if its `dist/` is missing.

## Deploy to Cloudflare Pages

- Project root: `apps/explorer`. Build command: `npm ci && npm run build`. Output directory: `dist`. Node 24
  (`NODE_VERSION=24`). Because `@xdevs/sdk` is a `file:` link to `../../sdk/ts`, connect the whole repository and add
  `cd ../../sdk/ts && npm ci && npm run build && cd - && ` in front of the build command if `sdk/ts/dist` is not committed.
- Set `VITE_API_URL` (for example `https://explorer-api.xdevschain.tech`), `VITE_RPC_URL`
  (`https://rpc.xdevschain.tech`) and `VITE_CHAIN_ID` as build environment variables.
- Custom domain: `explorer.xdevschain.tech` (one subdomain level, covered by the free `*.xdevschain.tech` certificate).
- Deep links work without a redirect file: Pages serves `index.html` for unknown paths when the build has no
  `404.html`. `public/_headers` sets a strict CSP (`script-src 'self'`, `connect-src 'self' https: wss:`), no framing,
  and long caching for hashed assets. If the API or RPC is served over plain `http:`, extend `connect-src`.
- Then set the indexer's `CORS_ORIGINS=https://explorer.xdevschain.tech`.

## Accessibility and design notes

Semantic landmarks and a skip link, one `h1` per page, tables with headers and captions (stacked as labelled cards on
narrow screens), visible focus, status text on every coloured badge, `role="alert"` for the look-alike and freeze
warnings, `prefers-color-scheme` plus a manual Auto / Light / Dark toggle, and `prefers-reduced-motion`.
