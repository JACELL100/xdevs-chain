# shellcheck shell=bash
# Shared helpers of the localnet scripts. Sourced, never executed:
#   source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
#
# Needs bash >= 4 (associative arrays), docker with the compose plugin (v2), and jq.
#
# Environment (all optional):
#   XDEVS_IMAGE         image of every service           (default xdevs:local)
#   XDEVS_PROJECT       compose project name             (default xdevs-localnet)
#   XDEVS_NETWORK       docker network of the localnet   (default xdevs-localnet)
#   XDEVS_BASE_PORT     first port of node0, as given to `xdevs-cli testnet init` (default 30300)
#   XDEVS_BLOCK_TIME    target block time, e.g. 1s / 500ms (default 1s)
#   XDEVS_COMPOSE_FILE  compose file                     (default deploy/local/docker-compose.yml)

XDEVS_REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
XDEVS_COMPOSE_FILE=${XDEVS_COMPOSE_FILE:-$XDEVS_REPO_ROOT/deploy/local/docker-compose.yml}
XDEVS_PROJECT=${XDEVS_PROJECT:-xdevs-localnet}
XDEVS_NETWORK=${XDEVS_NETWORK:-xdevs-localnet}
XDEVS_BASE_PORT=${XDEVS_BASE_PORT:-30300}
XDEVS_BLOCK_TIME=${XDEVS_BLOCK_TIME:-1s}
export XDEVS_IMAGE=${XDEVS_IMAGE:-xdevs:local}
export XDEVS_BLOCK_TIME

# Compose services of the localnet (generator directories node0..node4).
# shellcheck disable=SC2034  # used by the scripts that source this file
XDEVS_VALIDATORS=(node0 node1 node2 node3)
# shellcheck disable=SC2034
XDEVS_JOINER=node4

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }

# die MESSAGE [EXIT_CODE]
die() {
  log "error: $1"
  exit "${2:-3}"
}

need() {
  local c
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || die "missing required command: $c"
  done
}

# docker compose on the localnet project, with every profile enabled so `ps`, `logs`,
# `stop` and `down` see all services. `up` is always given explicit service names.
compose() {
  docker compose -f "$XDEVS_COMPOSE_FILE" -p "$XDEVS_PROJECT" \
    --profile validators --profile joiner --profile loadgen "$@"
}

# Container id of a compose service (running or not); empty if it was never created.
container_id() {
  compose ps -a -q "$1" 2>/dev/null | head -n1
}

# node_index nodeK -> K
node_index() {
  [[ $1 =~ ^node([0-9]+)$ ]] || die "not a localnet node name: $1"
  printf '%s\n' "${BASH_REMATCH[1]}"
}

# JSON-RPC port of nodeK inside its container (base + 10k + 2, see xdevs-cli testnet init).
node_rpc_port() {
  local k
  k=$(node_index "$1")
  printf '%s\n' $((XDEVS_BASE_PORT + 10 * k + 2))
}

# Host port the compose file publishes nodeK's RPC on (127.0.0.1:8545 + 10k).
node_host_rpc_port() {
  local k
  k=$(node_index "$1")
  printf '%s\n' $((8545 + 10 * k))
}

# duration_secs 90 | 90s | 15m | 6h | 1h30m -> seconds
duration_secs() {
  local s=$1 total=0 n unit
  [[ $s =~ ^[0-9]+$ ]] && { printf '%s\n' "$s"; return; }
  [[ $s =~ ^([0-9]+[smh])+$ ]] || die "invalid duration: $s (use e.g. 90, 90s, 15m, 6h, 1h30m)"
  while [[ $s =~ ^([0-9]+)([smh])(.*)$ ]]; do
    n=${BASH_REMATCH[1]} unit=${BASH_REMATCH[2]} s=${BASH_REMATCH[3]}
    case $unit in
      s) total=$((total + n)) ;;
      m) total=$((total + 60 * n)) ;;
      h) total=$((total + 3600 * n)) ;;
    esac
  done
  printf '%s\n' "$total"
}

# Block time in whole seconds, rounded up (>= 1): scales the chaos windows.
block_time_secs() {
  local bt=$XDEVS_BLOCK_TIME ms
  if [[ $bt =~ ^([0-9]+)ms$ ]]; then
    ms=${BASH_REMATCH[1]}
  elif [[ $bt =~ ^([0-9]+)(\.([0-9]{1,3}))?s?$ ]]; then
    local frac=${BASH_REMATCH[3]:-0}
    while ((${#frac} < 3)); do frac="${frac}0"; done
    ms=$((BASH_REMATCH[1] * 1000 + 10#$frac))
  else
    die "invalid XDEVS_BLOCK_TIME: $bt"
  fi
  printf '%s\n' $(((ms + 999) / 1000 > 0 ? (ms + 999) / 1000 : 1))
}

# Health of a compose service: healthy | unhealthy | starting | running | exited | missing.
service_health() {
  local cid
  cid=$(container_id "$1")
  [[ -n $cid ]] || { echo missing; return; }
  docker inspect -f '{{if eq .State.Status "running"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}running{{end}}{{else}}{{.State.Status}}{{end}}' "$cid" 2>/dev/null || echo missing
}

# wait_healthy TIMEOUT_SECS SERVICE...
wait_healthy() {
  local timeout=$1 deadline s h pending
  shift
  deadline=$((SECONDS + timeout))
  while :; do
    pending=()
    for s in "$@"; do
      h=$(service_health "$s")
      case $h in
        healthy) ;;
        exited | dead | missing)
          compose logs --no-color --tail 50 "$s" >&2 || true
          die "$s is $h"
          ;;
        *) pending+=("$s:$h") ;;
      esac
    done
    ((${#pending[@]} == 0)) && return 0
    ((SECONDS >= deadline)) && die "timeout after ${timeout}s waiting for: ${pending[*]}" 2
    sleep 2
  done
}

# dump_logs DIR: compose state and one log file per service (for CI artifacts).
dump_logs() {
  local dir=$1 s
  mkdir -p "$dir"
  compose ps -a >"$dir/ps.txt" 2>&1 || true
  docker network ls >"$dir/networks.txt" 2>&1 || true
  for s in $(compose config --services 2>/dev/null); do
    compose logs --no-color --timestamps "$s" >"$dir/$s.log" 2>&1 || true
    [[ -s $dir/$s.log ]] || rm -f "$dir/$s.log"
  done
  log "logs written to $dir"
}
