# UptimeRobot monitors (free plan)

External checks from outside our infrastructure (plan §24.1, §24.7 "RPC down -> critical").
The free plan gives about 50 monitors at a 5-minute interval, plus a public status page.
Features change, so re-check the plan before relying on one (custom HTTP method/body and
custom status-page domains have moved between tiers before).

Alert contacts: the ops Discord webhook (UptimeRobot "Webhook" or "Discord" integration) and
the admin email. Leave the webhook URL in UptimeRobot only.

| # | Name | Type | Target | Expect | Alert |
|---|---|---|---|---|---|
| 1 | RPC JSON-RPC | Keyword (HTTP POST, if your plan allows custom method/body) | `https://xdevschain-rpc.xdevs.in` with header `Content-Type: application/json`, body `{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}` | keyword `"result":"0xaf173"` exists (717171) | **critical**, after 1 failure |
| 1b | RPC (fallback without POST) | Keyword (GET) | `https://xdevschain-faucet.xdevs.in/api/status` | keyword `"healthy":true` (the faucet calls the RPC; see [faucet runbook](../../docs/runbooks/faucet.md)) | critical. This one also fires when the faucet runs low, so read the status JSON |
| 2 | RPC TLS / Cloudflare | HTTP(s) | `https://xdevschain-rpc.xdevs.in/` (GET) | any response below 500. A GET without a body may return 4xx from the node; if the free plan has no "accepted status codes" setting, use a Port monitor on 443 instead | warning |
| 3 | WebSocket host | Port | `xdevschain-ws.xdevs.in:443` | open | warning. UptimeRobot cannot speak WebSocket; the full check is `check-health.sh` / Grafana |
| 4 | Website | HTTP(s) | `https://xdevschain.xdevs.in` | 200 | warning |
| 5 | Explorer | HTTP(s) | `https://xdevschain-explorer.xdevs.in` | 200 | warning |
| 6 | Faucet | Keyword | `https://xdevschain-faucet.xdevs.in/api/status` | `"healthy":true` | warning |
| 7 | Bootnode 1 P2P | Port | `xdevschain-boot1.xdevs.in:30333` | open | warning |
| 8 | Bootnode 2 P2P | Port | `xdevschain-boot2.xdevs.in:30333` | open | warning |
| 9-12 | Validator 1-4 P2P | Port | `xdevschain-val1..4.xdevs.in:30333` | open | warning. A **second** validator down at once = chain halt; Grafana's `XdevsQuorumLost` is the authoritative alert |

Notes:

- Monitor 1 checks that the RPC answers. It does **not** check that blocks advance. The "no new
  block for 30 s" alert lives in Grafana (`XdevsNoNewBlock`), because a 5-minute external check
  is too coarse for it.
- If custom POST bodies are not available on the free plan, Grafana Cloud **Synthetic
  Monitoring** (free-tier quota) can run an HTTP POST check with a body and a regex on the
  response. Use it for monitor 1 and keep 1b as a second opinion.
- **Status page**: put monitors 1, 3, 4, 5 and 6 on a public status page ("xDEVS Chain testnet").
  Do not put the validator port monitors on it; there is no need to advertise them. Link the
  status page from the docs site. A custom domain `xdevschain-status.xdevs.in` (DNS-only CNAME to
  `stats.uptimerobot.com`) works only if your plan includes it.
- Maintenance windows: pause monitors 1 to 3 during planned RPC-node maintenance so subscribers
  are not paged. Validators restart one at a time in a rolling upgrade, so they need no window.
