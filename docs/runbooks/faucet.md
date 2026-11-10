# Runbook: tXDS faucet (deploy, refill, key compromise)

Runbook index item 6 (plan §33.8): **Faucet wallet refill & compromise response.**
Code: `apps/faucet/` (Cloudflare Worker + Turnstile + one Durable Object). Policy: plan §20.3.
Threat: "Faucet draining" (plan §25.1). Hosting: Cloudflare Workers free plan (plan §24.1).

> tXDS are testnet tokens with **no monetary value**. They will not convert to mainnet XDS.
> We still protect the faucet: if someone drains it, real users cannot test.

## 1. What runs where

| Piece | What it does |
|---|---|
| Worker `xdevs-faucet` | `GET /` form, `POST /api/claim`, `GET /api/status`. Checks the address and the Turnstile token. |
| Durable Object `FaucetSigner` (one instance, `faucet-hot-wallet`) | The only code that signs. Handles one claim at a time: checks the rate limit, checks the balance, picks the nonce, signs an EIP-1559 tx and sends it. |
| KV namespace `RATE_LIMIT` | Keys `addr:<address>`, `ip:<ipv4>`, `ip6:<prefix>::/64`. Each key expires after 24 h. |
| Hot wallet (`FAUCET_PRIVATE_KEY`) | Holds a **small** float only. Topped up from the faucet reserve. |
| Faucet & community reserve (300M tXDS at genesis, plan §20.2) | Cold key, kept offline. **Never** loaded into the Worker. |

### Configuration

| Name | Kind | Example / default | Notes |
|---|---|---|---|
| `FAUCET_PRIVATE_KEY` | secret | `0x` + 64 hex | Hot wallet key. The code never logs it or returns it. |
| `TURNSTILE_SECRET_KEY` | secret | from the Turnstile dashboard | Used for the server-side `siteverify` check. |
| `RPC_URL` | var | `https://rpc.testnet.<domain>` | The public RPC full node. |
| `CHAIN_ID` | var | `717171` | Checked against `eth_chainId` once per DO lifetime. If it does not match, claims return 503. |
| `TURNSTILE_SITE_KEY` | var | public site key | Shown in the page. |
| `DRIP_AMOUNT` | var | `100` | tXDS per claim. |
| `MIN_FAUCET_BALANCE` | var | `1000` | Below this, claims return 503 `faucet_empty`. |
| `CORS_ORIGINS` | var | explorer and wallet origins, comma-separated | Only these origins get CORS headers on `/api/*`. |
| `EXPLORER_TX_URL` | var | `https://explorer.testnet.<domain>/tx/{hash}` | Leave it empty if there is no explorer yet. |
| `TURNSTILE_EXPECTED_HOSTNAME` | var | `faucet.testnet.<domain>` | Optional extra check. Leave it empty when using the Turnstile test keys. |

## 2. Deploy (not done yet — run these later)

Wrangler is not installed in the repo because it is large. Use a machine with enough disk space, or `npx wrangler@4`.

```bash
cd apps/faucet
npm ci
npm run check                       # tsc --noEmit + node --test (offline, mocked)

npx wrangler@4 login

# 1. KV namespace for rate limits -> paste the printed ids into wrangler.toml
npx wrangler@4 kv namespace create RATE_LIMIT
npx wrangler@4 kv namespace create RATE_LIMIT --preview

# 2. Turnstile: Cloudflare dashboard -> Turnstile -> Add widget
#    Hostnames: faucet.testnet.<domain> (+ the *.workers.dev host while testing)
#    Mode: Managed. Put the SITE key in wrangler.toml [vars] TURNSTILE_SITE_KEY.
npx wrangler@4 secret put TURNSTILE_SECRET_KEY      # paste when prompted

# 3. Hot wallet: create a NEW key used only for the faucet (never reuse a key)
cast wallet new                     # or: xdevs-cli keys new  (once available)
npx wrangler@4 secret put FAUCET_PRIVATE_KEY        # paste when prompted; do NOT pass it as an argument or echo it
#    Then delete the key from your terminal scrollback and clipboard. Keep a copy only in the team password manager.

# 4. Fund the hot wallet from the reserve (see §3 Refill), then fill in RPC_URL, CHAIN_ID,
#    CORS_ORIGINS, EXPLORER_TX_URL and TURNSTILE_EXPECTED_HOSTNAME in wrangler.toml.

# 5. Ship
npx wrangler@4 deploy --dry-run --outdir dist   # checks the bundle and its size
npx wrangler@4 deploy

# 6. Smoke test
curl -s https://faucet.testnet.<domain>/api/status    # address, balance, healthy:true
#    Then make one real claim from a browser and check the tx in the explorer.
npx wrangler@4 tail                                   # live logs (no secrets are ever logged)
```

The DO migration (`new_sqlite_classes = ["FaucetSigner"]`) runs with the first deploy. The Workers Free plan only allows SQLite-backed Durable Objects. Check that this is still true before you deploy.

**Monitoring:** add an UptimeRobot keyword monitor on `/api/status` that looks for `"healthy":true`. It goes false when the balance drops below `max(MIN_FAUCET_BALANCE, DRIP_AMOUNT)` or the RPC is down. Send the alert to the ops Discord channel.

## 3. Refill (routine)

- **Float size:** keep about **10,000–20,000 tXDS** in the hot wallet, which covers 100–200 claims. Keep it small on purpose: if the key leaks, that is all you lose.
- **When to refill:** when `/api/status` shows less than ~3,000 tXDS, or when the low-balance alert fires. Below 1,000 tXDS (`MIN_FAUCET_BALANCE`) the faucet pauses itself and shows a friendly 503.
- **How:** sign from the reserve key on an offline or hardware wallet. Never put the reserve key on a server.
  ```bash
  cast send <FAUCET_HOT_ADDRESS> --value 10000ether --rpc-url $RPC_URL --ledger   # or xdevs-cli tx send
  ```
- Write each refill in the ops log: date, amount, tx hash, and the balance before and after. A sudden rise in refill frequency is the main sign that someone is draining the faucet.
- **Large requests** (validators, dApp teams, plan §20.3) are paid directly from the reserve after a Discord or form request. They never go through the Worker.

## 4. Key compromise / draining response

**Signs:** the balance drops faster than claims explain. Txs from the hot address that are not in `wrangler tail`. Many claims from one ASN or IP range. The Turnstile dashboard shows a high solve rate from bots.

1. **Pause now** (any one of these, fastest first):
   - `npx wrangler@4 secret delete FAUCET_PRIVATE_KEY`. Claims then return 503 `misconfigured`, and the Worker can no longer sign.
   - Or set `MIN_FAUCET_BALANCE = "1000000000"` and run `wrangler deploy`. The faucet then reports "running low".
   - Or remove the route or custom domain in the dashboard.
2. **If the key itself may be leaked** (secret exposed, laptop lost, unexplained txs):
   1. Make a new hot key on a clean machine (`cast wallet new`).
   2. **Sweep** what is left in the old hot wallet to the reserve, with a high priority fee so it gets in ahead of the attacker. The amount is small by design, so if you lose this race, accept the loss and move on.
   3. `npx wrangler@4 secret put FAUCET_PRIVATE_KEY` with the new key. Fund it with a **small** float.
   4. Deploy (`npx wrangler@4 deploy`). This also restarts the Durable Object, which resets its in-memory nonce.
   5. Find the cause: who had access to the key, in Cloudflare account audit logs and CI secrets. Rotate the Cloudflare API tokens and the account password, and make sure 2FA is on.
   6. Publish the new faucet address (docs, explorer label) so users can check it.
3. **If it is abuse through the Worker (key safe):**
   - Lower `DRIP_AMOUNT` (e.g. `"10"`) and redeploy.
   - In the Turnstile dashboard, keep the widget on Managed mode. Do not switch to Non-interactive or Invisible, which are weaker. Rotate the Turnstile secret if it may have leaked.
   - Add a Cloudflare WAF rate-limiting rule on `POST /api/claim`. The free plan includes one rule.
   - Block the abusing ASNs or countries with a WAF custom rule.
   - Speed up the GitHub-login limit (TODO below).
4. **Turnstile secret leaked:** rotate it in the Turnstile dashboard, then `wrangler secret put TURNSTILE_SECRET_KEY`. A leaked secret alone does not let anyone skip the captcha. Tokens still have to come from a real widget solve. But rotate it anyway.
5. Write a short incident note: timeline, how much was lost, the cause, and the follow-ups.

## 5. Stuck nonce

Symptom: claims succeed (the node accepted the tx) but the txs never get mined. Or claims fail with "nonce too low" or "replacement transaction underpriced".

1. Compare `eth_getTransactionCount(<hot>, "latest")` with `"pending"`. If `pending > latest` and nothing gets mined, a lower-nonce tx was probably dropped. That leaves a gap.
2. Fill the gap: send a 0-value self-transfer at the `latest` nonce with a higher fee, from a trusted machine that has the hot key (`cast send <hot> --value 0 --nonce <latest> --priority-gas-price 2gwei ...`).
3. `npx wrangler@4 deploy`, even with no changes, to restart the Durable Object. It then reads the nonce from the RPC again. The DO also re-reads the nonce from the RPC after any failed send.

## 6. Limits & known gaps (be honest with users)

- **Policy:** `DRIP_AMOUNT` (100 tXDS) once per 24 h per address **and** per IP. For IPv6, the limit applies to the whole /64. Turnstile is checked on every claim. Recipients refused: the zero address, everything in `0x…0000`–`0x…ffff` (precompiles and system predeploys `0x…1000`–`0x…1009`), the system caller `0xff…fe`, and the faucet's own address.
- **KV is not atomic.** Workers KV has no compare-and-set and is eventually consistent. Writes can take about 60 s or more to show up in other locations. To narrow the gap, the rate-limit check and the reservation both happen inside the single `FaucetSigner` DO, one claim at a time, in one location. KV's local cache can still return an old miss, so in rare cases one address or IP can get two drips within 24 h. The damage is capped by the drip size, the small float and the balance floor. **Fix if needed:** move the markers into the DO's own SQLite storage, which is strongly consistent and much cheaper on writes (see the next point).
- **KV free-tier write quota:** 1,000 writes/day and 1,000 deletes/day (check current limits). Each successful claim writes 2 keys, so the free plan tops out at about **500 claims/day**. After that, KV writes fail and claims return errors. Upgrade, or move the markers into DO storage, before a public launch.
- **Throughput:** all signing goes through one DO, one claim at a time, with about 5 RPC round trips per claim. That is roughly 1–3 claims per second. This is fine for a testnet faucet. Rate-limited requests also wait in the queue.
- **Nonce:** `max(pending nonce from the RPC, last used + 1)`. If the node drops a tx, later txs queue behind the gap until §5 is done.
- **IP limits are weak** against attackers who rent many IPv4 proxies. Turnstile is the main defence. Next steps: the GitHub limit and a WAF rule.
- **TODO — per-GitHub-account limit (plan §20.3):** add optional GitHub OAuth, keep the user id in a signed cookie, and add a `gh:<id>` key to the same check.
- **TODO — `.xdevs` names:** the form accepts names, but the API returns 400 `names_not_supported` until the node exposes `xdevs_resolveName`. Once it does, implement `resolveName()` in `src/address.ts`.
- **TODO — legal:** terms of service and a privacy policy for the faucet (plan §30 checklist). The faucet sends client IPs to Cloudflare Turnstile and keeps IP-derived keys for 24 h.
- **Not tested against real Cloudflare:** the tests are offline and use mocks for KV, Turnstile, JSON-RPC and the DO stub. Run `wrangler dev` with the Turnstile test keys against a local devnet before the first deploy.
