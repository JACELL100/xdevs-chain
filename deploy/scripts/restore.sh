#!/usr/bin/env bash
# restore.sh: restore an xDEVS Chain node's chain data from a snapshot made by snapshot.sh.
# Installed as /usr/local/bin/xdevs-restore (roles/snapshots); driven by
# deploy/ansible/restore-from-snapshot.yml. Runbook: docs/runbooks/restore-from-snapshot.md.
#
#   xdevs-restore --env /etc/xdevs/snapshot.env --latest
#   xdevs-restore --env /etc/xdevs/snapshot.env --snapshot xdevs-testnet-20261110T031700Z-h123456
#   xdevs-restore --file /tmp/xdevs-testnet-...-h123456.tar.zst      # local archive (+ .sha256)
#
# Safety rules
#   * The archive's SHA-256 must match its .sha256 file, and its genesis.json must be
#     byte-identical to this host's genesis.json (same chain), else nothing is touched.
#   * data/consensus/ (Malachite WAL + sign guard) is NEVER taken from a snapshot. The host's
#     own copy is kept in place. On a validator WITHOUT one (a fresh machine) the restore is
#     refused unless --allow-empty-signguard: with an empty sign guard the key could
#     double-sign heights it already signed elsewhere. Pass it only after the OLD machine with
#     this key is confirmed stopped for good (docs/runbooks/restore-from-snapshot.md).
#   * The previous data dir is kept as data.pre-restore-<UTC stamp> (delete it once the
#     node is healthy) unless --discard-old.
#
# Exit: 0 restored and healthy, 1 error (node state unchanged unless stated), 2 restored
#       but not healthy within --wait-healthy seconds.
set -euo pipefail

ENV_FILE=/etc/xdevs/snapshot.env
MODE=""
SNAP=""
FILE=""
YES=0
ALLOW_EMPTY_SIGNGUARD=0
DISCARD_OLD=0
WAIT_HEALTHY=900

usage() { sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; cat <<'EOF'
options: --env FILE  --latest | --snapshot NAME | --file PATH  --yes
         --allow-empty-signguard  --discard-old  --wait-healthy SECONDS (default 900)
EOF
}
while (($#)); do
  case $1 in
    --env) ENV_FILE=$2; shift 2 ;;
    --latest) MODE=latest; shift ;;
    --snapshot) MODE=name; SNAP=$2; shift 2 ;;
    --file) MODE=file; FILE=$2; shift 2 ;;
    --yes) YES=1; shift ;;
    --allow-empty-signguard) ALLOW_EMPTY_SIGNGUARD=1; shift ;;
    --discard-old) DISCARD_OLD=1; shift ;;
    --wait-healthy) WAIT_HEALTHY=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done
[[ -n $MODE ]] || { usage >&2; exit 1; }

# shellcheck source=/dev/null
[[ -r $ENV_FILE ]] && source "$ENV_FILE"
: "${XDEVS_HOME:=/var/lib/xdevs}"
: "${XDEVS_RPC:=http://127.0.0.1:8545}"
: "${XDEVS_CHAIN_ID:=}"
: "${XDEVS_ROLE:=fullnode}"
: "${SNAPSHOT_REMOTE:=}"
: "${SNAPSHOT_STAGING:=/var/lib/xdevs-snapshots}"
: "${RCLONE_CONFIG:=/etc/xdevs/rclone.conf}"
export RCLONE_CONFIG

log() { echo "$(date -u +%FT%TZ) xdevs-restore: $*" >&2; }
die() { log "ERROR: $*"; exit 1; }
((EUID == 0)) || die "run as root"
for bin in tar zstd sha256sum systemctl; do command -v "$bin" >/dev/null || die "missing $bin"; done

mkdir -p "$SNAPSHOT_STAGING"
chmod 0700 "$SNAPSHOT_STAGING"

# ---- 1. fetch + verify ------------------------------------------------------------------------
case $MODE in
  latest|name)
    command -v rclone >/dev/null || die "missing rclone"
    [[ -n $SNAPSHOT_REMOTE ]] || die "SNAPSHOT_REMOTE is not set"
    if [[ $MODE == latest ]]; then
      SNAP=$(rclone lsf --files-only "$SNAPSHOT_REMOTE" | grep -E '^xdevs-.*\.tar\.zst$' | sort | tail -n1)
      [[ -n $SNAP ]] || die "no snapshot found in $SNAPSHOT_REMOTE"
      SNAP=${SNAP%.tar.zst}
    fi
    SNAP=${SNAP%.tar.zst}
    ARCHIVE="$SNAPSHOT_STAGING/$SNAP.tar.zst"
    log "downloading $SNAP"
    for ext in .tar.zst .tar.zst.sha256 .json; do
      rclone copyto --retries 5 "$SNAPSHOT_REMOTE/$SNAP$ext" "$SNAPSHOT_STAGING/$SNAP$ext" \
        || { [[ $ext == .json ]] && log "no manifest (ok)"; } || die "download of $SNAP$ext failed"
    done
    ;;
  file)
    [[ -r $FILE ]] || die "$FILE not readable"
    ARCHIVE=$(readlink -f "$FILE")
    SNAP=$(basename "$ARCHIVE" .tar.zst)
    ;;
esac
[[ -r $ARCHIVE.sha256 ]] || die "missing $ARCHIVE.sha256"
(cd "$(dirname "$ARCHIVE")" && sha256sum -c --quiet "$(basename "$ARCHIVE").sha256") \
  || die "SHA-256 mismatch for $ARCHIVE"
log "archive checksum ok"

MANIFEST="${ARCHIVE%.tar.zst}.json"
if [[ -r $MANIFEST && -n $XDEVS_CHAIN_ID ]]; then
  m_chain=$(sed -n 's/.*"chain_id"[[:space:]]*:[[:space:]]*"\{0,1\}\([0-9]*\)"\{0,1\}.*/\1/p' "$MANIFEST" | head -n1)
  [[ -z $m_chain || $m_chain == "$XDEVS_CHAIN_ID" ]] || die "snapshot is for chain $m_chain, this node runs $XDEVS_CHAIN_ID"
fi

# Same genesis, byte for byte.
[[ -r $XDEVS_HOME/genesis.json ]] || die "$XDEVS_HOME/genesis.json missing (run site.yml first)"
snap_genesis=$(zstd -dc "$ARCHIVE" | tar -xOf - genesis.json | sha256sum | cut -d' ' -f1) \
  || die "archive has no genesis.json"
host_genesis=$(sha256sum "$XDEVS_HOME/genesis.json" | cut -d' ' -f1)
[[ $snap_genesis == "$host_genesis" ]] || die "snapshot genesis $snap_genesis != host genesis $host_genesis (different chain)"
log "genesis matches ($host_genesis)"

# ---- 2. sign-guard safety ----------------------------------------------------------------------------
CONSENSUS_DIR="$XDEVS_HOME/data/consensus"
if [[ $XDEVS_ROLE == validator && ! -d $CONSENSUS_DIR ]]; then
  if ((ALLOW_EMPTY_SIGNGUARD == 0)); then
    die "validator without $CONSENSUS_DIR (fresh host). Restoring here starts with an EMPTY sign
       guard. Confirm the old machine with this key is stopped and wiped, then re-run with
       --allow-empty-signguard (docs/runbooks/restore-from-snapshot.md)."
  fi
  log "WARNING: continuing with an empty sign guard (--allow-empty-signguard)"
fi

if ((YES == 0)); then
  read -r -p "Stop xdevs on $(hostname) and replace its chain data with $SNAP? [y/N] " ans
  [[ $ans == y || $ans == Y ]] || die "aborted by the operator"
fi

# ---- 3. swap the data dir ----------------------------------------------------------------------------
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OLD="$XDEVS_HOME/data.pre-restore-$STAMP"
log "stopping xdevs"
systemctl stop xdevs

NEW="$XDEVS_HOME/.restore-$STAMP"
mkdir -p "$NEW"
zstd -dc "$ARCHIVE" | tar -C "$NEW" --exclude='genesis.json' --exclude='data/consensus' -xf - \
  || { rm -rf "$NEW"; systemctl start xdevs; die "extract failed; old data left in place and node restarted"; }
[[ -d $NEW/data ]] || { rm -rf "$NEW"; systemctl start xdevs; die "archive has no data/ directory"; }

if [[ -d $XDEVS_HOME/data ]]; then
  mv "$XDEVS_HOME/data" "$OLD"
fi
mv "$NEW/data" "$XDEVS_HOME/data"
rmdir "$NEW"
# Keep this host's own WAL + sign guard.
if [[ -d $OLD/consensus ]]; then
  cp -a "$OLD/consensus" "$XDEVS_HOME/data/consensus"
  log "kept this host's data/consensus (WAL + sign guard)"
fi
chown -R xdevs:xdevs "$XDEVS_HOME/data"
if ((DISCARD_OLD == 1)) && [[ -d $OLD ]]; then
  rm -rf "$OLD"
  log "old data discarded"
elif [[ -d $OLD ]]; then
  log "previous data kept at $OLD; delete it once the node is healthy"
fi

log "starting xdevs"
systemctl start xdevs

# ---- 4. wait until healthy --------------------------------------------------------------------------
if command -v xdevs-check-health >/dev/null; then
  deadline=$((SECONDS + WAIT_HEALTHY))
  args=(--rpc "$XDEVS_RPC" --min-peers 1 --quiet)
  [[ -n $XDEVS_CHAIN_ID ]] && args+=(--chain-id "$XDEVS_CHAIN_ID")
  until xdevs-check-health "${args[@]}"; do
    ((SECONDS < deadline)) || { log "node not healthy after ${WAIT_HEALTHY}s: check journalctl -u xdevs"; exit 2; }
    sleep 10
  done
fi
[[ $MODE == file ]] || rm -f "$SNAPSHOT_STAGING/$SNAP".tar.zst* "$SNAPSHOT_STAGING/$SNAP.json" 2>/dev/null || true
log "restored $SNAP; node healthy"
