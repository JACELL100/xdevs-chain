#!/usr/bin/env bash
# localnet-down.sh: stop the Docker localnet and remove its containers, networks
# (including chaos partition networks) and, unless --keep-data, the chain data.
#
#   scripts/localnet-down.sh                   # wipe everything (next up = new genesis)
#   scripts/localnet-down.sh --keep-data       # keep the volume: same chain on next up
#   scripts/localnet-down.sh --logs out/logs   # save every service's logs first
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

KEEP=0 LOGS_DIR=""
while (($#)); do
  case $1 in
    --keep-data) KEEP=1; shift ;;
    --logs) LOGS_DIR=$2; shift 2 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
need docker

[[ -n $LOGS_DIR ]] && dump_logs "$LOGS_DIR"

# Partition networks hold containers too: detach them first so `down` can remove them.
for net in $(docker network ls -q --filter "label=xdevs.localnet.partition=1"); do
  for id in $(docker network inspect -f '{{range $k, $v := .Containers}}{{$k}} {{end}}' "$net"); do
    docker network disconnect -f "$net" "$id" || true
  done
  docker network rm "$net" >/dev/null || true
done

compose down --remove-orphans --timeout 20
# Only the localnet volume: `down --volumes` would also drop the `dev` service's chain.
if ((!KEEP)) && docker volume inspect "${XDEVS_PROJECT}_localnet" >/dev/null 2>&1; then
  docker volume rm "${XDEVS_PROJECT}_localnet" >/dev/null
fi
log "localnet stopped$( ((KEEP)) && echo ' (data kept)' || echo ' and wiped')"
