#!/usr/bin/env bash
# check-health.sh: health of ONE xDEVS Chain node over JSON-RPC.
# Installed on every host as /usr/local/bin/xdevs-check-health (roles/xdevs_node) and used by
# the Ansible playbooks; also handy from a laptop against the public endpoint.
#
#   check-health.sh                                         # local node, 127.0.0.1:8545
#   check-health.sh --rpc https://rpc.xdevschain.tech --chain-id 717171 --min-peers 0
#   check-health.sh --ref-rpc http://10.0.1.12:8545 --max-lag 5   # compare with another node
#
# Checks: RPC answers; chain id (--chain-id); head advances within --wait s; net_peerCount
# >= --min-peers; eth_syncing == false; optional lag behind --ref-rpc.
# (Fork / state-root comparison across nodes is scripts/check.sh's job.)
#
# Exit: 0 OK, 1 WARN (low peers, syncing, lagging), 2 CRIT (unreachable, wrong chain,
#       head not advancing). Output: one summary line (plus details unless --quiet).
# Needs only bash, curl and sed (no jq), so it runs on minimal images.
set -uo pipefail

RPC="http://127.0.0.1:8545"
WAIT=6
MIN_PEERS=3
CHAIN_ID=""
REF_RPC=""
MAX_LAG=5
QUIET=0
NO_PROGRESS=0
TIMEOUT=5

usage() {
  sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
  cat <<'EOF'
options:
  --rpc URL         node JSON-RPC (default http://127.0.0.1:8545)
  --wait S          seconds to wait for a new block (default 6 = 3 blocks at 2 s)
  --min-peers N     warn below N peers (default 3; 0 disables)
  --chain-id N      expected chain id (decimal); mismatch is critical
  --ref-rpc URL     reference node; warn when this node is more than --max-lag blocks behind
  --max-lag N       default 5
  --no-progress     skip the "head advances" check (genesis launch, halted-chain triage)
  --timeout S       per-request timeout (default 5)
  --quiet           only the summary line
EOF
}

while (($#)); do
  case $1 in
    --rpc) RPC=$2; shift 2 ;;
    --wait) WAIT=$2; shift 2 ;;
    --min-peers) MIN_PEERS=$2; shift 2 ;;
    --chain-id) CHAIN_ID=$2; shift 2 ;;
    --ref-rpc) REF_RPC=$2; shift 2 ;;
    --max-lag) MAX_LAG=$2; shift 2 ;;
    --no-progress) NO_PROGRESS=1; shift ;;
    --timeout) TIMEOUT=$2; shift 2 ;;
    --quiet) QUIET=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

STATUS=0          # 0 ok, 1 warn, 2 crit
MSGS=()
note() { MSGS+=("$1"); }
warn() { MSGS+=("WARN: $1"); ((STATUS < 1)) && STATUS=1; }
crit() { MSGS+=("CRIT: $1"); STATUS=2; }

# rpc URL METHOD -> raw JSON response on stdout; non-zero on transport error.
rpc() {
  curl -fsS --max-time "$TIMEOUT" -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":[]}" "$1"
}
# The "result" of a JSON-RPC response when it is a string ("0x...") or a bare literal.
result_of() {
  sed -n -e 's/.*"result"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
         -e 't' \
         -e 's/.*"result"[[:space:]]*:[[:space:]]*\([a-z0-9]*\).*/\1/p'
}
# "0x1a" -> 26; anything else -> empty (bash's printf would turn "" into 0).
hex2dec() { [[ $1 =~ ^0x[0-9a-fA-F]+$ ]] && printf '%d' "$1"; }

finish() {
  local label=OK
  ((STATUS == 1)) && label=WARN
  ((STATUS == 2)) && label=CRIT
  echo "$label rpc=$RPC height=${H2:-?} peers=${PEERS:-?} syncing=${SYNCING:-?}"
  if ((QUIET == 0)); then
    for m in "${MSGS[@]}"; do echo "  $m"; done
  fi
  exit "$STATUS"
}

# 1. reachable + chain id
if ! CID_JSON=$(rpc "$RPC" eth_chainId); then
  crit "RPC unreachable"
  finish
fi
CID=$(hex2dec "$(result_of <<<"$CID_JSON")")
if [[ -n $CHAIN_ID && $CID != "$CHAIN_ID" ]]; then
  crit "chain id $CID, expected $CHAIN_ID (wrong genesis or wrong endpoint)"
  finish
fi
note "chain id $CID"

# 2. head advances
H1=$(hex2dec "$(rpc "$RPC" eth_blockNumber | result_of)")
if [[ -z $H1 ]]; then crit "eth_blockNumber failed"; finish; fi
H2=$H1
if ((NO_PROGRESS == 0)); then
  deadline=$((SECONDS + WAIT))
  while ((SECONDS < deadline)); do
    sleep 1
    H2=$(hex2dec "$(rpc "$RPC" eth_blockNumber | result_of)")
    [[ -n $H2 ]] && ((H2 > H1)) && break
  done
  if [[ -z $H2 ]] || ((H2 <= H1)); then
    crit "head stuck at $H1 for ${WAIT}s (chain halted or node stalled: docs/runbooks/chain-halted.md)"
  else
    note "head $H1 -> $H2 in <= ${WAIT}s"
  fi
fi

# 3. peers
PEERS=$(hex2dec "$(rpc "$RPC" net_peerCount | result_of)")
if [[ -z $PEERS ]]; then
  warn "net_peerCount failed"
elif ((MIN_PEERS > 0 && PEERS < MIN_PEERS)); then
  warn "only $PEERS peers (< $MIN_PEERS)"
fi

# 4. syncing
SYNC_JSON=$(rpc "$RPC" eth_syncing || true)
if grep -q '"result"[[:space:]]*:[[:space:]]*false' <<<"$SYNC_JSON"; then
  SYNCING=false
elif [[ -n $SYNC_JSON ]]; then
  SYNCING=true
  warn "node is syncing: $(tr -d '\n' <<<"$SYNC_JSON" | cut -c1-160)"
else
  SYNCING=unknown
  warn "eth_syncing failed"
fi

# 5. lag behind a reference node
if [[ -n $REF_RPC ]]; then
  REF=$(hex2dec "$(rpc "$REF_RPC" eth_blockNumber | result_of)")
  if [[ -z $REF ]]; then
    warn "reference $REF_RPC unreachable"
  elif ((REF - H2 > MAX_LAG)); then
    warn "$((REF - H2)) blocks behind $REF_RPC (> $MAX_LAG)"
  fi
fi

finish
