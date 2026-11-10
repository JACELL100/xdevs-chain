#!/usr/bin/env bash
# chaos.sh: fault injection on the Docker localnet (deploy/local/docker-compose.yml).
#
#   scripts/chaos.sh stop node3              graceful stop (SIGINT, like Ctrl-C)
#   scripts/chaos.sh kill node1              SIGKILL: a crash without graceful shutdown
#   scripts/chaos.sh start node1 node3       start stopped / killed nodes; `start node4`
#                                            creates and starts the late joiner
#   scripts/chaos.sh restart node2           graceful restart
#   scripts/chaos.sh pause node2             freeze every process (GC stall / hung host)
#   scripts/chaos.sh unpause node2
#   scripts/chaos.sh isolate node3           disconnect from the localnet network
#   scripts/chaos.sh partition node0,node1 node2,node3
#                                            split into groups that only see each other;
#                                            the first group (and every unlisted node)
#                                            stays on the main network
#   scripts/chaos.sh heal                    undo every isolate / partition
#   scripts/chaos.sh netem node0 "delay 200ms 50ms loss 5%"
#                                            tc netem on every interface of the node
#   scripts/chaos.sh netem-clear node0
#   scripts/chaos.sh status                  containers, networks and heads
#
# netem runs `tc` in a throwaway NET_ADMIN sidecar that joins the node's network namespace
# (`docker run --network container:<node>`), so the node image needs neither iproute2 nor
# NET_ADMIN. The sidecar image (alpine + iproute2) is built locally on first use;
# override with XDEVS_NETEM_IMAGE.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib.sh
source "$SCRIPT_DIR/lib.sh"

NETEM_IMAGE=${XDEVS_NETEM_IMAGE:-xdevs-netem:local}
PARTITION_LABEL=xdevs.localnet.partition

usage() { sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//'; }

need docker

# cid NODE: id of an existing container (fails if the node was never started).
cid() {
  local id
  [[ $1 =~ ^node[0-9]+$ ]] || die "not a localnet node: $1"
  id=$(container_id "$1")
  [[ -n $id ]] || die "$1 has no container (start it first: scripts/chaos.sh start $1)"
  printf '%s\n' "$id"
}

on_network() { # on_network CID NETWORK
  docker inspect -f '{{json .NetworkSettings.Networks}}' "$1" | jq -e --arg n "$2" 'has($n)' >/dev/null
}

# Detach a container from the main network (and every partition network).
detach() {
  local id=$1 net
  for net in $(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' "$id"); do
    docker network disconnect "$net" "$id"
  done
}

ensure_netem_image() {
  docker image inspect "$NETEM_IMAGE" >/dev/null 2>&1 && return 0
  log "building the netem sidecar image $NETEM_IMAGE"
  printf 'FROM alpine:3.22\nRUN apk add --no-cache iproute2\n' |
    docker build -q -t "$NETEM_IMAGE" - >/dev/null
}

# tc_in NODE SCRIPT [ENV...]: run a shell script with tc inside the node's network namespace.
tc_in() {
  local id node=$1 script=$2
  shift 2
  id=$(cid "$node")
  ensure_netem_image
  docker run --rm --network "container:$id" --cap-add NET_ADMIN "$@" "$NETEM_IMAGE" sh -euc "$script"
}

# Interfaces of the namespace except lo (eth0, eth1, ... after a partition).
IFACES='ip -o link show | sed -n "s/^[0-9]*: \([^:@]*\).*/\1/p" | grep -vx lo'

cmd=${1:-}
[[ -n $cmd ]] || { usage >&2; exit 3; }
shift

case $cmd in
  stop)
    (($#)) || die "usage: chaos.sh stop NODE..."
    log "stop (graceful): $*"
    compose stop "$@"
    ;;
  kill)
    (($#)) || die "usage: chaos.sh kill NODE..."
    log "kill -9: $*"
    compose kill -s SIGKILL "$@"
    ;;
  start)
    (($#)) || die "usage: chaos.sh start NODE..."
    log "start: $*"
    # `up --no-recreate` starts existing (stopped / killed) containers and creates new ones
    # (the joiner) without touching the running ones.
    compose up -d --no-deps --no-recreate "$@"
    ;;
  restart)
    (($#)) || die "usage: chaos.sh restart NODE..."
    log "restart (graceful): $*"
    compose restart "$@"
    ;;
  pause)
    (($#)) || die "usage: chaos.sh pause NODE..."
    log "pause: $*"
    compose pause "$@"
    ;;
  unpause)
    (($#)) || die "usage: chaos.sh unpause NODE..."
    log "unpause: $*"
    compose unpause "$@"
    ;;
  isolate)
    (($#)) || die "usage: chaos.sh isolate NODE..."
    for node in "$@"; do
      log "isolate: $node (no network)"
      detach "$(cid "$node")"
    done
    ;;
  partition)
    (($# >= 2)) || die "usage: chaos.sh partition A,B C,D [E ...] (2+ groups)"
    log "partition: $* (group 1 stays on $XDEVS_NETWORK)"
    shift # group 1 stays where it is
    g=1
    for group in "$@"; do
      g=$((g + 1))
      net="${XDEVS_NETWORK}-part${g}"
      docker network inspect "$net" >/dev/null 2>&1 ||
        docker network create --label "$PARTITION_LABEL=1" "$net" >/dev/null
      for node in ${group//,/ }; do
        id=$(cid "$node")
        detach "$id"
        # Same DNS name inside the group, so the group still finds each other.
        docker network connect --alias "$node" "$net" "$id"
      done
    done
    ;;
  heal)
    log "heal: every node back on $XDEVS_NETWORK"
    for node in "${XDEVS_VALIDATORS[@]}" "$XDEVS_JOINER"; do
      id=$(container_id "$node")
      [[ -n $id ]] || continue
      on_network "$id" "$XDEVS_NETWORK" && continue
      detach "$id"
      docker network connect --alias "$node" "$XDEVS_NETWORK" "$id"
      log "  reconnected $node"
    done
    for net in $(docker network ls -q --filter "label=$PARTITION_LABEL=1"); do
      docker network rm "$net" >/dev/null
    done
    ;;
  netem)
    (($# == 2)) || die 'usage: chaos.sh netem NODE "delay 200ms 50ms loss 5%"'
    log "netem $1: $2"
    tc_in "$1" "for i in \$($IFACES); do tc qdisc replace dev \"\$i\" root netem \$NETEM; echo \"\$i: netem \$NETEM\"; done" -e "NETEM=$2"
    ;;
  netem-clear)
    (($#)) || die "usage: chaos.sh netem-clear NODE..."
    for node in "$@"; do
      log "netem-clear $node"
      tc_in "$node" "for i in \$($IFACES); do tc qdisc del dev \"\$i\" root 2>/dev/null || true; done"
    done
    ;;
  status)
    compose ps -a
    echo
    docker network ls --filter "name=$XDEVS_NETWORK"
    echo
    bash "$SCRIPT_DIR/check.sh" --allow-down --no-fork-check || true
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    usage >&2
    die "unknown command: $cmd"
    ;;
esac
