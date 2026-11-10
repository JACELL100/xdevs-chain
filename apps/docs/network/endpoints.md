# Network endpoints

::: warning Coming soon
**None of these endpoints are live yet.** They are reserved for the public testnet, which launches in Phase 3. Until then, run a [local dev chain](/guide/quickstart) or a [local multi-validator network](/operators/local-network).
:::

## Planned public testnet

| Service | URL | Status |
|---|---|---|
| Website and docs | `https://xdevschain.xdevs.in` | This site |
| JSON-RPC (HTTPS) | `https://xdevschain-rpc.xdevs.in` | <span class="status next">Coming soon</span> |
| WebSocket | `wss://xdevschain-ws.xdevs.in` | <span class="status next">Coming soon</span> |
| Block explorer | `https://xdevschain-explorer.xdevs.in` | <span class="status next">Coming soon</span> |
| Faucet | `https://xdevschain-faucet.xdevs.in` | <span class="status next">Coming soon</span> |
| Chain id | To be announced | <span class="status next">Coming soon</span> |

Service hosts use one subdomain level (`xdevschain-*.xdevs.in`) so they are covered by a single wildcard certificate for `*.xdevs.in`.

When the testnet launches, the chain id will be published here and registered on [chainlist](https://chainlist.org), and this page will have an **Add to MetaMask** button.

## Faucet

The faucet is built (a Cloudflare Worker with a Turnstile captcha) but not deployed. The planned policy:

- **100 tXDS** per request.
- Once per 24 hours per address **and** per IP.
- Larger amounts for validators and dApp teams on request.

Source: [`apps/faucet`](https://github.com/JACELL100/xdevs-chain/tree/main/apps/faucet) · Operations: [faucet runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/faucet.md).

::: danger tXDS has no value
Testnet tokens have **no monetary value**, are free, must never be sold, and **will not convert** to mainnet XDS. Anyone offering to sell you tXDS is scamming you.
:::

## Public RPC limits (planned)

The public RPC will be read-and-send only, with `debug_`, `txpool_` and `admin_` disabled, and these limits (most are node defaults already):

| Limit | Value |
|---|---|
| `eth_getLogs` range / results | 10,000 blocks / 10,000 results |
| `eth_call` gas cap | 50,000,000 |
| Batch size | 50 requests |
| WebSocket subscriptions | 20 per connection |
| Rate limiting | Per IP, at Cloudflare and in the node |
