# xDEVS bundler (ERC-4337 v0.9)

A single-operator ERC-4337 bundler for xDEVS Chain: the Enclave testnet (`https://bundler.xdevschain.tech`, chain
717171) and local dev chains (`xdevs run --dev`, anvil). It serves the canonical EntryPoint v0.9
(`0x433709009B8330FDa32311DF1C2AFA402eD8D009`) for the reference wallet (`apps/wallet`) and any other client.

It is built to be exposed publicly: every op is validated against current chain state and ERC-7562-style mempool rules
before it is accepted, bundles can only pay the operator at least what they cost, and the HTTP layer is rate-limited.
Deployment: [`docs/runbooks/bundler.md`](../../docs/runbooks/bundler.md) and [`deploy/bundler/`](../../deploy/bundler).

## Methods

| Method | Notes |
|---|---|
| `eth_chainId`, `eth_supportedEntryPoints` | |
| `eth_estimateUserOperationGas(op, ep)` | Gas fields may be omitted. Send a **dummy signature of the real shape** (signature failures are ignored while estimating). State overrides are not supported. Returns `preVerificationGas`, `verificationGasLimit`, `callGasLimit` (+ `paymasterVerificationGasLimit`, `paymasterPostOpGasLimit` with a paymaster). |
| `eth_sendUserOperation(op, ep)` | Validates the op (rules below) and adds it to the mempool; returns the userOpHash. The op is bundled within about a second. ERC-7769 error codes: `-32500` AA1x/AA2x, `-32501` AA3x / paymaster deposit / sponsorship policy, `-32502` STO-040, `-32503` AA22/AA32 or validUntil too close, `-32504` entity banned / throttled / over its mempool limit, `-32505` paymaster not staked, `-32506` aggregator, `-32507` AA24/AA34, `-32602` bad fields / fees / nonce gap / replacement underpriced, `-32005` rate limit or mempool full, `-32000` bundler paused (operator balance below its floor, shutting down). |
| `eth_getUserOperationReceipt(hash)` | `{ userOpHash, entryPoint, sender, nonce, paymaster, actualGasCost, actualGasUsed, success, reason, logs, receipt }`; `logs` are the op's own execution logs (between the previous op's `UserOperationEvent` / `BeforeExecution` and its own). `null` while pending. |
| `eth_getUserOperationByHash(hash)` | `{ userOperation, entryPoint, transactionHash, blockHash, blockNumber }` (the last three `null` while pending). |

UserOperations use the standard v0.7+/v0.9 JSON shape (`factory`/`factoryData`, `paymaster`/`paymaster…GasLimit`/`paymasterData`).
Receipts of ops this process did not submit are found through `UserOperationEvent` logs (last ~9,000 blocks).

HTTP: `POST /` JSON-RPC (batches up to `MAX_BATCH`), `GET /health` (200 / 503 with a JSON body), `GET /metrics`
(Prometheus; local scrapes only by default), `GET /` (service info).

## Safety model

**Admission** (`eth_sendUserOperation`, in this order; any failure rejects the op):

1. Shape and size: signature present, encoded op ≤ `MAX_OP_BYTES`, `verificationGasLimit` / `paymasterVerificationGasLimit`
   ≤ `MAX_VERIFICATION_GAS`, `callGasLimit` ≤ `MAX_CALL_GAS` (and ≥ 9,100 with callData), total gas fits a bundle.
2. **No operator loss**: `preVerificationGas` must cover the op's calldata in `handleOps`, the per-op loop overhead **and the
   full 21,000 intrinsic gas** (what `eth_estimateUserOperationGas` returns, +10 %). In a bundle of n ops the intrinsic gas
   is paid once but charged n times, so the beneficiary always receives at least the gas the bundle uses.
3. Fees: `maxPriorityFeePerGas` ≥ `MIN_PRIORITY_FEE_PER_GAS`, `maxFeePerGas` ≥ base fee × `BASE_FEE_MARGIN_PCT` % + that tip.
4. Operator balance ≥ `OPERATOR_MIN_BALANCE`, otherwise `-32000` (the bundler is paused, not the op's fault).
5. Sponsorship policy (below), paymaster **stake** (≥ `MIN_STAKE`, unstake delay ≥ `MIN_UNSTAKE_DELAY`) and **deposit**
   (must cover the `maxCost` of all its pending ops).
6. Mempool rules (ERC-7562): nonce not used and no gap (lower sequences of the same key must be pending), replacement of
   the same sender + nonce only with +10 % on both fee fields and never once it is in a submitted bundle, at most
   `SAME_SENDER_MEMPOOL_COUNT` ops per sender, `SAME_UNSTAKED_ENTITY_MEMPOOL_COUNT` per unstaked factory / paymaster
   (staked ones get more as they get ops included), `THROTTLED_ENTITY_MEMPOOL_COUNT` for throttled entities, banned
   entities refused, STO-040 (an address is a sender or a factory / paymaster in the mempool, not both), `MAX_MEMPOOL_OPS`.
7. **Simulation on current state**: `eth_call` of `handleOps([...pending predecessors of this sender, op])` from the
   operator: nonce, deployment, signature, prefund, paymaster validation and time range as the EntryPoint sees them. Then
   the gas meter (below) reads the account's and paymaster's `validationData`: aggregators are refused, ops expiring in
   less than `MIN_VALID_UNTIL_SECONDS` are refused (the meter step is skipped for chained same-sender ops).

**Reputation** (ERC-7562): each accepted op counts `opsSeen` for its sender, factory and paymaster, each included op
counts `opsIncluded`; both decay by 1/24 per hour. An entity whose `opsSeen / MIN_INCLUSION_RATE_DENOMINATOR` exceeds
`opsIncluded + THROTTLING_SLACK` is throttled (few ops in the mempool and per bundle, dropped if not included within
`THROTTLED_ENTITY_LIVE_BLOCKS`), beyond `+ BAN_SLACK` it is banned. An entity whose op passes simulation but makes a
bundle revert on chain is banned at once (`opsSeen = 10000`). `REPUTATION_ALLOWLIST` exempts entities you run yourself.

**Bundling** (every `BUNDLE_INTERVAL_MS`, and `BUNDLE_COLLECT_MS` after a new op):

- picks pending ops by effective tip, lowest nonce first per sender, several senders (and consecutive nonces of one
  sender) per bundle, up to `MAX_BUNDLE_OPS` / `MAX_BUNDLE_GAS`;
- **re-simulates the whole bundle** right before submitting and drops ops invalidated by earlier ones or by new state;
- the bundle transaction's `maxFeePerGas` / `maxPriorityFeePerGas` are the **minimum** of its ops' caps, so every op pays at
  least the operator's price per gas;
- pipelining: up to `MAX_INFLIGHT_BUNDLES` bundles in flight, only if they share no sender, factory or paymaster (ops of
  an entity in flight wait for its receipt);
- operator balance below `OPERATOR_MIN_BALANCE` or below the bundle's worst-case cost → no submission; three failed
  bundles in a row → bundling pauses for 60 s; a bundle not mined within `RECEIPT_TIMEOUT_MS` is waited for while the
  node still knows it, otherwise its ops go back to the mempool (dropped after 3 attempts).

**Operator key**: from `BUNDLER_PRIVATE_KEY_FILE`, a systemd credential (`$CREDENTIALS_DIRECTORY/operator-key`), a v3
keystore (`BUNDLER_KEYSTORE` + `BUNDLER_KEYSTORE_PASSWORD_FILE`), or `BUNDLER_PRIVATE_KEY` (dev). It is never logged.
Fees go to `BENEFICIARY` (default: the operator).

**HTTP**: per-IP token bucket for all requests (`RATE_LIMIT_RPS` / `RATE_LIMIT_BURST`) plus per-minute limits for
`eth_sendUserOperation` and `eth_estimateUserOperationGas`; at most `MAX_CONCURRENT_SIMULATIONS` simulations at once.
Behind Cloudflare set `TRUST_PROXY=cloudflare`: `CF-Connecting-IP` is honoured only from `TRUSTED_PROXIES` (cloudflared
on localhost), IPv6 clients are grouped per /64. Body ≤ `MAX_BODY_BYTES` (413), batches ≤ `MAX_BATCH`, CORS only for
`CORS_ORIGINS`, internal errors are logged with a reference and never echoed.

### Sponsorship policy (XdevsPaymaster)

Sponsorship itself is on chain: the wallet's activation op names an `XdevsPaymaster` (`0x…1007`) sponsor in
`paymasterData`, and the sponsor's deposit and rules (`active`, `openToAll`, `maxCostPerOp`, `perAccountCap`) decide.
Open-mode sponsors can be drained by anyone creating fresh accounts, so the bundler adds a policy for ops that use the
XdevsPaymaster **in open / allowlist mode** (`paymasterData` = 20-byte sponsor; signer-mode sponsors sign each op
themselves and are not affected):

- `SPONSOR_POLICY=activation` (default): an account's first sponsored op must deploy it (the wallet's "Activate"), and an
  account gets at most `SPONSOR_MAX_OPS_PER_SENDER` (default 1) sponsored ops through this bundler;
- `SPONSORED_OPS_PER_IP_PER_HOUR` (default 30; 10 in the Enclave env) caps sponsored ops per client IP;
- `SPONSOR_POLICY_SPONSORS` limits the policy to listed sponsors; `SPONSOR_POLICY=off` disables it.

Funding and staking the paymaster: [runbook §4](../../docs/runbooks/bundler.md#4-fund-the-operator-and-the-paymaster).

### Remaining limits (honest list)

- **No opcode / storage-access tracing.** The xDEVS node has no `debug_traceCall`, so the ERC-7562 opcode and storage
  rules (OP-xxx, STO-xxx except STO-040) are not checked. Validation that depends on block data or on state another op
  can change is caught only by the bundle re-simulation (op dropped) or, if it changes between re-simulation and
  inclusion, by an on-chain revert, which bans the responsible entity. The worst case is one reverted bundle (its gas)
  per entity, plus the three-failures pause.
- No alt-mempools, no P2P mempool, no aggregators, no state-override estimation, no `debug_bundler_*` methods.
- One operator key and one process; no fee bumping of a bundle stuck in the node's pool (it is waited for, then
  re-queued). Enclave's base fee sits at its 0.1 gwei floor, so this has not been an issue.
- Pipelined bundles never share an entity, so ops through one paymaster (e.g. all sponsored activations) go out one
  bundle per block, batched up to `MAX_BUNDLE_OPS` each.
- Reputation and sponsorship counters are per bundler instance (persisted in `DATA_DIR`); a second instance would keep
  its own.

## Configuration

All settings are environment variables (`deploy/bundler/bundler.env.example` is the Enclave configuration). Defaults in
`src/config.ts`.

| Variable | Default | Meaning |
|---|---|---|
| `RPC_URL` | `http://127.0.0.1:8545` | Node JSON-RPC (prefer one without public rate limits) |
| `CHAIN_ID` | not checked | Refuse to start if the RPC reports another chain |
| `HOST`, `PORT` | `127.0.0.1`, `4337` | Listen address |
| `ENTRY_POINT` | v0.9 canonical | |
| `BENEFICIARY` | operator | Receives bundle fees |
| `BUNDLER_PRIVATE_KEY_FILE` / `BUNDLER_KEYSTORE` (+ `BUNDLER_KEYSTORE_PASSWORD_FILE` / `_PASSWORD`) / `BUNDLER_PRIVATE_KEY` | | Operator key (first match; `$CREDENTIALS_DIRECTORY/operator-key` and `/keystore-password` are picked up automatically) |
| `OPERATOR_MIN_BALANCE` | `0.01` tXDS | Floor: below it no ops are accepted and no bundles sent |
| `OPERATOR_LOW_BALANCE` | `0.5` tXDS | `bundler_operator_balance_low` alarm threshold |
| `MIN_PRIORITY_FEE_PER_GAS` | `1000000` wei | Minimum tip |
| `BASE_FEE_MARGIN_PCT` | `100` | `maxFeePerGas` ≥ base fee × pct / 100 + minimum tip |
| `MAX_OP_BYTES` | `16384` | Encoded op size |
| `MAX_VERIFICATION_GAS`, `MAX_CALL_GAS` | `3000000`, `10000000` | Per-op gas caps |
| `MIN_VALID_UNTIL_SECONDS` | `30` | Refuse ops expiring sooner |
| `REQUIRE_PAYMASTER_STAKE`, `MIN_STAKE`, `MIN_UNSTAKE_DELAY` | `true`, `1` tXDS, `86400` s | Paymaster stake rule; also what "staked" means for limits |
| `MAX_MEMPOOL_OPS` | `1000` | Mempool capacity |
| `SAME_SENDER_MEMPOOL_COUNT`, `SAME_UNSTAKED_ENTITY_MEMPOOL_COUNT`, `THROTTLED_ENTITY_MEMPOOL_COUNT`, `THROTTLED_ENTITY_BUNDLE_COUNT`, `THROTTLED_ENTITY_LIVE_BLOCKS` | `4`, `10`, `4`, `4`, `10` | ERC-7562 limits |
| `MEMPOOL_TTL_SECONDS` | `1800` | Pending ops older than this are dropped |
| `MIN_INCLUSION_RATE_DENOMINATOR`, `THROTTLING_SLACK`, `BAN_SLACK` | `10`, `10`, `50` | ERC-7562 reputation |
| `REPUTATION_ALLOWLIST` | empty | Comma-separated entities never throttled / banned |
| `BUNDLE_INTERVAL_MS`, `BUNDLE_COLLECT_MS` | `1000`, `150` | Bundling cadence (`0` interval = manual, tests) |
| `MAX_BUNDLE_OPS`, `MAX_BUNDLE_GAS`, `MAX_INFLIGHT_BUNDLES` | `16`, `15000000`, `2` | Bundle size and pipelining |
| `RECEIPT_TIMEOUT_MS` | `120000` | Wait per receipt round |
| `SPONSOR_POLICY`, `SPONSOR_PAYMASTER`, `SPONSOR_POLICY_SPONSORS`, `SPONSOR_MAX_OPS_PER_SENDER`, `SPONSORED_OPS_PER_IP_PER_HOUR` | `activation`, `0x…1007`, all, `1`, `30` | Sponsorship policy |
| `TRUST_PROXY`, `TRUSTED_PROXIES` | `none`, loopback | `cloudflare` / `xff` client-IP headers, honoured only from these peers |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
| `MAX_BODY_BYTES`, `MAX_BATCH` | `131072`, `10` | Request limits |
| `RATE_LIMIT_RPS`, `RATE_LIMIT_BURST`, `SEND_PER_MINUTE`, `ESTIMATE_PER_MINUTE` | `20`, `60`, `30`, `60` | Per-IP rate limits |
| `MAX_CONCURRENT_SIMULATIONS` | `4` | Simulations running at once |
| `METRICS_ACCESS` | `local` | `local`: `/metrics` only for direct loopback scrapes; `all` |
| `DATA_DIR` | none (memory only) | State file `bundler-state.json` (mempool, reputation, sponsorship counters) |
| `LOG_LEVEL`, `QUIET` | `info` | JSON-lines logs on stdout / stderr; `QUIET` = silent |

### Metrics

`bundler_mempool_ops{status}`, `bundler_inflight_bundles`, `bundler_bundles_submitted_total`,
`bundler_bundles_included_total`, `bundler_bundles_failed_total{stage=submit|reverted|dropped}`,
`bundler_ops_received_total{result}`, `bundler_ops_rejected_total{code}`, `bundler_ops_included_total`,
`bundler_ops_dropped_total`, `bundler_operator_balance_wei`, `bundler_operator_balance_low`,
`bundler_operator_balance_below_floor`, `bundler_fees_collected_wei_total`, `bundler_gas_spent_wei_total`,
`bundler_reputation_entities{status}`, `bundler_head_block`, `bundler_rpc_up`, `bundler_rpc_requests_total{method}`,
`bundler_rate_limited_total{bucket}`.

## How estimation works without state overrides

The xDEVS node rejects `eth_call` state overrides, so the usual `EntryPointSimulations` override trick is not
available. Instead the bundler deploys a tiny helper, `contracts/XdevsBundlerGasMeter.sol` (CREATE2 through
`0x4e59b448…`, which Enclave, the dev genesis and anvil all have; plain CREATE otherwise; the first start on a chain
sends this one transaction), and calls `EntryPoint.delegateAndRevert(meter, measure(op, maxCost))` in an `eth_call`. The
meter runs **inside the EntryPoint**, so every call it makes has `msg.sender == EntryPoint`: SenderCreator deploys the
account, the account validates and executes, the paymaster validates and runs `postOp`. It measures each step and
reverts with the numbers (and the `validationData` the admission check uses); nothing persists. Margins: +20 % and a
fixed amount (passkey dummy signatures skip SHA-256 / P256VERIFY work). The bytecode is committed in
`src/gas-meter-artifact.ts`; `npm run build:meter` regenerates it with Foundry.

## Run locally

```bash
cd apps/bundler && npm ci
# 1. a dev chain with the predeploys, a paymaster sponsor and a test token (see below)
npm run devnet                      # xDEVS node if its binary is found (XDEVS_BIN), else anvil
# 2. the bundler (key = a funded dev key; the devnet script prints the exact command)
BUNDLER_PRIVATE_KEY=0x5de4…365a RPC_URL=http://127.0.0.1:8545 npm start   # listens on 127.0.0.1:4337
```

### `npm run devnet` (`scripts/devnet.ts`)

- `--chain xdevs`: starts `xdevs run --dev --genesis deploy/configs/dev-genesis-with-predeploys.json --block-time 1s` in a
  temporary data dir (binary from `XDEVS_BIN`, `CARGO_TARGET_DIR`, `target/{debug,release}` or `~/.xdevs-target`).
  That genesis already contains the smart-account implementation, factory, paymaster, EntryPoint v0.9 + SenderCreator,
  XNS, Permit2Expiring, WXDS and Multicall3, and funds the 10 Anvil dev keys.
- `--chain anvil`: `anvil --hardfork osaka --chain-id 7117` (Osaka = P256VERIFY at `0x100`) with the **same genesis
  alloc** copied in (`anvil_setCode` / `anvil_setStorageAt`).
- `--rpc <url>`: use an already-running chain.
- Then: configures dev key #1 as an open XdevsPaymaster sponsor (0.05 tXDS per op cap, 20 tXDS deposit), stakes the
  paymaster, and deploys an XDRC-20 test token from `contracts/out-wallet` (built on demand with
  `forge build --root contracts --out out-wallet --cache-path cache-wallet`). Prints the wallet `.env.local`.

### Smoke test of a running bundler

`node scripts/smoke.ts --bundler <url> --rpc <rpc>`: chain id, entry point, `/health`, and an estimate of a fresh
account's activation op (read-only). `--send --sponsor 0x…` also submits that activation op, sponsored, and waits for it.

## Tests

`npm test` (needs anvil; CI installs Foundry): `XDEVS_TEST_CHAIN=xdevs|anvil|auto` (default auto) for the chain tests.

- `test/unit.test.ts`, `test/server.test.ts` (`npm run test:unit`, no chain): reputation transitions and decay,
  admission rules (nonce, replacement, limits, STO-040, deposit, capacity), bundle selection, stale ops, the
  preVerificationGas coverage invariant for bundles of 1–16 ops, bundle fee caps, fee floor, static limits,
  validationData, rate limits and client IPs, configuration (incl. the shipped env example), keystore (a Foundry `cast`
  fixture), metrics, state file; HTTP body / batch limits, per-IP and per-method rate limits, CORS, sanitized errors,
  `/health`, `/metrics`.
- `test/bundler.test.ts`: userOpHash equality with `EntryPoint.getUserOpHash`, counterfactual deployment, sponsored
  deployment, AA24 / wrong EntryPoint / execution-revert errors, receipt and by-hash lookup from logs by a fresh instance.
- `test/e2e.test.ts`: four senders in **one** bundle (beneficiary gain ≥ operator spend, per-op log slices); chained
  same-sender ops and nonce gaps; a paymaster (`test/contracts/TestTogglePaymaster.sol`) **throttled** after its ops are
  invalidated post-admission; the same kind of paymaster **banned** after a front-running transaction makes a bundle
  revert on chain; unstaked paymaster refused; sponsorship policy; restart with the persisted mempool and reputation.

`npm run typecheck` — TypeScript.
