# Runbook: ERC-4337 bundler (bundler.xdevschain.tech)

Code: [`apps/bundler`](../../apps/bundler/README.md) (safety model, configuration, metrics). Deployment files:
[`deploy/bundler/`](../../deploy/bundler/) (`xdevs-bundler.service`, `bundler.env.example`). The bundler serves the
EntryPoint v0.9 (`0x433709009B8330FDa32311DF1C2AFA402eD8D009`) on Enclave (chain 717171) for the wallet
(`wallet.xdevschain.tech`) and other clients. Pattern: same as the indexer, a systemd service with its env file in
`/etc/xdevs-bundler/`, listening on `127.0.0.1:4337`, published through the host's Cloudflare Tunnel.

> tXDS have no monetary value, but the operator key pays the gas of every bundle and the paymaster sponsor's deposit
> pays sponsored activations: an outage or a drained key means the wallet cannot activate accounts.

## 1. What runs where

| Piece | Where | Notes |
|---|---|---|
| `xdevs-bundler.service` | The public RPC host (preferred: `RPC_URL=http://127.0.0.1:8545`, no public rate limits) or any VM with Node 24 (`RPC_URL=https://rpc.xdevschain.tech`) | ~60–100 MB RSS, `MemoryMax=150M`, `CPUWeight=20` |
| Operator key | `/etc/xdevs-bundler/operator-key` (root, 0600) → systemd credential | Hot key with a small float. Signs `handleOps` bundles. |
| Beneficiary (optional) | `BENEFICIARY` | Receives the bundle fees. Default: the operator itself (self-refilling). |
| State | `/var/lib/xdevs-bundler/bundler-state.json` | Mempool, reputation, sponsorship counters. Safe to delete (see §8). |
| Gas meter helper | CREATE2 address, deployed by the bundler on its first start | One transaction (~300k gas), then reused. |
| XdevsPaymaster sponsor | Sponsor EOA (cold-ish), deposit in the `0x…1007` predeploy | Pays sponsored activations; `NEXT_PUBLIC_PAYMASTER_SPONSOR` of the wallet build. |

## 2. Install (as root)

```bash
# Node 24 (NodeSource) — node must be /usr/bin/node for the unit file
curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && apt-get install -y nodejs git
node --version                                   # v24.x

useradd --system --home /var/lib/xdevs-bundler --shell /usr/sbin/nologin xdevs-bundler
git clone https://github.com/JACELL100/xdevs-chain /opt/xdevs-bundler
cd /opt/xdevs-bundler/apps/bundler
npm ci --omit=dev                                # runtime dependency: viem only
```

## 3. Operator key and configuration

```bash
install -d -m 0750 -g xdevs-bundler /etc/xdevs-bundler
install -m 0640 -g xdevs-bundler /opt/xdevs-bundler/deploy/bundler/bundler.env.example /etc/xdevs-bundler/bundler.env
$EDITOR /etc/xdevs-bundler/bundler.env           # RPC_URL, BENEFICIARY, SPONSOR_POLICY_SPONSORS (§4.2), limits

# A NEW key used only by the bundler (never reuse the faucet or a validator key):
cast wallet new                                   # note the address; keep a copy in the team password manager
install -m 0600 /dev/null /etc/xdevs-bundler/operator-key
$EDITOR /etc/xdevs-bundler/operator-key          # paste 0x<64 hex>, nothing else; do not echo it into the shell
```

The key file stays root-only: systemd's `LoadCredential=` gives the service a private copy at
`$CREDENTIALS_DIRECTORY/operator-key`, which the bundler reads; it is never in the environment or the logs. A v3
keystore also works (`LoadCredential=keystore-password:…` + `BUNDLER_KEYSTORE=/etc/xdevs-bundler/operator.json`).

## 4. Fund the operator and the paymaster

Enclave prices: base fee 0.1 gwei (the floor), suggested tip 0.001 gwei, so a bundle costs about **0.101 gwei per gas**.
Measured on the dev chain: a deploy + first-call op is ~0.49M gas, a plain call ~0.11M gas; a one-activation bundle is
≤ 1M gas ≈ **0.0001 tXDS**. Every op pays back at least the gas its bundle spends (README "Safety model"), so with the
default beneficiary the operator's balance does not go down in normal operation.

### 4.1 Operator: **5 tXDS**

- The node checks `gasLimit × maxFeePerGas` up front: a full bundle (15M gas × 0.201 gwei) needs ~0.003 tXDS on hand.
- `OPERATOR_MIN_BALANCE=0.05` (floor: below it the bundler refuses ops and `/health` is 503), `OPERATOR_LOW_BALANCE=0.5`
  (alert). 5 tXDS is ~50,000 activation bundles even if no fee came back, and bounds what a leaked key can lose.
- With a separate `BENEFICIARY` the operator only spends: top it up from the beneficiary when the alert fires.

```bash
cast send <OPERATOR_ADDRESS> --value 5ether --rpc-url https://rpc.xdevschain.tech --account <funding-account>
```

### 4.2 XdevsPaymaster (sponsored activation): **1 tXDS stake + 10 tXDS sponsor deposit**

The wallet's "Activate" op uses `paymaster = 0x…1007`, `paymasterData = <sponsor>`. On Enclave the paymaster is
predeployed but **unstaked and without sponsors**; this bundler refuses unstaked paymasters (ERC-7562: it reads
sponsor-keyed storage in validation). One-time setup:

```bash
RPC=https://rpc.xdevschain.tech; PM=0x0000000000000000000000000000000000001007

# (a) EntryPoint stake for the paymaster: 1 tXDS, unstake delay 1 day (= MIN_STAKE / MIN_UNSTAKE_DELAY).
#     Permissionless, but only Governance can unlock / withdraw it later: treat it as locked.
cast send $PM "addStake(uint32)" 86400 --value 1ether --rpc-url $RPC --account <funding-account>

# (b) The wallet's sponsor (a dedicated EOA = NEXT_PUBLIC_PAYMASTER_SPONSOR): active, open to all, at most
#     0.005 tXDS per op and 0.005 tXDS lifetime per account (an activation reserves ~0.00025 and is charged ~0.0001),
#     no off-chain signer.
cast send $PM "configure(bool,bool,uint128,uint128,address)" true true 5000000000000000 5000000000000000 \
  0x0000000000000000000000000000000000000000 --rpc-url $RPC --account <sponsor-account>

# (c) Deposit 10 tXDS for that sponsor (anyone can top it up): ~50,000-100,000 activations.
cast send $PM "deposit(address)" <SPONSOR_ADDRESS> --value 10ether --rpc-url $RPC --account <funding-account>

# Check
cast call $PM "sponsorInfo(address)((uint128,uint128,uint128,bool,bool,address))" <SPONSOR_ADDRESS> --rpc-url $RPC
cast call 0x433709009B8330FDa32311DF1C2AFA402eD8D009 "getDepositInfo(address)((uint256,bool,uint112,uint32,uint48))" $PM --rpc-url $RPC
```

Then set `SPONSOR_POLICY_SPONSORS=<SPONSOR_ADDRESS>` in the env file and `NEXT_PUBLIC_PAYMASTER_SPONSOR=<SPONSOR_ADDRESS>`
(plus `NEXT_PUBLIC_BUNDLER_URL=https://bundler.xdevschain.tech`) in the wallet build. The bundler's policy then allows
one sponsored op per account, which must be its activation, and `SPONSORED_OPS_PER_IP_PER_HOUR=10` per client IP; the
on-chain `perAccountCap` bounds the sponsor's exposure even against other bundlers. Refill the deposit when
`sponsorInfo(...).balance` drops below ~2 tXDS.

## 5. Start

```bash
cp /opt/xdevs-bundler/deploy/bundler/xdevs-bundler.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now xdevs-bundler
journalctl -u xdevs-bundler -f -o cat            # JSON lines: "bundler ready", "deployed gas meter" (first start), "listening"
curl -s http://127.0.0.1:4337/health             # {"status":"ok",...}
```

The first start on Enclave sends one transaction (gas meter deployment through the CREATE2 proxy) and waits for it.

## 6. Cloudflare Tunnel: bundler.xdevschain.tech → 127.0.0.1:4337

On the host's cloudflared (`/etc/cloudflared/config.yml`, see [deploy/cloudflare](../../deploy/cloudflare/README.md)),
add the ingress rule **before** the final `http_status:404` rule, then route DNS and restart:

```yaml
  - hostname: bundler.xdevschain.tech
    service: http://127.0.0.1:4337
```

```bash
cloudflared tunnel route dns <tunnel-name-or-id> bundler.xdevschain.tech
systemctl restart cloudflared
```

(If cloudflared runs on another machine, point the rule at that host's private address, bind the bundler with `HOST=` to
that interface, and set `TRUSTED_PROXIES` to cloudflared's address.) `TRUST_PROXY=cloudflare` makes the per-IP limits
use `CF-Connecting-IP`, honoured only from the trusted peers. Optional second layer: a Cloudflare WAF rate-limiting rule on
`bundler.xdevschain.tech` (e.g. 120 requests / 10 s per IP).

## 7. Verify

```bash
B=https://bundler.xdevschain.tech
curl -s $B -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'                 # "0xaf173"
curl -s $B -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"eth_supportedEntryPoints","params":[]}'    # ["0x4337…D009"]
curl -s $B/health                                       # 200 {"status":"ok",...}
curl -s -o /dev/null -w '%{http_code}\n' $B/metrics     # 404 through the tunnel (local scrapes only)
curl -s http://127.0.0.1:4337/metrics | grep bundler_operator_balance

# Smoke test (from a checkout with `npm ci` in apps/bundler):
cd apps/bundler
node scripts/smoke.ts --bundler $B --rpc https://rpc.xdevschain.tech                       # read-only: estimate an activation
node scripts/smoke.ts --bundler $B --rpc https://rpc.xdevschain.tech --sponsor <SPONSOR> --send   # one real sponsored activation
```

Then activate an account in the wallet (passkey sign-up → Activate) and check the op in the explorer.

## 8. Operations

- **Monitoring**: UptimeRobot keyword monitor on `https://bundler.xdevschain.tech/health` for `"status":"ok"` (503 when the
  RPC is unreachable, the operator is below its floor or the process is stopping). Prometheus scrape of
  `127.0.0.1:4337/metrics`; alert on `bundler_operator_balance_low == 1`, `increase(bundler_bundles_failed_total[1h]) > 0`,
  `bundler_rpc_up == 0`, `bundler_mempool_ops{status="pending"} > 100` for 10 min.
- **Refill**: §4.1 / §4.2. Write each refill in the ops log.
- **Upgrade**: `cd /opt/xdevs-bundler && git pull && cd apps/bundler && npm ci --omit=dev && systemctl restart xdevs-bundler`.
  The stop is graceful (`SIGTERM`: no new ops, the bundle being submitted finishes, state written); pending ops and
  submitted bundles are picked up again after the restart.
- **Reset reputation / mempool**: `systemctl stop xdevs-bundler`, move `/var/lib/xdevs-bundler/bundler-state.json` away,
  start. (Ops in it are lost; clients resubmit.) To exempt an entity you run, use `REPUTATION_ALLOWLIST`.
- **Pause**: `systemctl stop xdevs-bundler` (the wallet shows the bundler as unreachable), or remove the tunnel rule.
- **Bundles failing** (`bundler_bundles_failed_total` rising, "bundle reverted on chain" in the logs): the logs name the
  banned entity and the AA reason. Three failures in a row pause bundling for 60 s by design. If a paymaster you trust was
  banned, fix its cause, then reset (above) or allowlist it.
- **Stuck operator nonce** ("bundle not mined yet" for minutes): compare `cast nonce <OPERATOR> --block latest` and
  `--block pending`; fill a gap with a 0-value self-transfer at the latest nonce with a higher tip, from a trusted machine
  with the key, then restart the service.

## 9. Key compromise

1. `systemctl stop xdevs-bundler`.
2. Sweep the old operator's balance to a safe address (high tip; the float is small by design, accept losing the race).
3. New key (`cast wallet new`), write it to `/etc/xdevs-bundler/operator-key`, fund it (§4.1), `systemctl start xdevs-bundler`.
4. The paymaster deposit and stake are not reachable with the operator key; the sponsor key is separate.
5. Find the cause (who had root on the host), write a short incident note.
