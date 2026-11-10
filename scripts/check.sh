#!/usr/bin/env bash
# check.sh: safety and liveness check of a running xDEVS network.
#
# Safety (always, unless --no-fork-check): every live node must return the same block hash
# AND state root at genesis, at the common height (min head) and a few heights below it,
# and every stored commit certificate must point at that block hash.
# Liveness (optional): --wait-height, --expect progress|halt, --max-lag.
#
#   scripts/check.sh                                   # running localnet nodes: no fork
#   scripts/check.sh --wait-height 20 --timeout 180    # wait until every node is at >= 20
#   scripts/check.sh --expect progress --window 10     # every node advances within 10 s
#   scripts/check.sh --nodes "node0 node1" --expect halt --window 15
#   scripts/check.sh --samples 50 --logs               # deep fork scan + safety log alerts
#   XDEVS_RPC_URLS="v1=http://10.0.0.11:8545 v2=http://10.0.0.12:8545" scripts/check.sh
#
# Transport: Docker localnet nodes are queried with `docker exec <node> curl` (works for
# isolated / partitioned containers, needs no published ports). With XDEVS_RPC_URLS (or
# --via http) the URLs are queried directly, e.g. the Oracle VM validators.
#
# Output: one JSON summary line on stdout; progress messages on stderr.
# Exit:   0 ok
#         1 safety: fork, state-root or certificate mismatch, safety log alert
#         2 liveness: --wait-height timeout, no progress, unexpected progress, lag, halt/panic log
#         3 a node is unreachable / cannot serve a block, or a usage error
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

usage() {
  cat <<'EOF'
usage: check.sh [options]
  --nodes "A B ..."    nodes to check (default: XDEVS_RPC_URLS names, else running localnet nodes)
  --via exec|http      transport (default: http if XDEVS_RPC_URLS is set, else exec)
  --wait-height H      first wait until every node is at height >= H (see --timeout)
  --timeout S          --wait-height timeout in seconds (default 120)
  --expect MODE        none (default) | progress (every node advances >= --min-blocks
                       within --window) | halt (the highest head advances <= 1 within --window)
  --window S           observation window of --expect, seconds (default 10)
  --min-blocks N       blocks each node must make within --window for "progress" (default 1)
  --max-lag N          fail if a node is more than N blocks behind the highest head
  --depth N            fork-check the N heights ending at the common height (default 3)
  --samples N          also fork-check N heights spread over the whole chain (default 1)
  --no-fork-check      skip the block hash / state root comparison
  --allow-down         skip unreachable nodes instead of failing
  --logs               scan the localnet logs for safety alerts (docker compose logs)
  --logs-since D       only scan logs newer than D (e.g. 5m); implies --logs
  --logs-file F        scan this log file (e.g. collected journalctl output) instead
EOF
}

NODES=()
VIA=""
EXPECT=none
WINDOW=10
MIN_BLOCKS=1
WAIT_HEIGHT=""
TIMEOUT=120
DEPTH=3
SAMPLES=1
MAX_LAG=""
FORK_CHECK=1
ALLOW_DOWN=0
LOGS=0
LOGS_SINCE=""
LOGS_FILE=""
RPC_TIMEOUT=${XDEVS_RPC_TIMEOUT:-5}

while (($#)); do
  case $1 in
    --nodes) read -r -a NODES <<<"${2//,/ }"; shift 2 ;;
    --via) VIA=$2; shift 2 ;;
    --wait-height) WAIT_HEIGHT=$2; shift 2 ;;
    --timeout) TIMEOUT=$2; shift 2 ;;
    --expect) EXPECT=$2; shift 2 ;;
    --window) WINDOW=$2; shift 2 ;;
    --min-blocks) MIN_BLOCKS=$2; shift 2 ;;
    --max-lag) MAX_LAG=$2; shift 2 ;;
    --depth) DEPTH=$2; shift 2 ;;
    --samples) SAMPLES=$2; shift 2 ;;
    --no-fork-check) FORK_CHECK=0; shift ;;
    --allow-down) ALLOW_DOWN=1; shift ;;
    --logs) LOGS=1; shift ;;
    --logs-since) LOGS=1; LOGS_SINCE=$2; shift 2 ;;
    --logs-file) LOGS_FILE=$2; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done
case $EXPECT in none | progress | halt) ;; *) die "--expect must be none, progress or halt" ;; esac
for v in "$WINDOW" "$MIN_BLOCKS" "$TIMEOUT" "$DEPTH" "$SAMPLES" ${WAIT_HEIGHT:+"$WAIT_HEIGHT"} ${MAX_LAG:+"$MAX_LAG"}; do
  [[ $v =~ ^[0-9]+$ ]] || die "not a non-negative integer: $v"
done
need jq

# ---- endpoints -------------------------------------------------------------------------
declare -A URL=() CID=() PORT=()
if [[ -n ${XDEVS_RPC_URLS:-} ]]; then
  for kv in $XDEVS_RPC_URLS; do
    [[ $kv == *=* ]] || die "XDEVS_RPC_URLS entries must be name=url, got: $kv"
    URL[${kv%%=*}]=${kv#*=}
  done
fi
if [[ -z $VIA ]]; then
  if [[ -n ${XDEVS_RPC_URLS:-} ]]; then VIA="http"; else VIA="exec"; fi
fi
case $VIA in
  http) need curl ;;
  exec) need docker ;;
  *) die "--via must be exec or http" ;;
esac

if ((${#NODES[@]} == 0)); then
  if [[ -n ${XDEVS_RPC_URLS:-} ]]; then
    for kv in $XDEVS_RPC_URLS; do NODES+=("${kv%%=*}"); done
  elif [[ $VIA == exec ]]; then
    while read -r s; do
      [[ $s =~ ^node[0-9]+$ ]] && NODES+=("$s")
    done < <(compose ps --services --status running 2>/dev/null | sort -V)
    ((${#NODES[@]})) || die "no running localnet node (start one with scripts/localnet-up.sh)"
  else
    NODES=("${XDEVS_VALIDATORS[@]}")
  fi
fi

for n in "${NODES[@]}"; do
  if [[ $VIA == http ]]; then
    [[ -n ${URL[$n]:-} ]] || URL[$n]="http://127.0.0.1:$(node_host_rpc_port "$n")"
  else
    CID[$n]=$(container_id "$n")
    PORT[$n]=$(node_rpc_port "$n")
  fi
done

TIMEOUT_CMD=()
command -v timeout >/dev/null 2>&1 && TIMEOUT_CMD=(timeout $((RPC_TIMEOUT + 10)))

# rpc NODE METHOD [PARAMS_JSON] -> the `result` as compact JSON (may be `null`); 1 on error.
rpc() {
  local node=$1 method=$2 params=${3:-[]} body out
  body=$(printf '{"jsonrpc":"2.0","id":1,"method":"%s","params":%s}' "$method" "$params")
  if [[ $VIA == http ]]; then
    out=$(curl -fsS -m "$RPC_TIMEOUT" -H 'content-type: application/json' \
      --data "$body" "${URL[$node]}" 2>/dev/null) || return 1
  else
    [[ -n ${CID[$node]:-} ]] || return 1
    out=$("${TIMEOUT_CMD[@]}" docker exec "${CID[$node]}" curl -fsS -m "$RPC_TIMEOUT" \
      -H 'content-type: application/json' --data "$body" \
      "http://127.0.0.1:${PORT[$node]}/" 2>/dev/null) || return 1
  fi
  jq -c 'if type == "object" and has("result") then .result else error("rpc error") end' \
    <<<"$out" 2>/dev/null
}

declare -A H=()
# Head of every node into H (-1 = unreachable).
sample_heads() {
  local n r
  for n in "${NODES[@]}"; do
    if r=$(rpc "$n" eth_blockNumber) && [[ $r =~ ^\"(0x[0-9a-fA-F]+)\"$ ]]; then
      H[$n]=$((BASH_REMATCH[1]))
    else
      H[$n]=-1
    fi
  done
}

heads_str() {
  local n s=""
  for n in "${NODES[@]}"; do s+="$n=${H[$n]} "; done
  printf '%s\n' "${s% }"
}

FAILS=()
SAFETY=0 LIVENESS=0 DOWN=0
fail() { # fail safety|liveness|down MESSAGE
  case $1 in
    safety) SAFETY=1 ;;
    liveness) LIVENESS=1 ;;
    down) DOWN=1 ;;
  esac
  FAILS+=("$1: $2")
  log "FAIL ($1): $2"
}

# ---- 1. wait for a height ----------------------------------------------------------------
sample_heads
if [[ -n $WAIT_HEIGHT ]]; then
  log "waiting up to ${TIMEOUT}s for height >= $WAIT_HEIGHT on ${NODES[*]}"
  deadline=$((SECONDS + TIMEOUT))
  while :; do
    ready=1
    for n in "${NODES[@]}"; do
      ((H[$n] >= WAIT_HEIGHT)) || ready=0
    done
    ((ready)) && break
    if ((SECONDS >= deadline)); then
      fail liveness "timeout after ${TIMEOUT}s waiting for height $WAIT_HEIGHT ($(heads_str))"
      break
    fi
    sleep 1
    sample_heads
  done
fi

# ---- 2. reachability -----------------------------------------------------------------------
LIVE=()
for n in "${NODES[@]}"; do
  if ((H[$n] < 0)); then
    if ((ALLOW_DOWN)); then
      log "skipping unreachable $n"
    else
      fail down "$n is unreachable"
    fi
  else
    LIVE+=("$n")
  fi
done

# ---- 3. progress / halt over a window --------------------------------------------------------
declare -A ADV=()
if [[ $EXPECT != none ]] && ((${#LIVE[@]})); then
  declare -A H0=()
  max0=-1
  for n in "${LIVE[@]}"; do
    H0[$n]=${H[$n]}
    ((H[$n] > max0)) && max0=${H[$n]}
  done
  sleep "$WINDOW"
  sample_heads
  max1=-1
  for n in "${LIVE[@]}"; do
    if ((H[$n] < 0)); then
      fail down "$n became unreachable during the ${WINDOW}s window"
      continue
    fi
    ADV[$n]=$((H[$n] - H0[$n]))
    ((H[$n] > max1)) && max1=${H[$n]}
    if [[ $EXPECT == progress ]] && ((ADV[$n] < MIN_BLOCKS)); then
      fail liveness "$n advanced ${ADV[$n]} block(s) in ${WINDOW}s, expected >= $MIN_BLOCKS (head ${H[$n]})"
    fi
  done
  if [[ $EXPECT == halt ]] && ((max1 - max0 > 1)); then
    fail liveness "chain advanced $((max1 - max0)) blocks in ${WINDOW}s (head $max0 -> $max1) but should be halted"
  fi
  log "after ${WINDOW}s: $(heads_str)"
fi

# Nodes still answering after the window.
ALIVE=()
for n in "${LIVE[@]}"; do ((H[$n] >= 0)) && ALIVE+=("$n"); done
MAX=-1 MIN=-1
for n in "${ALIVE[@]}"; do
  ((H[$n] > MAX)) && MAX=${H[$n]}
  ((MIN < 0 || H[$n] < MIN)) && MIN=${H[$n]}
done

# ---- 4. lag ----------------------------------------------------------------------------------
if [[ -n $MAX_LAG ]]; then
  for n in "${ALIVE[@]}"; do
    if ((MAX - H[$n] > MAX_LAG)); then
      fail liveness "$n is $((MAX - H[$n])) blocks behind the highest head $MAX (max lag $MAX_LAG)"
    fi
  done
fi

# ---- 5. fork / state-root / certificate check ------------------------------------------------
FORKS=0 ROOTS=0 CERTS=0
CHECKED=()
if ((FORK_CHECK)) && ((${#ALIVE[@]})) && ((MIN >= 0)); then
  heights=(0)
  for ((i = 0; i < DEPTH; i++)); do
    ((MIN - i >= 1)) && heights+=($((MIN - i)))
  done
  for ((i = 1; i <= SAMPLES; i++)); do
    s=$((MIN * i / (SAMPLES + 1)))
    ((s >= 1)) && heights+=("$s")
  done
  mapfile -t CHECKED < <(printf '%s\n' "${heights[@]}" | sort -n -u)
  for h in "${CHECKED[@]}"; do
    hex=$(printf '0x%x' "$h")
    declare -A HASH=() ROOT=()
    for n in "${ALIVE[@]}"; do
      if ! blk=$(rpc "$n" eth_getBlockByNumber "[\"$hex\",false]") || [[ $blk == null ]]; then
        fail down "$n cannot serve block $h (head ${H[$n]})"
        continue
      fi
      read -r "HASH[$n]" "ROOT[$n]" < <(jq -r '"\(.hash) \(.stateRoot)"' <<<"$blk")
      if ((h > 0)) && cert=$(rpc "$n" xdevs_getCommitCertificate "[\"$hex\"]") && [[ $cert != null ]]; then
        ch=$(jq -r '.blockHash // .block_hash // empty' <<<"$cert")
        if [[ -n $ch && ${ch,,} != "${HASH[$n],,}" ]]; then
          CERTS=$((CERTS + 1))
          fail safety "$n: commit certificate at $h is for $ch but its block is ${HASH[$n]}"
        fi
      fi
    done
    hashes=$(printf '%s\n' "${HASH[@]}" | sort -u | grep -c . || true)
    roots=$(printf '%s\n' "${ROOT[@]}" | sort -u | grep -c . || true)
    if ((roots > 1)); then
      ROOTS=$((ROOTS + 1))
      fail safety "state root mismatch at height $h: $(for n in "${!ROOT[@]}"; do printf '%s=%s ' "$n" "${ROOT[$n]}"; done)"
    fi
    if ((hashes > 1)); then
      FORKS=$((FORKS + 1))
      fail safety "FORK at height $h: $(for n in "${!HASH[@]}"; do printf '%s=%s ' "$n" "${HASH[$n]}"; done)"
    fi
    unset HASH ROOT
  done
fi

# ---- 6. safety log alerts ---------------------------------------------------------------------
ALERTS=null
if ((LOGS)) || [[ -n $LOGS_FILE ]]; then
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' EXIT
  if [[ -n $LOGS_FILE ]]; then
    sed 's/\x1b\[[0-9;]*m//g' "$LOGS_FILE" >"$tmp"
  else
    need docker
    compose logs --no-color ${LOGS_SINCE:+--since "$LOGS_SINCE"} 2>&1 | sed 's/\x1b\[[0-9;]*m//g' >"$tmp" || true
  fi
  # Same classification as crates/testutils/src/alerts.rs (WARN / ERROR events only),
  # plus a halted consensus engine and Rust panics.
  count() { grep -E '(WARN|ERROR)' "$tmp" | grep -cF -- "$1" || true; }
  sign_guard=$(count 'sign guard:')
  equivocation=$(count 'equivocation evidence')
  fork_log=$(count 'FORK:')
  halted=$(count 'consensus halted')
  panics=$(grep -cF -- 'panicked at' "$tmp" || true)
  ALERTS=$(jq -cn --argjson s "$sign_guard" --argjson e "$equivocation" --argjson f "$fork_log" \
    --argjson h "$halted" --argjson p "$panics" \
    '{sign_guard_refusals:$s, equivocations:$e, forks:$f, consensus_halted:$h, panics:$p}')
  ((sign_guard + equivocation + fork_log > 0)) &&
    fail safety "safety alerts in the node logs: $ALERTS"
  ((halted + panics > 0)) &&
    fail liveness "consensus halted / panic in the node logs: $ALERTS"
fi

# ---- summary ---------------------------------------------------------------------------------
if ((SAFETY)); then CODE=1; elif ((LIVENESS)); then CODE=2; elif ((DOWN)); then CODE=3; else CODE=0; fi
heights_json=$(for n in "${NODES[@]}"; do printf '%s\t%s\n' "$n" "${H[$n]}"; done |
  jq -Rn '[inputs | split("\t") | {(.[0]): (.[1] | tonumber | if . < 0 then null else . end)}] | add // {}')
adv_json=$(for n in "${!ADV[@]}"; do printf '%s\t%s\n' "$n" "${ADV[$n]}"; done |
  jq -Rn '[inputs | split("\t") | {(.[0]): (.[1] | tonumber)}] | add // {}')
checked_json=$(printf '%s\n' "${CHECKED[@]}" | jq -Rn '[inputs | select(length > 0) | tonumber]')
fails_json=$(printf '%s\n' "${FAILS[@]}" | jq -Rn '[inputs | select(length > 0)]')
jq -cn \
  --arg time "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson ok "$([[ $CODE == 0 ]] && echo true || echo false)" \
  --argjson code "$CODE" \
  --arg expect "$EXPECT" \
  --argjson window "$WINDOW" \
  --argjson heights "$heights_json" \
  --argjson advanced "$adv_json" \
  --argjson max "$MAX" --argjson min "$MIN" \
  --argjson checked "$checked_json" \
  --argjson forks "$FORKS" --argjson roots "$ROOTS" --argjson certs "$CERTS" \
  --argjson alerts "$ALERTS" \
  --argjson failures "$fails_json" \
  '{time:$time, ok:$ok, code:$code, heights:$heights,
    max_height:(if $max < 0 then null else $max end),
    min_height:(if $min < 0 then null else $min end),
    expect:$expect, window_secs:$window, advanced:$advanced,
    checked_heights:$checked, forks:$forks, root_mismatches:$roots, cert_mismatches:$certs,
    log_alerts:$alerts, failures:$failures}'
if ((CODE == 0)); then
  log "ok: $(heads_str)${CHECKED[*]:+; same hash + state root at heights ${CHECKED[*]}}"
fi
exit "$CODE"
