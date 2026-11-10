#!/usr/bin/env bash
# localnet-up.sh: start the Docker localnet (4 validators, optionally the joiner) and wait
# until every node's JSON-RPC answers.
#
#   scripts/localnet-up.sh                 # use xdevs:local (build it if missing)
#   scripts/localnet-up.sh --build         # rebuild the image first
#   scripts/localnet-up.sh --fresh         # wipe the previous localnet (new keys + genesis)
#   scripts/localnet-up.sh --joiner        # also start node4, the late full node
#   XDEVS_IMAGE=ghcr.io/jacell100/xdevs-chain:main scripts/localnet-up.sh   # prebuilt image
#   XDEVS_BLOCK_TIME=2s scripts/localnet-up.sh --fresh
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD=0 FRESH=0 JOINER=0 TIMEOUT=180
while (($#)); do
  case $1 in
    --build) BUILD=1; shift ;;
    --fresh) FRESH=1; shift ;;
    --joiner) JOINER=1; shift ;;
    --timeout) TIMEOUT=$2; shift 2 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
need docker jq

if ((FRESH)); then
  log "wiping the previous localnet"
  bash "$SCRIPT_DIR/localnet-down.sh"
fi

if ((BUILD)); then
  log "building $XDEVS_IMAGE"
  compose build init
elif ! docker image inspect "$XDEVS_IMAGE" >/dev/null 2>&1; then
  if [[ $XDEVS_IMAGE == */* ]]; then
    log "pulling $XDEVS_IMAGE"
    docker pull "$XDEVS_IMAGE"
  else
    log "$XDEVS_IMAGE not found: building it (first build takes a while)"
    compose build init
  fi
fi

services=("${XDEVS_VALIDATORS[@]}")
((JOINER)) && services+=("$XDEVS_JOINER")
log "starting ${services[*]} (image $XDEVS_IMAGE, block time $XDEVS_BLOCK_TIME)"
# `init` runs first (depends_on: service_completed_successfully) and generates the network.
compose up -d --no-build "${services[@]}"
wait_healthy "$TIMEOUT" "${services[@]}"

log "localnet is up:"
for s in "${services[@]}"; do
  log "  $s  rpc http://127.0.0.1:$(node_host_rpc_port "$s")"
done
log "next: scripts/check.sh --wait-height 5   |   scripts/chaos.sh status   |   scripts/localnet-down.sh"
