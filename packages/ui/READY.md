# Web platform foundation: READY

The npm workspace, `@xdevs/ui` and the Enclave chain in `@xdevs/sdk` are in place. Verified: root `npm install`
(one node_modules), SDK build + typecheck + 45 tests, UI typecheck + 7 tests, `tsc --noEmit` of `apps/status`
(AppShell + next/link types OK). **NOT yet verified: a `next build` of an app**, because the disk dropped below the
500 MB safety line (about 250 MB free at the time of writing). Check `df -h /c` before any `next build` and do not
build below ~600 MB free; the first agent to run `next build` on `apps/status` or `apps/faucet-web` please tell the
coordinator whether it worked.

## Workspace layout

```
package.json            "xdevs-web" (private), npm workspaces, ONE package-lock.json + node_modules at the root
sdk/ts                  @xdevs/sdk   (prebuilt to dist/: run `npm run build:sdk` before building apps)
packages/ui             @xdevs/ui    (TypeScript source, consumed via transpilePackages)
apps/explorer           @xdevs/explorer   -> explorer.xdevschain.tech   (Pages project xdevs-explorer)
apps/wallet             @xdevs/wallet     -> wallet.xdevschain.tech     (Pages project xdevs-wallet)
apps/faucet-web         @xdevs/faucet-web -> faucet.xdevschain.tech     (Pages project xdevs-faucet)
apps/status             @xdevs/status     -> status.xdevschain.tech     (Pages project xdevs-status)
```

Standalone (NOT workspace members, own lockfiles, untouched): `apps/site`, `apps/indexer`, `apps/bundler`,
`apps/faucet` (the Cloudflare Worker). `npm ci` inside them still works on its own.

Pinned versions (same as apps/site): next 16.4.6, react / react-dom 19.3.1, viem 2.57.17, typescript 5.9.3,
@types/react 19.3.0, @types/react-dom 19.3.0, @types/node 24.19.2. Do not add other versions of these.

Every app has the scripts `dev`, `build` (static export to `out/`), `typecheck`, `test`; run them with
`npm run <script> -w @xdevs/<app>` from the root, or `npm run <script>` inside the app folder.

### Notes for the explorer and wallet agents

- Their `package.json` was rewritten to the Next.js set (name kept; deps: @xdevs/sdk, @xdevs/ui, next, react,
  react-dom, viem; devDeps: types + typescript). `vite` is gone, and the old `package-lock.json`, `node_modules`
  and `dist` were deleted. **Source files were not touched**: `src/`, `index.html`, `vite.config.ts`,
  `tsconfig.json` and `test/` are still the Vite versions for you to port.
- The `test` scripts were kept as they were. The explorer's `test/register-tsx.mjs` loads esbuild from Vite,
  which is no longer installed: rewrite those tests to plain `.ts` (node strips types natively) or drop the hook.
- `apps/status/` is a working minimal Next app (layout + client Shell + page + next.config + tsconfig):
  copy its `next.config.ts`, `tsconfig.json`, `app/layout.tsx`, `app/Shell.tsx` and `.gitignore` as a template.
  `apps/faucet-web/` has the same starter. The faucet/status agent owns and replaces both starters.

## Adding a dependency (lock protocol applies)

Only from the repo root, only while holding the heavy lock, exact versions, no audit/fund noise:

```sh
L=C:/Users/hp/AppData/Local/Temp/claude/02452e894feb37b3/b4705712-160b-489d-8961-b50a7488f4b1/scratchpad/heavy.lock
until mkdir $L 2>/dev/null; do sleep 15; done
npm install <pkg>@<exact-version> -w apps/<x> --save-exact --no-audit --no-fund; rc=$?
rmdir $L; exit $rc
```

Never run `npm install` inside an app folder without `-w` from the root, never create a nested lockfile, and
keep deps minimal (disk is nearly full). Prefer no new deps at all: the kit, viem and the SDK cover most needs.

## Lock protocol (all agents)

Heavy commands (`npm install`/`npm ci`, `next build`, `next dev` for long, large test runs) must hold the lock:

```sh
L=C:/Users/hp/AppData/Local/Temp/claude/02452e894feb37b3/b4705712-160b-489d-8961-b50a7488f4b1/scratchpad/heavy.lock
until mkdir $L 2>/dev/null; do sleep 15; done
( cd apps/<x> && npm run build ); rc=$?
rmdir $L    # ALWAYS release, also on failure
```

Delete `.next/` after a local build if you do not need it (`rm -rf apps/<x>/.next`), keep `out/` only while checking.

## Using @xdevs/ui

### next.config.ts (template)

```ts
import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  output: 'export',          // static HTML in out/ for Cloudflare Pages
  trailingSlash: false,
  images: { unoptimized: true },
  reactStrictMode: true,
  poweredByHeader: false,
  transpilePackages: ['@xdevs/ui', '@xdevs/sdk'], // @xdevs/ui is TS source; the SDK is prebuilt dist/
}

export default nextConfig
```

`tsconfig.json`: copy `apps/status/tsconfig.json` (same as apps/site: `moduleResolution: "bundler"`, `jsx: "react-jsx"`,
`@/*` path alias, Next plugin).

### CSS (import once, in `app/layout.tsx`, before your own CSS)

```ts
import '@xdevs/ui/styles.css'   // = tokens.css + base.css + components.css, in that order
import './app.css'              // your app styles afterwards
```

Granular imports exist (`@xdevs/ui/tokens.css`, `@xdevs/ui/base.css`, `@xdevs/ui/components.css`), same order.
Components never import CSS themselves. Tokens are the site's `--xh-*` variables (`--xh-bg` #05060a canvas,
`--xh-text` platinum, `--xh-final` teal = the ONE accent, `--xh-alert`, plus app-only `--xh-warn`), base classes are the
site's `xh-*` (`xh-wrap`, `xh-btn`, `xh-chip`, `xh-label`, `xh-mono`, `xh-num`, `xh-sr`, …) and kit classes are `xu-*`
(`xu-page`, `xu-page-title`, `xu-page-sub`, `xu-grid` are handy page helpers).

### Fonts (Geist / Geist Mono, same as the site)

```tsx
import { Geist, Geist_Mono } from 'next/font/google'
const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })
// <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
```

`--xh-sans` / `--xh-mono` in tokens.css read those variables, with system fallbacks. `next build` downloads the font
files once (needs network, works in CI).

### AppShell

Put it in a small client wrapper so it gets the current path and next/link (see `apps/status/app/Shell.tsx`):

```tsx
'use client'
import { AppShell, type NavItem } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'

const NAV: NavItem[] = [
  { label: 'Blocks', href: '/blocks' },
  { label: 'Transactions', href: '/txs' },
  { label: 'Validators', href: '/validators' },
]

export default function Shell({ children }: { children: React.ReactNode }) {
  return (
    <AppShell appName="Explorer" app="explorer" nav={NAV} currentPath={usePathname() ?? '/'} linkComponent={Link}
      headerActions={/* optional: search box, connect button */ null}>
      {children}
    </AppShell>
  )
}
```

Props: `appName` (string, shown after the mark), `app` ('explorer' | 'wallet' | 'faucet' | 'status': marks the
footer cross-link), `nav` (NavItem[]: `{ label, href, match?, external? }`; "/" matches exactly, others by prefix),
`currentPath`, `linkComponent` (next/link `Link`), `homeHref` ('/'), `network` (NetworkBadge props, default probes the
Enclave RPC every 60 s; `false` hides it), `headerActions`, `banner`, `footerNote`, `children`. It renders a skip link,
the sticky header, `<main id="main">` and the footer ("Built by THE xDEVS" -> https://xdevs.in, cross-links to
xdevschain.tech, Explorer, Wallet, Faucet, Status, Docs, GitHub, and the "EnclaveXDS has no monetary value" line).
Wrap page content in `<div className="xu-page">` for the standard width and padding.

### Components (all from `@xdevs/ui`)

| Component | Notes |
|---|---|
| `Logo`, `Mark` | Inline SVG mark (+ wordmark). `variant`: metal (default) / mono / solid; `< 24px` switches to solid |
| `Button` | `variant` primary / ghost / quiet / danger, `size` sm / md / lg, `href` renders `<a>` (`external`, `linkComponent`), `icon` (`ArrowIcon`, `ExternalIcon`) |
| `Card` | `title`, `actions`, `flush` (no padding, e.g. around a Table) |
| `Badge` | `tone` neutral / final / warn / alert / dim, `dot` |
| `Stat` | `label`, `value`, `hint`, `loading` |
| `CopyButton` | client; `value`, optional `label` |
| `Hash` | `value`, `href`, `head`/`tail` (6/6), `full`, `copy`, `linkComponent`; children replace the text |
| `AddressChip` | `address`, `label` (XNS name etc.), `href`, `head`/`tail` (4/4), `copy`, `linkComponent` |
| `Skeleton` | `width`, `height`, `radius`, `lines` |
| `EmptyState` | `title`, `description`, `icon`, `action` |
| `ErrorState` | `title`, `description`, `error` (shows `shortMessage`/`message`), `onRetry` (client only), `action` |
| `Tabs` | client; `items: { id, label, count?, content? }[]`, `value`/`defaultValue`/`onChange` |
| `Table` | `columns: { key, header, render?, align?, mono?, width? }[]`, `rows`, `rowKey`, `loading`, `empty`, `caption` |
| `NetworkBadge` | client; "Enclave testnet" + dot: live / degraded / down / checking / unknown; `status` fixed or `probe` |
| `AddToWallet` | client; EIP-3085 `wallet_addEthereumChain` for Enclave + switch, handles no wallet / rejection |

Helpers: `ENCLAVE` (chainId 717171, `chainIdHex` '0xaf173', names, rpc/ws/explorer/explorer-api/bundler URLs),
`XDEVS_LINKS` (site, docs, endpoints, explorer, wallet, faucet, status, github, xdevs), `enclaveAddChainParameters()`,
`probeRpc(url?, { expectedChainId, timeoutMs })`, `truncateHex`, `formatInt`, `cx`. Also importable without React from
`@xdevs/ui/network` and `@xdevs/ui/format` (handy in node tests).

SSR / static export: everything renders on the server. `CopyButton`, `Tabs`, `NetworkBadge`, `AddToWallet` and the
shell's mobile menu are `'use client'` islands. Functions (`onRetry`, `onChange`, `render`, `linkComponent`) can only be
passed from client components, or from server components into the non-client ones (`Table`, `Hash`, `Button`, `AppShell`).

### Brand assets

- In React use `<Mark />` / `<Logo />` (inline SVG, unique gradient ids via `useId`): no files needed.
- Favicon: copy `packages/ui/assets/favicon.svg` to `apps/<x>/public/favicon.svg` once and commit it (done for
  status and faucet-web), then `icons: { icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }] }` in metadata.
- All brand files (mark, logo lockups, ink/mono/solid variants, favicon-32, usage rules) are in `packages/ui/assets/`
  (copied from `apps/site/public/brand`). Copy what you need into your app's `public/`; for an OG image or to
  reference by URL, `https://xdevschain.tech/brand/<file>.svg` and `https://xdevschain.tech/og.png` are live.

## @xdevs/sdk chains

| Export | Chain id | Name | Coin | RPC |
|---|---|---|---|---|
| `xdevsEnclave` | **717171** (0xAF173) | xDEVS Chain Enclave | EnclaveXDS (tXDS) | https://rpc.xdevschain.tech, wss://ws.xdevschain.tech; explorer https://explorer.xdevschain.tech (apiUrl https://explorer-api.xdevschain.tech) |
| `xdevsDevnet` | 717170 | xDEVS Chain Devnet | Testnet XDS (tXDS) | none yet: `defineXdevsDevnet({ rpcUrl, wsUrl?, explorerUrl? })` |
| `xdevsLocal` | 7117 | xDEVS Chain Local | Testnet XDS (tXDS) | http://127.0.0.1:8545, ws://127.0.0.1:8545 (`xdevs run --dev`) |
| `xdevsTestnet` | | deprecated alias of `xdevsEnclave` | | |

Also: `XDEVS_ENCLAVE_CHAIN_ID`, `XDEVS_DEVNET_CHAIN_ID`, `XDEVS_LOCAL_CHAIN_ID`, `XDEVS_TESTNET_CHAIN_ID` (deprecated),
`XDEVS_ENCLAVE_ENDPOINTS` (rpc, ws, explorer, explorerApi, wallet, faucet, status, bundler, site), `enclaveXds`,
`testnetXds`, `xdevsChains`, `getAddChainParameters`, `addXdevsChainToWallet` (refuses chains with no RPC URL).

Breaking vs before: `xdevsDevnet` used to be the local 7117 chain (now `xdevsLocal`), and
`XDEVS_TESTNET_PLACEHOLDER_DOMAIN` / `XDEVS_TESTNET_IS_PLACEHOLDER` are gone. 717171 is free in the chainid.network
registry (checked 2026-11-17, 2888 chains listed; 717170 also free).

Build: `npm run build:sdk` (root) or `cd sdk/ts && npm run build`. Apps import the built `dist/`, so rebuild the SDK
after changing it.
