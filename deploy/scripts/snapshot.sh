#!/usr/bin/env bash
# snapshot.sh: daily state snapshot of an xDEVS Chain node -> Oracle Object Storage
# (plan §24.4 item 10, §24.8). Installed as /usr/local/bin/xdevs-snapshot by
# deploy/ansible/roles/snapshots and run by cron on ONE host (snapshot_enabled).
#
#   xdevs-snapshot --env /etc/xdevs/snapshot.env            # cron
#   xdevs-snapshot --env /etc/xdevs/snapshot.env --no-upload --keep-local   # local test
#
# What it does
#   1. Preflight: local node healthy (head advancing). On a VALIDATOR it also requires every
#      other validator listed in SNAPSHOT_QUORUM_METRICS to be advancing, because stopping
#      this one must never take the network to >= 1/3 of the power offline. If the check
#      fails it SKIPS today's snapshot (exit 3); the "snapshot stale" alert fires after 36 h.
#   2. Records the head (number, hash, stateRoot), stops xdevs (graceful SIGINT via systemd),
#      archives genesis.json + data/ WITHOUT data/consensus/ (the WAL and sign guard are
#      per-validator and must never be copied to another host), restarts xdevs at once.
#   3. Hashes the archive, writes a manifest, uploads both with rclone, keeps the newest
#      SNAPSHOT_KEEP (default 3) on the remote, and writes textfile metrics for Alloy.
#
# Downtime = the time to tar + zstd the data dir. The redb databases are not safe to copy
# while the node writes them, so the node is stopped. (Open issue: an online
# `xdevs db snapshot` command would remove the downtime.)
#
# Exit: 0 ok, 1 error, 3 skipped by preflight.
set -euo pipefail

ENV_FILE=/etc/xdevs/snapshot.env
UPLOAD=1
KEEP_LOCAL=0
FORCE=0

usage() { sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; echo "options: --env FILE  --no-upload  --keep-local  --force (skip the quorum preflight)"; }
while (($#)); do
  case $1 in
    --env) ENV_FILE=$2; shift 2 ;;
    --no-upload) UPLOAD=0; shift ;;
    --keep-local) KEEP_LOCAL=1; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

# shellcheck source=/dev/null
[[ -r $ENV_FILE ]] && source "$ENV_FILE"
: "${XDEVS_HOME:=/var/lib/xdevs}"
: "${XDEVS_RPC:=http://127.0.0.1:8545}"
: "${XDEVS_CHAIN_ID:=}"
: "${XDEVS_ROLE:=fullnode}"
: "${XDEVS_NETWORK:=testnet}"
: "${SNAPSHOT_REMOTE:=}"
: "${SNAPSHOT_KEEP:=3}"
: "${SNAPSHOT_MIN_PEERS:=3}"
: "${SNAPSHOT_STAGING:=/var/lib/xdevs-snapshots}"
: "${SNAPSHOT_TEXTFILE_DIR:=/var/lib/alloy/textfile}"
: "${SNAPSHOT_QUORUM_METRICS:=}"      # space-separated http://ip:port/metrics of the OTHER validators
: "${RCLONE_CONFIG:=/etc/xdevs/rclone.conf}"
export RCLONE_CONFIG

log() { echo "$(date -u +%FT%TZ) xdevs-snapshot: $*" >&2; }
die() { log "ERROR: $*"; write_metrics 0; exit 1; }

for bin in curl tar zstd sha256sum systemctl; do
  command -v "$bin" >/dev/null || { log "missing $bin"; exit 1; }
done
((UPLOAD == 0)) || command -v rclone >/dev/null || { log "missing rclone"; exit 1; }
((UPLOAD == 0)) || [[ -n $SNAPSHOT_REMOTE ]] || { log "SNAPSHOT_REMOTE is not set"; exit 1; }

rpc() {
  curl -fsS --max-time 5 -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" "$XDEVS_RPC"
}
json_field() { sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"; }
metric_value() { sed -nE "s/^$1(\{[^}]*\})? ([0-9.e+]+).*/\2/p" | head -n1; }

write_metrics() { # $1 = 1 success / 0 failure
  [[ -d $SNAPSHOT_TEXTFILE_DIR ]] || return 0
  local f="$SNAPSHOT_TEXTFILE_DIR/xdevs_snapshot.prom" tmp
  tmp=$(mktemp "$SNAPSHOT_TEXTFILE_DIR/.xdevs_snapshot.XXXXXX") || return 0
  {
    echo "# HELP xdevs_snapshot_last_run_success 1 if the last snapshot run succeeded."
    echo "# TYPE xdevs_snapshot_last_run_success gauge"
    echo "xdevs_snapshot_last_run_success $1"
    echo "# HELP xdevs_snapshot_last_run_timestamp_seconds Unix time of the last snapshot run."
    echo "# TYPE xdevs_snapshot_last_run_timestamp_seconds gauge"
    echo "xdevs_snapshot_last_run_timestamp_seconds $(date +%s)"
    if [[ $1 == 1 ]]; then
      echo "# HELP xdevs_snapshot_last_success_timestamp_seconds Unix time of the last successful snapshot."
      echo "# TYPE xdevs_snapshot_last_success_timestamp_seconds gauge"
      echo "xdevs_snapshot_last_success_timestamp_seconds $(date +%s)"
      echo "# HELP xdevs_snapshot_last_height Head height recorded for the last snapshot."
      echo "# TYPE xdevs_snapshot_last_height gauge"
      echo "xdevs_snapshot_last_height ${HEIGHT:-0}"
      echo "# HELP xdevs_snapshot_last_size_bytes Size of the last snapshot archive."
      echo "# TYPE xdevs_snapshot_last_size_bytes gauge"
      echo "xdevs_snapshot_last_size_bytes ${SIZE:-0}"
      echo "# HELP xdevs_snapshot_last_downtime_seconds Node downtime of the last snapshot."
      echo "# TYPE xdevs_snapshot_last_downtime_seconds gauge"
      echo "xdevs_snapshot_last_downtime_seconds ${DOWNTIME:-0}"
    elif [[ -r $f ]]; then
      grep -E '^xdevs_snapshot_last_(success_timestamp_seconds|height|size_bytes|downtime_seconds) ' "$f" || true
    fi
  } >"$tmp"
  chmod 0644 "$tmp"
  mv -f "$tmp" "$f"
}

# ---- 1. preflight ----------------------------------------------------------------------------
if command -v xdevs-check-health >/dev/null; then
  hc_args=(--rpc "$XDEVS_RPC" --min-peers "$SNAPSHOT_MIN_PEERS" --wait 10 --quiet)
  [[ -n $XDEVS_CHAIN_ID ]] && hc_args+=(--chain-id "$XDEVS_CHAIN_ID")
  if ! xdevs-check-health "${hc_args[@]}" >&2; then
    if ((FORCE == 0)); then log "SKIP: local node is not healthy"; write_metrics 0; exit 3; fi
    log "local node not healthy, continuing because of --force"
  fi
fi

if [[ $XDEVS_ROLE == validator && $FORCE == 0 ]]; then
  for url in $SNAPSHOT_QUORUM_METRICS; do
    h1=$(curl -fsS --max-time 5 "$url" | metric_value xdevs_block_height || true)
    sleep 6
    h2=$(curl -fsS --max-time 5 "$url" | metric_value xdevs_block_height || true)
    if [[ -z $h1 || -z $h2 ]] || ! awk -v a="$h1" -v b="$h2" 'BEGIN { exit !(b > a) }'; then
      log "SKIP: validator $url is not advancing ($h1 -> $h2). Stopping this validator now could"
      log "      take >= 1/3 of the voting power offline and halt the chain."
      write_metrics 0
      exit 3
    fi
  done
fi

# ---- 2. record head, stop, archive, start -------------------------------------------------------
BLOCK=$(rpc eth_getBlockByNumber '["latest",false]') || die "eth_getBlockByNumber failed"
HEX_HEIGHT=$(json_field number <<<"$BLOCK")
[[ $HEX_HEIGHT =~ ^0x[0-9a-fA-F]+$ ]] || die "could not read the head block"
HEIGHT=$((HEX_HEIGHT))
HASH=$(json_field hash <<<"$BLOCK")
ROOT=$(json_field stateRoot <<<"$BLOCK")
VERSION=$(/usr/local/bin/xdevs --version 2>/dev/null | head -n1 || echo unknown)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
NAME="xdevs-${XDEVS_NETWORK}-${STAMP}-h${HEIGHT}"
mkdir -p "$SNAPSHOT_STAGING"
chmod 0700 "$SNAPSHOT_STAGING"
ARCHIVE="$SNAPSHOT_STAGING/$NAME.tar.zst"

need=$(du -sb "$XDEVS_HOME/data" | cut -f1)
avail=$(df -B1 --output=avail "$SNAPSHOT_STAGING" | tail -n1 | tr -d ' ')
((avail > need)) || die "not enough space in $SNAPSHOT_STAGING ($avail B free, data is $need B)"

STOPPED=0
restart_node() {
  if ((STOPPED == 1)); then
    systemctl start xdevs || log "ERROR: could not restart xdevs - check it NOW"
    STOPPED=0
  fi
}
trap restart_node EXIT

log "head $HEIGHT $HASH; stopping xdevs"
t0=$(date +%s)
systemctl stop xdevs
STOPPED=1
tar -C "$XDEVS_HOME" --exclude='data/consensus' --exclude='data/consensus/*' -cf - genesis.json data \
  | zstd -q -T0 -3 -o "$ARCHIVE"
restart_node
DOWNTIME=$(( $(date +%s) - t0 ))
log "xdevs restarted after ${DOWNTIME}s"

SIZE=$(stat -c %s "$ARCHIVE")
SHA=$(sha256sum "$ARCHIVE" | cut -d' ' -f1)
echo "$SHA  $NAME.tar.zst" >"$ARCHIVE.sha256"
cat >"$SNAPSHOT_STAGING/$NAME.json" <<EOF
{
  "name": "$NAME",
  "network": "$XDEVS_NETWORK",
  "chain_id": "${XDEVS_CHAIN_ID}",
  "height_before_stop": $HEIGHT,
  "block_hash_before_stop": "$HASH",
  "state_root_before_stop": "$ROOT",
  "node_version": "$VERSION",
  "created_utc": "$STAMP",
  "source_host": "$(hostname)",
  "archive": "$NAME.tar.zst",
  "archive_sha256": "$SHA",
  "archive_bytes": $SIZE,
  "excludes": ["data/consensus"],
  "note": "The head inside the archive is height_before_stop or a few blocks higher."
}
EOF

# ---- 3. upload + retention -------------------------------------------------------------------------
if ((UPLOAD == 1)); then
  for f in "$ARCHIVE" "$ARCHIVE.sha256" "$SNAPSHOT_STAGING/$NAME.json"; do
    rclone copyto --retries 5 "$f" "$SNAPSHOT_REMOTE/$(basename "$f")" || die "upload of $f failed"
  done
  rclone check --one-way --download "$SNAPSHOT_STAGING" "$SNAPSHOT_REMOTE" --include "$NAME.tar.zst" \
    || die "uploaded archive does not match the local one"
  mapfile -t all < <(rclone lsf --files-only "$SNAPSHOT_REMOTE" | grep -E '^xdevs-.*\.tar\.zst$' | sort)
  if ((${#all[@]} > SNAPSHOT_KEEP)); then
    for old in "${all[@]:0:${#all[@]}-SNAPSHOT_KEEP}"; do
      base=${old%.tar.zst}
      log "retention: deleting $base"
      rclone deletefile "$SNAPSHOT_REMOTE/$old" || true
      rclone deletefile "$SNAPSHOT_REMOTE/$old.sha256" || true
      rclone deletefile "$SNAPSHOT_REMOTE/$base.json" || true
    done
  fi
fi

((KEEP_LOCAL == 1)) || rm -f "$ARCHIVE" "$ARCHIVE.sha256" "$SNAPSHOT_STAGING/$NAME.json"
write_metrics 1
log "done: $NAME ($SIZE bytes, sha256 $SHA)"
