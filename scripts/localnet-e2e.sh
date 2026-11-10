#!/usr/bin/env bash
# localnet-e2e.sh: Phase-2 exit scenarios on the Docker localnet (CI job `localnet-e2e`).
# Starts a fresh 4-validator localnet, injects faults with chaos.sh and asserts with
# check.sh after every step (no fork, same state roots, progress or halt as expected).
#
#   scripts/localnet-e2e.sh                      # all scenarios: core extended joiner
#   scripts/localnet-e2e.sh core joiner          # a subset (in this order)
#   XDEVS_E2E_ARTIFACTS=out/e2e scripts/localnet-e2e.sh   # logs + check JSON on failure
#
# Scenarios
#   core      4 validators reach height 20; stop 1 -> progress; stop 2 -> halt (2/4 < 2/3);
#             start both -> progress resumes, no fork
#   extended  kill -9 + restart; pause/unpause; isolate 1 (progress) and heal; 2|2 partition
#             (halt) and heal; 150 ms +- 50 ms delay, 2 % loss on two validators
#   joiner    node4 (full node, not in the validators' peer lists) syncs from genesis and
#             follows the head
#   Every run ends with a deep fork check over all nodes plus a scan of the node logs for
#   safety alerts (sign guard / equivocation / FORK) and halts / panics.
#
# The localnet is left running for inspection; scripts/localnet-down.sh removes it.
# Set XDEVS_E2E_KEEP=0 to remove it at the end of a successful run.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

ARTIFACTS=${XDEVS_E2E_ARTIFACTS:-}
KEEP=${XDEVS_E2E_KEEP:-1}
SCENARIOS=("$@")
((${#SCENARIOS[@]})) || SCENARIOS=(core extended joiner)
for s in "${SCENARIOS[@]}"; do
  case $s in core | extended | joiner) ;; *) die "unknown scenario: $s (core, extended, joiner)" ;; esac
done
need docker jq

BT=$(block_time_secs)
V=("${XDEVS_VALIDATORS[@]}")
STEP=""
CHECKS_LOG=""
if [[ -n $ARTIFACTS ]]; then
  mkdir -p "$ARTIFACTS"
  CHECKS_LOG="$ARTIFACTS/checks.jsonl"
  : >"$CHECKS_LOG"
fi

on_exit() {
  local rc=$?
  if ((rc != 0)); then
    log "FAILED during: ${STEP:-setup} (exit $rc)"
    if [[ -n $ARTIFACTS ]]; then
      bash "$SCRIPT_DIR/check.sh" --allow-down --no-fork-check >"$ARTIFACTS/heads.json" 2>/dev/null || true
      dump_logs "$ARTIFACTS"
    fi
  elif [[ $KEEP == 0 ]]; then
    bash "$SCRIPT_DIR/localnet-down.sh"
  fi
  exit "$rc"
}
trap on_exit EXIT

step() {
  STEP=$*
  log "=== $*"
}

# check ARGS...: run check.sh, keep its JSON, fail the run on a non-zero exit.
check() {
  local out rc=0
  out=$(bash "$SCRIPT_DIR/check.sh" "$@") || rc=$?
  printf '%s\n' "$out"
  [[ -n $CHECKS_LOG ]] && printf '%s\n' "$out" >>"$CHECKS_LOG"
  if ((rc != 0)); then
    log "check.sh $* -> exit $rc"
    return "$rc"
  fi
}

chaos() { bash "$SCRIPT_DIR/chaos.sh" "$@"; }

# Highest head among NODES (reachable ones).
head_of() {
  local out
  out=$(bash "$SCRIPT_DIR/check.sh" --nodes "$*" --allow-down --no-fork-check 2>/dev/null) || true
  [[ -n $out ]] || out='{}'
  jq -r '.max_height // 0' <<<"$out"
}

# rejoin NODES...: wait until every validator is past the current head + 3 (the chain moves
# with the rejoined nodes and they caught up), then check forks across all of them.
rejoin() {
  local target
  target=$(($(head_of "${V[@]}") + 3))
  check --nodes "${V[*]}" --wait-height "$target" --timeout $((120 + 30 * BT)) --max-lag 3 >/dev/null
  check --nodes "${V[*]}" --expect progress --window $((5 * BT)) >/dev/null
}

scenario_core() {
  step "core: 4 validators reach height 20"
  check --nodes "${V[*]}" --wait-height 20 --timeout $((120 + 30 * BT)) --expect progress --window $((5 * BT)) >/dev/null

  step "core: stop 1 of 4 validators -> the other 3 keep finalizing"
  chaos stop node3
  check --nodes "node0 node1 node2" --expect progress --window $((8 * BT)) --min-blocks 3 >/dev/null

  step "core: stop a 2nd validator -> 2/4 < 2/3 voting power, the chain halts"
  chaos stop node2
  sleep $((3 * BT)) # let an in-flight height finish
  check --nodes "node0 node1" --expect halt --window $((10 * BT)) >/dev/null

  step "core: restart both -> progress resumes, no fork"
  chaos start node2 node3
  rejoin
}

scenario_extended() {
  step "extended: kill -9 a validator -> the rest progress; it restarts (head re-verified) and catches up"
  chaos kill node1
  check --nodes "node0 node2 node3" --expect progress --window $((8 * BT)) --min-blocks 3 >/dev/null
  chaos start node1
  rejoin

  step "extended: pause a validator -> the rest progress; unpause -> it catches up"
  chaos pause node2
  check --nodes "node0 node1 node3" --expect progress --window $((8 * BT)) --min-blocks 3 >/dev/null
  chaos unpause node2
  rejoin

  step "extended: isolate a validator from the network -> the rest progress; heal -> it catches up"
  chaos isolate node3
  check --nodes "node0 node1 node2" --expect progress --window $((8 * BT)) --min-blocks 3 >/dev/null
  chaos heal
  rejoin

  step "extended: 2|2 partition -> no side has 2/3, the chain halts; heal -> resumes"
  chaos partition node0,node1 node2,node3
  sleep $((3 * BT))
  check --nodes "${V[*]}" --expect halt --window $((10 * BT)) >/dev/null
  chaos heal
  rejoin

  step "extended: 150ms +- 50ms delay and 2% loss on two validators -> still progressing"
  chaos netem node0 "delay 150ms 50ms loss 2%"
  chaos netem node1 "delay 150ms 50ms loss 2%"
  check --nodes "${V[*]}" --expect progress --window $((15 * BT)) --min-blocks 3 >/dev/null
  chaos netem-clear node0 node1
  rejoin
}

scenario_joiner() {
  step "joiner: node4 (full node) syncs from genesis and follows the head"
  chaos start "$XDEVS_JOINER"
  wait_healthy 120 "$XDEVS_JOINER"
  local target
  target=$(head_of "${V[@]}")
  check --nodes "$XDEVS_JOINER" --wait-height "$target" --timeout $((120 + target / 5)) >/dev/null
  check --nodes "${V[*]} $XDEVS_JOINER" --expect progress --window $((5 * BT)) --max-lag 3 >/dev/null
}

step "setup: fresh localnet ($XDEVS_IMAGE, block time $XDEVS_BLOCK_TIME)"
bash "$SCRIPT_DIR/localnet-up.sh" --fresh

for s in "${SCENARIOS[@]}"; do
  "scenario_$s"
done

step "final: deep fork + state-root check on every node, safety alerts in the logs"
check --samples 20 --depth 10 --logs

STEP=""
log "all scenarios passed: ${SCENARIOS[*]}"
