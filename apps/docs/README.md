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

## Hosting: Cloudflare Pages

The site is published at **https://xdevschain.xdevs.in** from the Cloudflare Pages project `xdevs-chain-docs` (Direct Upload; the `xdevs.in` DNS zone is on Cloudflare).

`.github/workflows/docs.yml` builds the site on every pull request and push to `main` that touches `apps/docs/**` (the build fails on dead links), and on `main` deploys it with `wrangler pages deploy`. It needs the repository secret `CLOUDFLARE_API_TOKEN` (Account › Cloudflare Pages: Edit) and the variable `CLOUDFLARE_ACCOUNT_ID`. The custom domain is attached to the Pages project and served through a proxied `CNAME xdevschain → xdevs-chain-docs.pages.dev`; Cloudflare issues the certificate. No `CNAME` file is needed.
