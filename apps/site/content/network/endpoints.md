# Network endpoints

<span class="status ok">Live</span> · **xDEVS Chain Enclave**, the public testnet, has been running since 17 November 2026 with four validators in Oracle Cloud (two in Hyderabad, two in Mumbai).

:::warning[Testnet]
Enclave is a test network. It may be reset (with notice on the [status page](https://status.xdevschain.tech)) while the protocol is in Phase 3, and its coin has no value.
:::

## Enclave testnet

| Service | URL | Status |
|---|---|---|
| JSON-RPC (HTTPS) | `https://rpc.xdevschain.tech` | <span class="status ok">Live</span> |
| WebSocket | `wss://ws.xdevschain.tech` | <span class="status ok">Live</span> |
| Network status | [`https://status.xdevschain.tech`](https://status.xdevschain.tech) | <span class="status ok">Live</span>: blocks, finality, validators, every endpoint |
| Block explorer | [`https://explorer.xdevschain.tech`](https://explorer.xdevschain.tech) | <span class="status ok">Live</span> |
| Wallet | [`https://wallet.xdevschain.tech`](https://wallet.xdevschain.tech) | <span class="status ok">Live</span> (preview); passkey accounts need the bundler |
| Faucet | [`https://faucet.xdevschain.tech`](https://faucet.xdevschain.tech) | <span class="status ok">Live</span>: 100 tXDS per claim, every 24 h |
| Explorer API | `https://explorer-api.xdevschain.tech/api/v1/…` | <span class="status ok">Live</span>: indexer for history, tokens, approvals and vault queues ([spec](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/explorer-api.md)) |
| ERC-4337 bundler | `https://bundler.xdevschain.tech` | <span class="status next">Next</span> |
| Website and docs | `https://xdevschain.tech` | This site |

Opening `https://rpc.xdevschain.tech` in a browser takes you to its [info page](https://status.xdevschain.tech/rpc).

## Add Enclave to your wallet

| Field | Value |
|---|---|
| Network name | xDEVS Chain Enclave |
| RPC URL | `https://rpc.xdevschain.tech` |
| Chain ID | `717171` (`0xaf173`) |
| Currency | EnclaveXDS, symbol `tXDS` (18 decimals) |
| Block explorer | `https://explorer.xdevschain.tech` |

The [status page](https://status.xdevschain.tech), the [faucet](https://faucet.xdevschain.tech) and the [SDK](/developers/sdk) (`addXdevsChainToWallet(wallet, xdevsEnclave)`) can add it for you.

```bash
# Latest block
cast block-number --rpc-url https://rpc.xdevschain.tech
# Or with curl
curl -s https://rpc.xdevschain.tech -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}'
```

## Network facts

| | |
|---|---|
| Chain id | 717171 (`xdevs-enclave-1`) |
| Block time | 2 s |
| Finality | Every committed block is final (BFT, 2/3+ of the validators sign it) |
| Validators | 4, equal power, set in genesis (staking-driven changes come with Phase 3) |
| Genesis | Built with [`tools/genesis-builder`](https://github.com/JACELL100/xdevs-chain/tree/main/tools/genesis-builder) from [`configs/enclave.json`](https://github.com/JACELL100/xdevs-chain/blob/main/tools/genesis-builder/configs/enclave.json); system contracts at `0x…1000`–`0x…1009` ([predeploys](/concepts/predeploys)) |
| Node release | [`v0.2.0-devnet.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.2.0-devnet.1) |
| Validator P2P | Open only between the validators; users and apps connect through the RPC |

## Faucet

The faucet is a Cloudflare Worker with a Turnstile captcha, paying out from a small hot wallet:

- **100 tXDS** per request.
- Once per 24 hours per address **and** per IP.
- Larger amounts for validators and dApp teams on request.

Source: [`apps/faucet`](https://github.com/JACELL100/xdevs-chain/tree/main/apps/faucet) (API) and [`apps/faucet-web`](https://github.com/JACELL100/xdevs-chain/tree/main/apps/faucet-web) (page) · Operations: [faucet runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/faucet.md).

:::danger[tXDS has no value]
Testnet tokens have **no monetary value**, are free, must never be sold, and **will not convert** to mainnet XDS. Anyone offering to sell you tXDS is scamming you.
:::

## Public RPC limits

The public RPC runs on a full node behind a Cloudflare Tunnel; the node opens no inbound port. Its limits (node defaults):

| Limit | Value |
|---|---|
| `eth_getLogs` range / results | 10,000 blocks / 10,000 results |
| `eth_call` gas cap | 50,000,000 |
| Batch size | 50 requests |
| WebSocket subscriptions | 20 per connection |
| Rate limiting | At Cloudflare (all clients reach the node through the tunnel) |

For heavy use, [run your own node](/operators/run-a-node).
