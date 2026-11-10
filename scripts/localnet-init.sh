#!/usr/bin/env bash
# Generate the Docker localnet into a shared volume. Runs INSIDE the `init` compose
# service (deploy/local/docker-compose.yml), not on the host.
#
# `xdevs-cli testnet init --host H` gives every node the same host H. In Docker each node
# is its own container, reached by its service name (node0, node1, ...). So we generate
# with a placeholder DNS name and rewrite every peer address to the owner's service
# name. Ports stay unique per node (base + 10k), so the port identifies the owner:
#
#   /dns4/xdevs-localnet-host/tcp/30310/p2p/12D3...  ->  /dns4/node1/tcp/30310/p2p/12D3...
#
# With a DNS --host the generator binds every listener to 0.0.0.0, so the RPC is
# reachable through the published ports.
#
# The late joiner (the last node, a full node) is removed from the validators' peer
# lists: like a real new node, it dials the validators, they do not know it in advance.
#
# Idempotent: an existing genesis.json is reused unless XDEVS_FORCE_INIT=1.
set -euo pipefail

OUT=${XDEVS_LOCALNET_DIR:-/localnet}
VALIDATORS=${XDEVS_VALIDATORS:-4}
FULL_NODES=${XDEVS_FULL_NODES:-1}
BASE_PORT=${XDEVS_BASE_PORT:-30300}
BLOCK_TIME=${XDEVS_BLOCK_TIME:-1s}
CHAIN_ID=${XDEVS_CHAIN_ID:-7117}
FORCE=${XDEVS_FORCE_INIT:-0}
# Must not parse as an IP address, so the generator writes /dns4/<name> multiaddrs.
PLACEHOLDER=xdevs-localnet-host

log() { printf 'localnet-init: %s\n' "$*" >&2; }

if [[ -f $OUT/genesis.json && $FORCE != 1 ]]; then
  log "$OUT/genesis.json exists: reusing the generated localnet (XDEVS_FORCE_INIT=1 regenerates)"
  exit 0
fi

total=$((VALIDATORS + FULL_NODES))
log "generating $VALIDATORS validators + $FULL_NODES full node(s), chain id $CHAIN_ID, block time $BLOCK_TIME"
mkdir -p "$OUT"
summary=$(mktemp)
force=()
if [[ -n $(ls -A "$OUT") ]]; then
  force=(--force)
fi
xdevs-cli testnet init \
  --validators "$VALIDATORS" \
  --full-nodes "$FULL_NODES" \
  --out "$OUT" \
  --host "$PLACEHOLDER" \
  --base-port "$BASE_PORT" \
  --block-time "$BLOCK_TIME" \
  --chain-id "$CHAIN_ID" \
  "${force[@]}" >"$summary"
# Written outside $OUT first: redirecting straight into $OUT would create the file before
# xdevs-cli runs, and `testnet init` refuses a non-empty directory.
mv "$summary" "$OUT/testnet-summary.txt"

# Placeholder host -> service name of the node that owns the port.
sed_args=()
for ((k = 0; k < total; k++)); do
  p2p=$((BASE_PORT + 10 * k))
  gossip=$((p2p + 1))
  sed_args+=(
    -e "s#/dns4/${PLACEHOLDER}/tcp/${p2p}/#/dns4/node${k}/tcp/${p2p}/#g"
    -e "s#/dns4/${PLACEHOLDER}/tcp/${gossip}/#/dns4/node${k}/tcp/${gossip}/#g"
  )
done
# (testnet-summary.txt keeps the generator's output as is, placeholder host included.)
files=("$OUT/genesis.json")
for ((k = 0; k < total; k++)); do
  files+=("$OUT/node${k}/config.toml")
done
sed -i "${sed_args[@]}" "${files[@]}"

if grep -l -- "$PLACEHOLDER" "${files[@]}" >&2; then
  log "error: unrewritten placeholder addresses left in the files above"
  exit 1
fi

# Validators do not list the late full nodes (one item per line in the generated TOML lists).
if ((FULL_NODES > 0)); then
  for ((v = 0; v < VALIDATORS; v++)); do
    for ((j = VALIDATORS; j < total; j++)); do
      sed -i "\#\"/dns4/node${j}/tcp/#d" "$OUT/node${v}/config.toml"
    done
  done
fi

# The init container runs as root; the nodes run as the image's `xdevs` user.
if [[ $(id -u) == 0 ]] && id xdevs >/dev/null 2>&1; then
  chown -R xdevs:xdevs "$OUT"
fi

log "done:"
for ((k = 0; k < total; k++)); do
  role=validator
  ((k >= VALIDATORS)) && role="full node (joiner)"
  log "  node${k}: ${role}, rpc :$((BASE_PORT + 10 * k + 2)), config $OUT/node${k}/config.toml"
done
