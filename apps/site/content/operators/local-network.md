# Local multi-validator network

You can run a real BFT network of several validators on your own machine, either as plain processes or in Docker with chaos tooling. This is how Phase 2 consensus is developed and tested.

<span class="status ok">Built (Phase 2)</span> · Static validator set from the genesis file · Dev and test use only

:::danger[Dev keys]
`testnet init` writes **unencrypted consensus keys** by default, and the genesis funds the public Anvil dev accounts. Use these networks for development and testing only.
:::

## Four validators as processes

```bash
cargo build

# Keys, one shared genesis.json and node0..node3/config.toml
./target/debug/xdevs-cli testnet init --validators 4 --out ./testnet

# One terminal per validator
./target/debug/xdevs run --config ./testnet/node0/config.toml
./target/debug/xdevs run --config ./testnet/node1/config.toml
./target/debug/xdevs run --config ./testnet/node2/config.toml
./target/debug/xdevs run --config ./testnet/node3/config.toml

cast block-number --rpc-url http://127.0.0.1:30332
```

All four validators must be up before the first block: one validator holds only 1/4 of the voting power.

### Ports

Node *k* uses ports `base + 10k` (default base `30300`):

| Offset | Use | node0 | node1 | node2 | node3 |
|---|---|---|---|---|---|
| +0 | Consensus P2P | 30300 | 30310 | 30320 | 30330 |
| +1 | Transaction gossip | 30301 | 30311 | 30321 | 30331 |
| +2 | JSON-RPC (HTTP + WS) | 30302 | 30312 | 30322 | 30332 |
| +3 | Metrics (`[metrics] listen`) | 30303 | 30313 | 30323 | 30333 |

### `testnet init` options

| Option | Default | Meaning |
|---|---|---|
| `--validators N` | required | Number of validators (equal voting power) |
| `--out DIR` | required | Output directory (must be empty unless `--force`) |
| `--chain-id ID` | `7117` | Chain id |
| `--block-time D` | `2s` | Target block time |
| `--base-port P` | `30300` | First port; node *k* uses `P + 10k` … `P + 10k + 3` |
| `--host H` | `127.0.0.1` | Host the nodes listen on and reach each other at |
| `--full-nodes M` | `0` | Extra full nodes (no consensus key) |
| `--password` / `--password-file FILE` | none | Encrypt the consensus keys; configs point `[validator] password_file` at the file |
| `--light-kdf` | off | Cheap scrypt parameters for encrypted keys |
| `--force` | off | Replace an existing testnet in `--out` |

Each node directory contains `consensus_key.hex` (or `consensus_key.json` when encrypted), `node_key.json` (libp2p identity, same peer id on both networks), `config.toml` and, once running, `data/`. Every node lists every other node as a consensus peer and a gossip bootnode.

Single keys can be generated with:

```bash
xdevs-cli keygen --consensus --out consensus_key.json   # Ed25519 validator key (encrypted keystore)
xdevs-cli keygen --node-key --out node_key.json         # libp2p node identity
```

## What the network guarantees

With four equal validators:

| Situation | Voting power online | Expected |
|---|---|---|
| All 4 up | 4/4 | Progress |
| 1 stopped, killed, paused or isolated | 3/4 > 2/3 | Progress; the node catches up when it returns |
| 2 stopped, or a 2–2 partition | 2/4 < 2/3 | **Halt**, then resume after recovery |
| Delay or loss on some links | 4/4 | Progress, possibly with more rounds |
| Any situation | | **Never a fork**: same block hash and state root at every height on every node |

A new full node started late syncs from genesis and follows the head. Each validator keeps a persistent **double-sign guard**, so a restarted validator never signs two conflicting votes.

## Docker localnet

The Docker setup runs 4 validators, an optional late-joining full node and the load generator, with scripts to inject faults. It needs Linux or macOS with Docker Engine 25+ (on Windows, WSL 2 with Docker Desktop), bash 4+ and `jq`.

```bash
scripts/localnet-up.sh --build             # build the image, start node0-3, wait for healthy
scripts/check.sh --wait-height 10          # all nodes at >= 10, same hashes and state roots
cast block-number --rpc-url http://127.0.0.1:8545   # node0
scripts/chaos.sh start node4               # late joiner, syncs from genesis
scripts/chaos.sh kill node1                # crash a validator
scripts/chaos.sh partition node0,node1 node2,node3   # the chain halts
scripts/chaos.sh heal                      # and resumes
scripts/localnet-down.sh                   # stop and wipe
```

The first image build takes 15–30 minutes and about 5 GB of disk. To skip it, use the image CI publishes: `export XDEVS_IMAGE=ghcr.io/jacell100/xdevs-chain:main`. In Docker, node RPCs are published on `127.0.0.1:8545`, `:8555`, `:8565`, `:8575` (and `:8585` for the joiner).

CI runs the same scenarios on every pull request: stop one validator (chain continues), stop two (chain halts), restart (chain resumes), kill -9, pause, isolate, partition, network delay and loss, and a late joiner, followed by a deep fork scan. A separate soak workflow runs the network under load for up to 5.5 hours.

Full instructions: [localnet runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/localnet.md) · [soak runbook](https://github.com/JACELL100/xdevs-chain/blob/main/docs/runbooks/soak.md).

## Limits today

- The validator set is fixed in `genesis.json`. Staking-driven validator changes, rewards and slashing are Phase 3.
- Peers are static (`persistent_peers`, `bootnodes`); Kademlia discovery is off.
- P2P runs over TCP; QUIC is planned after a Linux soak.
