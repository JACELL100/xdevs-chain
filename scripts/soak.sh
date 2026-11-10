#!/usr/bin/env bash
# soak.sh: continuous load against a RUNNING network plus periodic safety / liveness
# checks; writes a JSON summary. Runbook: docs/runbooks/soak.md.
#
#   scripts/localnet-up.sh && scripts/soak.sh --duration 30m
#   scripts/soak.sh --duration 5h30m --tps 50 --artifacts out/soak     # CI soak workflow
#   XDEVS_RPC_URLS="v1=http://10.0.0.11:8545 v2=http://10.0.0.12:8545 v3=... v4=..." \
#     scripts/soak.sh --mode host --duration 24h --loadgen-rpc http://10.0.0.11:8545 \
#       --log-cmd 'for h in v1 v2 v3 v4; do ssh $h journalctl -u xdevs --since "-25h" -o cat; done'
#
# Load: xdevs-loadgen (--tps / --senders). If it fails within 15 s (e.g. not built yet),
# the fallback is one `xdevs-cli send --wait` loop per dev account 5..9 (about 5 tx per
# block): the run continues and the summary says which load ran.
# Docker mode runs both inside the `loadgen` compose service; host mode runs the binaries
# from PATH (XDEVS_LOADGEN_BIN / XDEVS_CLI_BIN override).
#
# Every --poll s: the highest head (stall tracking). Every --interval s: check.sh
# --expect progress (fork + state-root check included). At the end: a deep fork check over
# the whole chain and a scan of the node logs for safety alerts.
#
# PASS = zero forks, zero state-root / certificate mismatches, zero safety log alerts,
#        no consensus halt or panic, longest stall <= --stall-limit, the final check
#        passed, and the load ran (unless --no-load).
# Exit 0 on PASS, 1 otherwise. The summary is written either way.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

usage() {
  cat <<'EOF'
usage: soak.sh [options]
  --duration D        how long to run: 90s, 30m, 6h, 1h30m (default 30m)
  --interval S        seconds between full checks (default 60)
  --poll S            seconds between head polls for stall tracking (default 5)
  --window S          progress window of each full check (default 10)
  --stall-limit S     longest tolerated time without a new block (default 30)
  --tps N             loadgen target TPS (default 20)
  --senders N         loadgen sender accounts (default 10)
  --no-load           checks only, no load
  --nodes "A B"       nodes to check (default: every running localnet node / XDEVS_RPC_URLS)
  --mode docker|host  docker: localnet + `loadgen` service; host: RPC URLs (XDEVS_RPC_URLS)
                      and binaries on PATH (default docker)
  --loadgen-rpc URL   RPC endpoint the load is sent to (default node0)
  --log-cmd CMD       host mode: shell command printing the node logs to scan at the end
  --artifacts DIR     output directory (default soak-<UTC timestamp>)
  --out FILE          summary file (default DIR/summary.json)
  --max-consecutive N abort after N failed checks in a row (default 5)
  --final-samples N   heights spread over the chain in the final fork check (default 50)
EOF
}

DURATION=30m INTERVAL=60 POLL=5 WINDOW=10 STALL_LIMIT=30 TPS=20 SENDERS=10 NO_LOAD=0
NODES="" MODE=docker LOADGEN_RPC="" LOG_CMD="" ART="" OUT="" MAX_CONSEC=5 FINAL_SAMPLES=50
while (($#)); do
  case $1 in
    --duration) DURATION=$2; shift 2 ;;
    --interval) INTERVAL=$2; shift 2 ;;
    --poll) POLL=$2; shift 2 ;;
    --window) WINDOW=$2; shift 2 ;;
    --stall-limit) STALL_LIMIT=$2; shift 2 ;;
    --tps) TPS=$2; shift 2 ;;
    --senders) SENDERS=$2; shift 2 ;;
    --no-load) NO_LOAD=1; shift ;;
    --nodes) NODES=$2; shift 2 ;;
    --mode) MODE=$2; shift 2 ;;
    --loadgen-rpc) LOADGEN_RPC=$2; shift 2 ;;
    --log-cmd) LOG_CMD=$2; shift 2 ;;
    --artifacts) ART=$2; shift 2 ;;
    --out) OUT=$2; shift 2 ;;
    --max-consecutive) MAX_CONSEC=$2; shift 2 ;;
    --final-samples) FINAL_SAMPLES=$2; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done
for v in "$INTERVAL" "$POLL" "$WINDOW" "$STALL_LIMIT" "$TPS" "$SENDERS" "$MAX_CONSEC" "$FINAL_SAMPLES"; do
  [[ $v =~ ^[0-9]+$ ]] || die "not a non-negative integer: $v"
done
DUR=$(duration_secs "$DURATION")
((DUR > 0)) || die "--duration must be > 0"
need jq
case $MODE in
  docker) need docker ;;
  host) [[ -n ${XDEVS_RPC_URLS:-} ]] || die "--mode host needs XDEVS_RPC_URLS (name=url ...)" ;;
  *) die "--mode must be docker or host" ;;
esac
if [[ -z $LOADGEN_RPC ]]; then
  if [[ $MODE == docker ]]; then
    LOADGEN_RPC="http://node0:$(node_rpc_port node0)"
  else
    first=${XDEVS_RPC_URLS%% *}
    LOADGEN_RPC=${first#*=}
  fi
fi

ART=${ART:-soak-$(date -u +%Y%m%dT%H%M%SZ)}
mkdir -p "$ART"
OUT=${OUT:-$ART/summary.json}
CHECK=(bash "$SCRIPT_DIR/check.sh")
[[ -n $NODES ]] && CHECK+=(--nodes "$NODES")
[[ $MODE == host ]] && CHECK+=(--via http)

# Public Anvil dev keys (accounts 5..9 of the dev genesis), used only by the fallback load.
FALLBACK_KEYS="0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba \
0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e \
0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356 \
0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97 \
0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6"
# Recipient: dev account 0.
FALLBACK_TO=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
# shellcheck disable=SC2016  # expanded by the inner shell
FALLBACK_SCRIPT='
end=$((SECONDS + DUR))
send_loop() {
  ok=0 err=0
  while [ "$SECONDS" -lt "$end" ]; do
    if "$CLI" send --rpc "$RPC" --private-key "$1" --to "$TO" --value 0.000001 --wait >/dev/null 2>&1; then
      ok=$((ok + 1))
    else
      err=$((err + 1)); sleep 1
    fi
  done
  echo "fallback-sender sent=$ok failed=$err"
}
for k in $KEYS; do send_loop "$k" & done
wait'

# ---- start -----------------------------------------------------------------------------------
log "soak: ${DUR}s, mode $MODE, load -> $LOADGEN_RPC, artifacts in $ART"
first_json=$("${CHECK[@]}" --wait-height 1 --timeout 120 2>"$ART/check-start.log") ||
  die "the network is not healthy before the soak: $first_json" 1
START_H=$(jq -r '.max_height' <<<"$first_json")
START_TS=$(date +%s)
START_ISO=$(date -u +%Y-%m-%dT%H:%M:%SZ)
END_TS=$((START_TS + DUR))

LOAD_MODE=none LOAD_PID="" LOAD_RC=""
start_loadgen() {
  if [[ $MODE == docker ]]; then
    compose run --rm --no-deps -T loadgen --rpc "$LOADGEN_RPC" --senders "$SENDERS" \
      --tps "$TPS" --duration "${DUR}s" >"$ART/loadgen.log" 2>&1 &
  else
    "${XDEVS_LOADGEN_BIN:-xdevs-loadgen}" --rpc "$LOADGEN_RPC" --senders "$SENDERS" \
      --tps "$TPS" --duration "${DUR}s" >"$ART/loadgen.log" 2>&1 &
  fi
  LOAD_PID=$!
  LOAD_MODE=loadgen
}
start_fallback() {
  local left=$((END_TS - $(date +%s)))
  ((left > 0)) || left=1
  if [[ $MODE == docker ]]; then
    compose run --rm --no-deps -T -e "RPC=$LOADGEN_RPC" -e "DUR=$left" -e "KEYS=$FALLBACK_KEYS" \
      -e "TO=$FALLBACK_TO" -e CLI=xdevs-cli --entrypoint bash loadgen -c "$FALLBACK_SCRIPT" \
      >"$ART/load-fallback.log" 2>&1 &
  else
    RPC=$LOADGEN_RPC DUR=$left KEYS=$FALLBACK_KEYS TO=$FALLBACK_TO CLI=${XDEVS_CLI_BIN:-xdevs-cli} \
      bash -c "$FALLBACK_SCRIPT" >"$ART/load-fallback.log" 2>&1 &
  fi
  LOAD_PID=$!
  LOAD_MODE=fallback-cli
}
if ((!NO_LOAD)); then
  start_loadgen
  sleep 15
  if ! kill -0 "$LOAD_PID" 2>/dev/null; then
    rc=0
    wait "$LOAD_PID" || rc=$?
    if ((rc != 0)); then
      log "xdevs-loadgen exited with $rc after <15 s ($(tail -n1 "$ART/loadgen.log" 2>/dev/null)); falling back to xdevs-cli transfers"
      start_fallback
    fi
  fi
fi

# ---- loop ------------------------------------------------------------------------------------
CHECKS=0 FAILED=0 CONSEC=0 FORKS=0 ROOTS=0 CERTS=0 LIVENESS_FAILS=0 DOWN_FAILS=0
LAST_H=$START_H LAST_CHANGE=$START_TS MAX_STALL=0 ABORTED="" NEXT_CHECK=$((START_TS + INTERVAL))
observe() { # observe HEIGHT: stall tracking
  local h=$1 now
  now=$(date +%s)
  if [[ $h =~ ^[0-9]+$ ]] && ((h > LAST_H)); then
    LAST_H=$h
    LAST_CHANGE=$now
  fi
  if ((now - LAST_CHANGE > MAX_STALL)); then MAX_STALL=$((now - LAST_CHANGE)); fi
}

while (($(date +%s) < END_TS)); do
  if (($(date +%s) >= NEXT_CHECK)); then
    rc=0
    out=$("${CHECK[@]}" --expect progress --window "$WINDOW" 2>>"$ART/check.log") || rc=$?
    [[ -n $out ]] || out='{}'
    printf '%s\n' "$out" >>"$ART/checks.jsonl"
    CHECKS=$((CHECKS + 1))
    FORKS=$((FORKS + $(jq -r '.forks // 0' <<<"$out")))
    ROOTS=$((ROOTS + $(jq -r '.root_mismatches // 0' <<<"$out")))
    CERTS=$((CERTS + $(jq -r '.cert_mismatches // 0' <<<"$out")))
    observe "$(jq -r '.max_height // empty' <<<"$out")"
    if ((rc == 0)); then
      CONSEC=0
    else
      FAILED=$((FAILED + 1))
      CONSEC=$((CONSEC + 1))
      case $rc in
        1) ABORTED="safety failure: $(jq -c '.failures' <<<"$out")" ;;
        2) LIVENESS_FAILS=$((LIVENESS_FAILS + 1)) ;;
        *) DOWN_FAILS=$((DOWN_FAILS + 1)) ;;
      esac
      log "check #$CHECKS failed (exit $rc): $(jq -c '.failures' <<<"$out" 2>/dev/null)"
    fi
    ((CONSEC >= MAX_CONSEC)) && ABORTED="$CONSEC consecutive failed checks"
    [[ -n $ABORTED ]] && break
    log "check #$CHECKS: head $LAST_H, longest stall ${MAX_STALL}s, failed checks $FAILED, load $LOAD_MODE"
    NEXT_CHECK=$(($(date +%s) + INTERVAL))
  else
    out=$("${CHECK[@]}" --allow-down --no-fork-check 2>/dev/null) || true
    [[ -n $out ]] || out='{}'
    observe "$(jq -r '.max_height // empty' <<<"$out")"
    sleep "$POLL"
  fi
done
[[ -n $ABORTED ]] && log "aborting the soak: $ABORTED"

# ---- stop the load ---------------------------------------------------------------------------
LOAD_OK=true LOAD_DETAIL=""
if [[ -n $LOAD_PID ]]; then
  for _ in $(seq 1 60); do kill -0 "$LOAD_PID" 2>/dev/null || break; sleep 1; done
  if kill -0 "$LOAD_PID" 2>/dev/null; then
    kill "$LOAD_PID" 2>/dev/null || true
    if [[ $MODE == docker ]]; then
      # `compose run` containers outlive a killed client.
      docker ps -q --filter "label=com.docker.compose.project=$XDEVS_PROJECT" \
        --filter "label=com.docker.compose.oneoff=True" | xargs -r docker kill >/dev/null 2>&1 || true
    fi
  fi
  LOAD_RC=0
  wait "$LOAD_PID" 2>/dev/null || LOAD_RC=$?
  if [[ $LOAD_MODE == fallback-cli ]]; then
    # Sum the "fallback-sender sent=N failed=M" lines of every sender.
    sum_field() {
      awk -v k="$1=" '{ for (i = 1; i <= NF; i++) if (index($i, k) == 1) s += substr($i, length(k) + 1) }
                      END { print s + 0 }' "$ART/load-fallback.log" 2>/dev/null || echo 0
    }
    sent=$(sum_field sent)
    errs=$(sum_field failed)
    LOAD_DETAIL="sent=$sent failed=$errs"
    ((sent > 0)) || LOAD_OK=false
  else
    LOAD_DETAIL=$(tail -n 5 "$ART/loadgen.log" 2>/dev/null | tr '\n' ' ')
    [[ $LOAD_RC == 0 || -n $ABORTED ]] || LOAD_OK=false
  fi
elif ((!NO_LOAD)); then
  LOAD_OK=false
fi
((NO_LOAD)) && LOAD_MODE=none

# ---- final deep check --------------------------------------------------------------------------
FINAL_ARGS=(--samples "$FINAL_SAMPLES" --depth 10 --allow-down)
if [[ $MODE == docker ]]; then
  FINAL_ARGS+=(--logs)
elif [[ -n $LOG_CMD ]]; then
  bash -c "$LOG_CMD" >"$ART/node-logs.txt" 2>&1 || log "warning: --log-cmd failed"
  FINAL_ARGS+=(--logs-file "$ART/node-logs.txt")
fi
FINAL_RC=0
final_json=$("${CHECK[@]}" "${FINAL_ARGS[@]}" 2>>"$ART/check.log") || FINAL_RC=$?
[[ -n $final_json ]] || final_json='{}'
FORKS=$((FORKS + $(jq -r '.forks // 0' <<<"$final_json")))
ROOTS=$((ROOTS + $(jq -r '.root_mismatches // 0' <<<"$final_json")))
CERTS=$((CERTS + $(jq -r '.cert_mismatches // 0' <<<"$final_json")))
observe "$(jq -r '.max_height // empty' <<<"$final_json")"
[[ $MODE == docker ]] && dump_logs "$ART/logs"

END_ISO=$(date -u +%Y-%m-%dT%H:%M:%SZ)
ELAPSED=$(($(date +%s) - START_TS))
jq -n \
  --arg started "$START_ISO" --arg ended "$END_ISO" \
  --argjson planned "$DUR" --argjson elapsed "$ELAPSED" \
  --arg mode "$MODE" --arg aborted "$ABORTED" \
  --argjson start_h "$START_H" --argjson end_h "$LAST_H" \
  --argjson checks "$CHECKS" --argjson failed "$FAILED" \
  --argjson liveness "$LIVENESS_FAILS" --argjson down "$DOWN_FAILS" \
  --argjson forks "$FORKS" --argjson roots "$ROOTS" --argjson certs "$CERTS" \
  --argjson max_stall "$MAX_STALL" --argjson stall_limit "$STALL_LIMIT" \
  --arg load_mode "$LOAD_MODE" --argjson load_ok "$LOAD_OK" --arg load_detail "$LOAD_DETAIL" \
  --argjson tps "$TPS" --argjson senders "$SENDERS" --arg load_rpc "$LOADGEN_RPC" \
  --argjson final "$final_json" --argjson final_rc "$FINAL_RC" \
  '($final.log_alerts // {}) as $alerts
   | {
      started: $started, ended: $ended, mode: $mode,
      planned_secs: $planned, elapsed_secs: $elapsed,
      aborted: (if $aborted == "" then null else $aborted end),
      start_height: $start_h, end_height: $end_h, blocks: ($end_h - $start_h),
      avg_block_time_secs: (if $end_h > $start_h then (($elapsed / ($end_h - $start_h)) * 1000 | round / 1000) else null end),
      checks: $checks, failed_checks: $failed,
      liveness_check_failures: $liveness, unreachable_check_failures: $down,
      forks: $forks, root_mismatches: $roots, cert_mismatches: $certs,
      max_stall_secs: $max_stall, stall_limit_secs: $stall_limit,
      log_alerts: $final.log_alerts,
      load: {mode: $load_mode, ok: $load_ok, tps: $tps, senders: $senders, rpc: $load_rpc, detail: $load_detail},
      final_check: {code: $final_rc, heights: $final.heights, checked_heights: $final.checked_heights, failures: $final.failures}
    }
   | .pass = (.aborted == null and .forks == 0 and .root_mismatches == 0 and .cert_mismatches == 0
              and (($alerts.sign_guard_refusals // 0) + ($alerts.equivocations // 0) + ($alerts.forks // 0)
                   + ($alerts.consensus_halted // 0) + ($alerts.panics // 0)) == 0
              and .max_stall_secs <= .stall_limit_secs and $final_rc == 0 and .load.ok)' >"$OUT"

if jq -e '.pass' "$OUT" >/dev/null; then
  log "soak PASSED: $(jq -c '{blocks, forks, root_mismatches, max_stall_secs, load: .load.mode}' "$OUT")"
  log "summary: $OUT"
else
  log "soak FAILED: $(jq -c '{aborted, forks, root_mismatches, cert_mismatches, max_stall_secs, log_alerts, load, final_check: .final_check.failures}' "$OUT")"
  log "summary: $OUT"
  exit 1
fi
