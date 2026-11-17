# xDEVS Explorer

Block explorer for **xDEVS Chain Enclave**, the public testnet (chain id 717171, coin EnclaveXDS / `tXDS`), at
[explorer.xdevschain.tech](https://explorer.xdevschain.tech). Next.js (App Router) static export for Cloudflare Pages,
the shared brand kit [`@xdevs/ui`](../../packages/ui) (same look as xdevschain.tech), viem and [`@xdevs/sdk`](../../sdk/ts).

## Pages

| Path | What it shows |
|---|---|
| `/` | Live chain head with the finality badge, blocks and transactions streaming in (WebSocket `newHeads`, polling fallback), measured stats from the last 30 blocks (height, average block time, throughput), active validators, transactions per block. |
| `/blocks`, `/txs` | Live lists; older pages from the indexer when it is up. |
| `/block/:number` | Header fields, fee split, transactions and the **commit certificate**: which validators precommitted, weighted by voting power against the 2/3 line, with each signature (`xdevs_getCommitCertificate`). |
| `/tx/:hash` | Status, value, gas, fee split (burn / Treasury / tip), **decoded input** for the predeploys (Staking, Governance, XNS, WXDS, Permit2Expiring), XDRC-20 / ERC-20 and the Guardian Vault, transfers, approvals with expiry, EIP-7702 authorizations, decoded logs, and a **Guardian Vault panel** when the tx queued, cancelled or executed a vault action. |
| `/address/:address` | Balance in tXDS, nonce, smart-account and **EOA lock** badges (`xdevs_getEoaLock`, XIP-0001), primary `.xdevs` name (XNS reverse record), guardians; tabs for transactions, transfers (spam hidden by default), live approvals with revoke hints, vault queue. Look-alike (address poisoning) warning from this browser's history. |
| `/token/:address` | XDRC-20 / ERC-20 facts (supply, decimals, default and max approval duration), recent transfers. |
| `/validators` | Validator set from the Staking predeploy (`0x…1000`) or `xdevs_validators`: status, stake, voting power, commission, uptime, epoch. |
| `/name/:name` | `.xdevs` name: resolution, owner, expiry, grace period. |
| `/search?q=` | Block number, tx hash, address or `.xdevs` name go straight to their page; other text asks the indexer. |

## Data sources

1. **Indexer API** (`apps/indexer`, [spec](../../docs/specs/explorer-api.md)) for lists, history, labels, spam flags,
   approvals and vault queues.
2. **Node RPC** for everything live: the head (WebSocket `newHeads`, else `eth_blockNumber` polling), commit certificates,
   validators, the EOA lock, XNS reverse names, token metadata. When the API is down, block / tx / address / token pages
   fall back to RPC-only views (labelled "Read directly from the node").
3. When **neither** answers, every page shows a calm "Network not reachable yet — Enclave testnet launching soon" state
   with both endpoint checks and a retry. Re-probes back off (15 s → 2 min), so an offline network never floods the console.

## Configuration

Build-time variables (inlined, public). Defaults are the Enclave endpoints from `@xdevs/sdk`:

| Variable | Default | Meaning |
|---|---|---|
| `NEXT_PUBLIC_RPC_URL` | `https://rpc.xdevschain.tech` | Node JSON-RPC (HTTP). |
| `NEXT_PUBLIC_WS_URL` | `wss://ws.xdevschain.tech` | Node WebSocket for `newHeads`. |
| `NEXT_PUBLIC_API_URL` | `https://explorer-api.xdevschain.tech` | Explorer API origin (no `/api/v1`). |
| `NEXT_PUBLIC_CHAIN_ID` | `717171` | Expected chain id; a different id from the RPC shows a warning in the footer. |
| `NEXT_PUBLIC_NETWORK_NAME` | `xDEVS Chain Enclave` | Label. |
| `NEXT_PUBLIC_DEMO` | unset | `1` = demo mode: deterministic fixture chain (8 validators, XDRC-20 tokens, a smart account with a Guardian Vault queue, `.xdevs` names), no network calls, "Demo data" banner on every page. |

See `.env.example` (for example a local node + indexer on 8545 / 8080).

## Develop

From the repository root (npm workspaces; build the SDK once with `npm run build:sdk`):

```bash
npm run dev -w @xdevs/explorer                        # http://localhost:3000
NEXT_PUBLIC_DEMO=1 npm run dev -w @xdevs/explorer     # with demo data
npm run typecheck -w @xdevs/explorer
npm test -w @xdevs/explorer                           # node --test: formatting, look-alikes, routes/rewrites, decoding,
                                                      # stats, commit certificates, demo chain, data fallbacks, components
npm run build -w @xdevs/explorer                      # static export into apps/explorer/out
npm start -w @xdevs/explorer                          # serves out/ with the _redirects rewrites (scripts/serve.mjs)
```

The component tests compile `.tsx` with the SWC binary that Next.js ships (`test/jsx-hooks.mjs`); no extra dependency.

## Deploy (Cloudflare Pages, project `xdevs-explorer`)

- Build: `npm ci && npm run build:sdk && npm run build -w @xdevs/explorer`, output `apps/explorer/out`, `NODE_VERSION=24`.
- **Dynamic routes** cannot be pre-rendered in a static export. Each one is exported once with the placeholder param
  `_` (`generateStaticParams() → [{ … : '_' }]`), and `public/_redirects` serves that page for every value with a 200
  rewrite (the URL stays):

  ```
  /block/*    /block/_    200
  /tx/*       /tx/_       200
  /address/*  /address/_  200
  /token/*    /token/_    200
  /name/*     /name/_     200
  ```

  The page reads the real value from `window.location` after hydration. Links to these pages are plain `<a>` (a
  document load, no RSC fetch for a payload that does not exist); static pages use `next/link`.
- `public/_headers`: CSP (`script-src 'self' 'unsafe-inline'`: a Next static export inlines its RSC payload), no framing,
  immutable caching for `/_next/static/*`. If the API or RPC is served over plain `http:`, extend `connect-src`.
- Custom domain `explorer.xdevschain.tech`; set the indexer's `CORS_ORIGINS=https://explorer.xdevschain.tech`.

## Design and accessibility

Dark brand from `@xdevs/ui` (platinum on near-black, one teal accent for "final / live / links"), tabular numerals,
monospace hashes with copy buttons, skeleton loading, responsive down to 360 px. Skip link and landmarks from the
AppShell, one `h1` per page, table captions, text on every coloured badge, keyboard-reachable everything (`/` focuses
the search), `role="alert"` for look-alike and freeze warnings, and `prefers-reduced-motion` turns off the ticker, row
and pulse animations.
