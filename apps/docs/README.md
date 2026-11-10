# xDEVS Chain docs site

Landing page and documentation for xDEVS Chain, served at **https://xdevschain.xdevs.in** (not live yet). Built with [VitePress](https://vitepress.dev) (the only dependency).

## Develop

```bash
cd apps/docs
npm ci
npm run docs:dev        # http://localhost:5173
npm run docs:build      # static site in .vitepress/dist (fails on dead links)
npm run docs:preview    # serve the built site
```

Layout: `index.md` is the landing page; `guide/`, `concepts/`, `developers/`, `operators/` and `network/` hold the docs; `.vitepress/config.mts` holds the nav and sidebar; `public/` is copied as-is. This README is excluded from the site (`srcExclude`).

Content rules: keep "built today" and "planned" clearly separated (use `<span class="status ok|next|planned">`), never publish performance numbers that haven't been measured, and link to the specs in `docs/specs/` on GitHub instead of copying them.

## Hosting: Cloudflare Pages (recommended) vs GitHub Pages

The `xdevs.in` DNS zone is on Cloudflare, like the planned RPC, faucet and explorer hosts.

**Cloudflare Pages (recommended).** Connect the repository in the Cloudflare dashboard with:

| Setting | Value |
|---|---|
| Root directory | `apps/docs` |
| Build command | `npm ci && npm run docs:build` |
| Output directory | `.vitepress/dist` |
| Environment | `NODE_VERSION=24` |

Then add the custom domain `xdevschain.xdevs.in` in the Pages project; Cloudflare creates the DNS record and certificate itself. It is free, gives preview URLs for pull requests, and keeps everything in one DNS zone. No `CNAME` file is needed.

**GitHub Pages (alternative).** `.github/workflows/docs.yml` builds the site on every pull request and push to `main` that touches `apps/docs/**`. Its deploy job runs only when started by hand (Actions → docs → Run workflow, with `deploy` checked), so nothing is published by accident. Before using it:

1. Settings → Pages → Source: **GitHub Actions**.
2. Add a DNS record in Cloudflare: `CNAME xdevschain → jacell100.github.io` (DNS only, not proxied, until GitHub has issued its certificate).
3. Settings → Pages → Custom domain: `xdevschain.xdevs.in`, then enable "Enforce HTTPS".

With the Actions-based deploy, GitHub ignores a `CNAME` file in the artifact and uses the domain set in Settings, so this site doesn't ship `public/CNAME`. Without a custom domain the site would live at `https://jacell100.github.io/xdevs-chain/`; build it with `DOCS_BASE=/xdevs-chain/` in that case.

Pick one host. Pointing the same domain at both doesn't work.
