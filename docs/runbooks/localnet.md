# Runbook: Docker localnet and chaos testing

A 4-validator xDEVS Chain network (plus an optional late full node) in Docker, with scripts to
inject faults and check that the chain stays safe and live. CI runs the same scenarios on every
PR (`.github/workflows/localnet.yml`, job `localnet-e2e`). Background: research doc
`docs/research/phase2-consensus.md` §6.12.

DEV / TESTNET ONLY: the consensus keys sit unencrypted in a Docker volume, and the dev accounts
use the public Anvil keys.

## Requirements

- Linux or macOS with Docker Engine 25+ and the compose plugin v2 (`docker compose version`). On
  Windows, use WSL 2 with Docker Desktop. The scripts need bash 4+ (macOS: `brew install bash`).
- `jq`. `curl` is only needed for `check.sh --via http`.
- A first image build takes 15 to 30 minutes and about 5 GB of disk. To skip it, use the image
  that CI publishes: `export XDEVS_IMAGE=ghcr.io/jacell100/xdevs-chain:main`.

## Layout

| Service | Profile | Role | RPC on the host |
|---|---|---|---|
| `init` | validators, joiner | one-shot: `xdevs-cli testnet init` into the `localnet` volume | none |
| `node0` to `node3` | `validators` | genesis validators (equal power) | `127.0.0.1:8545`, `:8555`, `:8565`, `:8575` |
| `node4` | `joiner` | full node, starts late and syncs from genesis | `127.0.0.1:8585` |
| `loadgen` | `loadgen` | `xdevs-loadgen` (used by `soak.sh`) | none |
| `dev` | `dev` | Phase-1 single-node dev chain | `127.0.0.1:8545` |

Inside the `xdevs-localnet` network, node k listens on `30300 + 10k` (consensus p2p), `+1` (tx
gossip), `+2` (JSON-RPC, HTTP + WS) and `+3` (Prometheus metrics). Nodes reach each other by
service name.

**How addresses are generated.** `xdevs-cli testnet init --host H` gives every node the same host
`H`. Docker needs one host per container. `scripts/localnet-init.sh` therefore generates with a
placeholder DNS name and rewrites each `/dns4/<placeholder>/tcp/<port>` address to the service
that owns the port (ports are unique per node). A DNS `--host` makes the generator bind to
`0.0.0.0`. The script also drops the joiner from the validators' peer lists, so `node4` dials the
validators the way a real new node would. We chose this over host networking because
`docker network disconnect` partitions and per-container `tc netem` both need one network
namespace per node.

The genesis, keys, block time and chain id are created once per volume. `localnet-down.sh` wipes
the volume, and `localnet-up.sh --fresh` regenerates everything.

## Everyday use

```bash
scripts/localnet-up.sh --build             # build xdevs:local, start node0-3, wait for healthy
scripts/check.sh --wait-height 10          # all nodes at >= 10, same hashes and state roots
cast block-number --rpc-url http://127.0.0.1:8545   # any Ethereum tool works against node0
scripts/chaos.sh start node4               # late joiner
scripts/chaos.sh status                    # containers, networks, heads
docker compose -f deploy/local/docker-compose.yml -p xdevs-localnet logs -f node2
scripts/localnet-down.sh                   # stop and wipe (--keep-data keeps the chain)
```

Settings (environment variables): `XDEVS_IMAGE` (default `xdevs:local`), `XDEVS_BLOCK_TIME`
(default `1s`, applied when the volume is first created), `XDEVS_CHAIN_ID` (default 7117),
`XDEVS_LOG` (`RUST_LOG` filter, default `info`) and `XDEVS_RESTART` (restart policy, default
`no` so that chaos tests stay deterministic).

Metrics: `docker exec xdevs-localnet-node0-1 curl -s 127.0.0.1:30303/metrics`.

## Chaos commands

| Command | Fault |
|---|---|
| `chaos.sh stop node3` | graceful stop (SIGINT, like Ctrl-C) |
| `chaos.sh kill node1` | SIGKILL: a crash with no graceful shutdown. The node re-verifies its head on restart |
| `chaos.sh start node1 node3` | start stopped or killed nodes; `start node4` creates the joiner |
| `chaos.sh restart node2` | graceful restart |
| `chaos.sh pause node2` / `unpause node2` | freeze every process (a GC stall or a hung VM) |
| `chaos.sh isolate node3` | disconnect the node from every network |
| `chaos.sh partition node0,node1 node2,node3` | split into groups that only see each other. The first group and any unlisted nodes stay on the main network |
| `chaos.sh heal` | undo every `isolate` and `partition` |
| `chaos.sh netem node0 "delay 200ms 50ms loss 5%"` | `tc netem` on every interface of the node |
| `chaos.sh netem-clear node0` | remove netem |

`netem` runs `tc` in a short-lived sidecar with `NET_ADMIN` that joins the node's network
namespace (`docker run --network container:<node>`). The node image does not need iproute2 or
extra capabilities. The sidecar image (`alpine` + `iproute2`, tag `xdevs-netem:local`) is built
on first use.

While a node is isolated or partitioned, its published host port may not answer. `check.sh`
queries nodes through `docker exec` by default, so it is not affected.

## What the chain must do (4 equal validators)

| Situation | Voting power online | Expected |
|---|---|---|
| all 4 up | 4/4 | progress |
| 1 stopped, killed, paused or isolated | 3/4 > 2/3 | progress; the node catches up when it returns |
| 2 stopped, or a 2\|2 partition | 2/4 < 2/3 | **halt** (no block for 10+ block times), then resume after recovery |
| delay or loss on some links | 4/4 | progress, possibly with more rounds |
| any situation | | **never** a fork: same block hash and state root at every height on every node |

## check.sh

```bash
scripts/check.sh                                   # running nodes: reachable, no fork
scripts/check.sh --expect progress --window 10     # every node advances within 10 s
scripts/check.sh --nodes "node0 node1" --expect halt --window 15
scripts/check.sh --wait-height 100 --timeout 300 --max-lag 3
scripts/check.sh --samples 50 --depth 10 --logs    # deep scan + safety alerts in the logs
```

check.sh compares `hash` and `stateRoot` from `eth_getBlockByNumber` at genesis, at the common
height (the lowest head) and the `--depth` heights below it, and at `--samples` heights spread
over the chain. It also checks that each `xdevs_getCommitCertificate` points at that block.
`--logs` counts WARN/ERROR lines containing `sign guard:`, `equivocation evidence` or `FORK:`
(the same classification as the in-process harness, `crates/testutils/src/alerts.rs`), plus
`consensus halted` and `panicked at`.

Output is one JSON line on stdout. Exit codes: `0` ok, `1` safety (fork, root or certificate
mismatch, safety log alert), `2` liveness, `3` unreachable node or usage error. To check nodes
outside Docker, list them with `XDEVS_RPC_URLS="v1=http://10.0.0.11:8545 v2=..."`.

## The CI scenario run

`scripts/localnet-e2e.sh [core] [extended] [joiner]` starts a fresh localnet and runs:

- **core:** the 4 validators reach height 20. Stop 1, and the chain keeps going. Stop a 2nd, and
  it halts. Start both, and it resumes with no fork.
- **extended:** kill -9 and restart; pause and unpause; isolate one validator and heal; a 2|2
  partition (halts) and heal; 150 ms ± 50 ms delay with 2 % loss on two validators.
- **joiner:** `node4` syncs from genesis, follows the head within 3 blocks, and agrees on every
  checked hash.

The run ends with a deep fork scan and a log scan. Set `XDEVS_E2E_ARTIFACTS=dir` to collect each
check's JSON and every service log when a step fails. CI uploads these as the
`localnet-e2e-logs` artifact.

## Troubleshooting

- **`init` fails:** run `docker compose -f deploy/local/docker-compose.yml -p xdevs-localnet logs init`.
  A volume left from an older run is reused as is, so run `scripts/localnet-down.sh` first.
- **Nodes are healthy but the height stays at 0:** check that the nodes found each other:
  `docker compose ... logs node0 | grep -i -E "peer|dial"`. All four validators must be up for
  the first block, since one validator has only 1/4 of the power.
- **A node exits right after a kill:** look for `consensus halted` or a head-verification error
  in its log. Either one is a real bug, so keep the logs.
- **Port 8545 is busy:** something else (Anvil, or the `dev` profile) is listening there. Stop it,
  or edit the published ports.
