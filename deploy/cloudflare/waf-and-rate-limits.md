# WAF and rate limiting for the public RPC

Plan §19.3: a read-only public endpoint, rate-limited at Cloudflare, with the node's own caps
as the second layer. These rules apply to `xdevschain-rpc.xdevs.in` and `xdevschain-ws.xdevs.in`
on the **free plan**. Check the current free-plan quotas in the dashboard. When these notes were
written, the free plan included 5 WAF custom rules, 1 rate-limiting rule (counted per IP, 10 s
period), and the Free Managed Ruleset.

## Settings that break JSON-RPC: turn them off for the RPC hosts

JSON-RPC clients (ethers, viem, web3.py, Foundry `cast`, MetaMask's background requests) are not
browsers. They cannot solve a JavaScript challenge or a CAPTCHA.

| Setting | Where | For the RPC hosts |
|---|---|---|
| **Bot Fight Mode** | Security -> Bots | **Off.** It is zone-wide on the free plan and cannot be scoped to a host. Leave it off while the zone serves the RPC. |
| **Browser Integrity Check** | Configuration Rule: `http.host in {"xdevschain-rpc.xdevs.in" "xdevschain-ws.xdevs.in"}` -> Browser Integrity Check **Off** | Off |
| **Security Level / "I'm Under Attack"** | Same Configuration Rule -> Security Level **Essentially Off** (never use Under Attack on these hosts) | Off |
| **Caching** | Default is fine: POST is never cached. Do not add a "Cache Everything" rule that matches these hosts. | No cache |
| **Rocket Loader / Auto Minify / Email obfuscation** | They only touch HTML. Leave them off for these hosts anyway. | Off |
| **WebSockets** | Network -> WebSockets | **On** (needed for `wss://`) |

## WAF custom rules (Security -> WAF -> Custom rules), in this order

1. **Only JSON-RPC methods on the RPC hosts.** Action: *Block*
   ```text
   (http.host in {"xdevschain-rpc.xdevs.in" "xdevschain-ws.xdevs.in"})
   and not (http.request.method in {"POST" "OPTIONS" "GET"})
   ```
   GET stays open because WebSocket upgrades are GET requests.
2. **WS host: only WebSocket upgrades.** Action: *Block*
   ```text
   (http.host eq "xdevschain-ws.xdevs.in")
   and not any(lower(http.request.headers["upgrade"][*])[*] eq "websocket")
   and http.request.method ne "OPTIONS"
   ```
   If your plan rejects this expression, use
   `(http.host eq "xdevschain-ws.xdevs.in" and http.request.method eq "POST")` instead.
3. **RPC host: GET only on `/`** (health probes, browsers that open the URL). Action: *Block*
   ```text
   (http.host eq "xdevschain-rpc.xdevs.in" and http.request.method eq "GET"
    and http.request.uri.path ne "/")
   ```
4. *(Optional)* **Known-abusive ASNs or countries.** Add one only after the logs show abuse.
   Action: *Managed Challenge* does not work for RPC clients, so use *Block*.

Debug and admin namespaces (`debug_`, `txpool_`, `admin_`) are already **not enabled** on the
node (`[rpc] namespaces = ["eth","net","web3","xdevs"]`, `group_vars/rpc.yml`). The node is the
real control. Body-matching WAF fields are not available on every plan, so do not rely on them.

## Rate-limiting rule (Security -> WAF -> Rate limiting rules): the free plan has one

```text
If incoming requests match:  (http.host in {"xdevschain-rpc.xdevs.in" "xdevschain-ws.xdevs.in"})
Characteristics:             IP                          (free plan: IP only)
Period:                      10 seconds                  (free plan: 10 s)
Requests:                    100                         (= 10 req/s per IP, bursts allowed)
Action:                      Block, for 10 seconds       (free plan: 10 s mitigation)
```

A JSON-RPC **batch** (up to 50 calls, `[rpc] max_batch = 50`) counts as one HTTP request here.
The node counts each call, so heavy batch users hit the node's global caps
(`max_concurrent_requests`, `evm_timeout_ms`), not Cloudflare. A WebSocket counts once when it
opens. The node enforces the subscription caps on the connection itself.

Tune it: watch Security -> Events for blocks of legitimate dApps. If the faucet or explorer
backend calls the RPC from Cloudflare Workers, their requests come from Cloudflare IPs. Give
them their own endpoint (for example a service binding, or a separate tunnel hostname that only
they know) rather than raising the public limit.

## Managed rules

Enable the **Cloudflare Free Managed Ruleset** (Security -> WAF -> Managed rules). It targets
common web exploits, does not inspect JSON-RPC semantics, and is safe for these hosts.

## When the RPC is under attack

1. Security -> Events: find the top IPs / ASNs / user agents.
2. Add a temporary Block rule (custom rule 4) for the source. Lower the rate limit to 50 / 10 s.
3. If the node itself is saturated: `journalctl -u xdevs` on rpc-1, and the RPC latency panels
   in Grafana. As a last resort, stop `cloudflared` on rpc-1 (`systemctl stop cloudflared`). The
   chain keeps running, because validators do not depend on the public RPC.
4. Record the incident (time, source, rule added). Remove temporary rules after 7 days.
