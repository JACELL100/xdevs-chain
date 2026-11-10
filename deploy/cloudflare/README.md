# Cloudflare: xdevs.in for xDEVS Chain

`xdevs.in` is on Cloudflare (free plan). Every public xDEVS Chain host is **one level below
`xdevs.in`** (`xdevschain-*.xdevs.in`), so the free Universal SSL certificate (`xdevs.in` +
`*.xdevs.in`) covers it. A name like `rpc.testnet.xdevs.in` would need Advanced Certificate
Manager (paid).

| File | What |
|---|---|
| [dns-records.md](dns-records.md) | Every DNS record, proxied or not, and why |
| [waf-and-rate-limits.md](waf-and-rate-limits.md) | WAF custom rules, the rate-limiting rule, settings that break JSON-RPC |
| [tunnel-config.example.yml](tunnel-config.example.yml) | Reference `/etc/cloudflared/config.yml` (deployed by `roles/cloudflared`) |

## Public RPC through Cloudflare Tunnel (plan §24.2)

The RPC full node runs on a home machine. It opens **no inbound port**: `cloudflared` keeps an
outbound connection to Cloudflare, and Cloudflare forwards `xdevschain-rpc` / `xdevschain-ws`
to `127.0.0.1:8545` / `127.0.0.1:8546` on that machine.

One-time setup, from a workstation with `cloudflared` installed:

```bash
cloudflared tunnel login                                   # pick the xdevs.in zone
cloudflared tunnel create xdevschain-rpc                   # prints <tunnel-id>, writes ~/.cloudflared/<tunnel-id>.json
cloudflared tunnel route dns xdevschain-rpc xdevschain-rpc.xdevs.in
cloudflared tunnel route dns xdevschain-rpc xdevschain-ws.xdevs.in
```

1. Put `<tunnel-id>` and the JSON file's content into `vault.yml`
   (`vault_cloudflared_tunnel_id`, `vault_cloudflared_credentials_json`). Then delete the JSON
   file from the workstation, or keep it only in the offline key backup.
2. `ansible-playbook site.yml --limit rpc` installs `cloudflared`, writes the config, validates
   the ingress rules and starts the service.
3. Apply [waf-and-rate-limits.md](waf-and-rate-limits.md) **before** you announce the
   endpoints.
4. Check:
   ```bash
   curl -s https://xdevschain-rpc.xdevs.in -H 'content-type: application/json' \
     --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'
   deploy/scripts/check-health.sh --rpc https://xdevschain-rpc.xdevs.in --chain-id 717171 --min-peers 0
   websocat wss://xdevschain-ws.xdevs.in   # then: {"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]}
   ```

**Fallback during the early devnet** (plan §24.2): run `cloudflared` on one validator, pointing
at its localhost RPC, with the strict limits below. Validators should not serve public RPC (§12.4),
so move it to a separate full node as soon as you can.

**Client IPs.** Every request reaches the node from `cloudflared` on 127.0.0.1, so the node's own
per-IP limiter (`[rpc] rate_limit_per_ip`) would treat all users as one client. It stays at 0 on
the RPC node. Per-client limits live at Cloudflare. The node's global caps still apply:
`max_concurrent_requests`, `evm_timeout_ms`, `max_in_flight_per_connection`, `max_batch`,
`max_logs_range` and `gas_cap`.

## Other hosts

- **Website** `xdevschain.xdevs.in`: Cloudflare Pages custom domain (docs site).
- **Explorer** `xdevschain-explorer.xdevs.in`: wherever the explorer UI is hosted (Cloudflare
  Pages / Vercel). See [dns-records.md](dns-records.md).
- **Faucet** `xdevschain-faucet.xdevs.in`: Workers custom domain of `apps/faucet`. Turnstile is
  already in the Worker. See [faucet.md](../../docs/runbooks/faucet.md).

Free-plan limits and features change. Re-check them in the dashboard before relying on a number
in these files.
