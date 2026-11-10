# DNS records (zone `xdevs.in`)

Proxied (orange cloud) records get Cloudflare TLS, DDoS protection, the WAF and rate limits.
Raw TCP/UDP P2P **cannot** be proxied on the free plan (Spectrum is paid), so P2P names are
**DNS only** (grey cloud) and point straight at the VM's public IP. Those IPs are public anyway:
every peer learns them.

Values in angle brackets are placeholders. Never commit real IPs to the repo.

| Name | Type | Content | Proxy | Created by | Purpose |
|---|---|---|---|---|---|
| `xdevschain` | CNAME | `<pages-project>.pages.dev` | Proxied | Pages -> Custom domains | Website and docs |
| `xdevschain-rpc` | CNAME | `<tunnel-id>.cfargotunnel.com` | Proxied | `cloudflared tunnel route dns` | Public JSON-RPC (HTTPS) |
| `xdevschain-ws` | CNAME | `<tunnel-id>.cfargotunnel.com` | Proxied | `cloudflared tunnel route dns` | Public WebSocket (`wss://`) |
| `xdevschain-explorer` | CNAME | `<explorer-host>` (e.g. `<project>.pages.dev`, or `cname.vercel-dns.com`) | Proxied for Pages; **DNS only** for Vercel (Vercel issues its own certificate) | Hosting provider | Block explorer UI |
| `xdevschain-faucet` | (Workers custom domain) | `xdevs-faucet` Worker | Proxied | `wrangler` / Workers -> Domains | tXDS faucet |
| `xdevschain-boot1` | A | `<oracle-bootnode-public-ip>` | **DNS only** | Manual | Bootnode P2P (30333/30334) |
| `xdevschain-boot2` | A | `<gcp-bootnode-public-ip>` | **DNS only** | Manual | Second bootnode on another provider |
| `xdevschain-val1` .. `xdevschain-val4` | A | `<validator-N-public-ip>` | **DNS only** | Manual | P2P dial names for nodes outside the Oracle VCN (`p2p_host` in the inventory) |
| `xdevschain-status` (optional) | CNAME | `stats.uptimerobot.com` | **DNS only** | Manual | UptimeRobot status page, if your plan allows custom domains |

Notes:

- **TTL**: Auto for proxied records. Use 300 s for the DNS-only P2P records, so that an IP
  change (for example after an Oracle instance is re-created) spreads quickly. Nodes dial
  `/dns4/<name>/...`, so an IP change is one DNS edit.
- **CAA** (optional hardening): `xdevs.in CAA 0 issue "letsencrypt.org"`,
  `0 issue "pki.goog"`, `0 issue "digicert.com"` (the CAs Cloudflare uses). Add the CA of any
  non-Cloudflare host (Vercel uses Let's Encrypt).
- **DNSSEC**: enable it in the dashboard (DNS -> Settings). It is free.
- Do **not** add a wildcard `*.xdevs.in` record. Unknown names should not resolve.
- Validators never get a proxied RPC name. Public RPC is only `xdevschain-rpc` / `xdevschain-ws`.
