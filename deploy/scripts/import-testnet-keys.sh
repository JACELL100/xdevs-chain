#!/usr/bin/env bash
# import-testnet-keys.sh: turn `xdevs-cli testnet init` output into the vault-encrypted key
# layout that deploy/ansible/roles/validator_keys expects. Run it on the OFFLINE / trusted
# workstation where the keys were generated (docs/runbooks/bootstrap-validator.md).
#
#   export ANSIBLE_VAULT_PASSWORD_FILE=~/.xdevs-vault-pass     # or answer the prompts
#   deploy/scripts/import-testnet-keys.sh \
#     --testnet-dir ./xdevs-testnet --keys-dir ~/xdevs-keys/testnet \
#     --password-file ./keystore-password.txt \
#     --map "validator-1=node0 validator-2=node1 validator-3=node2 validator-4=node3 rpc-1=node4 bootnode-1=node5 bootnode-2=node6"
#
# Result, per inventory host:  <keys-dir>/<host>/{consensus_key.json,password,node_key.json}.vault
# (consensus key + password for validators only). Plaintext consensus_key.hex seeds are
# refused: generate the testnet with --password-file so keys are encrypted keystores.
# Afterwards: back up the PLAINTEXT originals offline (deploy/scripts/backup-keys.md), then
# shred them from this machine.
set -euo pipefail

TESTNET_DIR="" KEYS_DIR="" PASSWORD_FILE="" MAP=""
while (($#)); do
  case $1 in
    --testnet-dir) TESTNET_DIR=$2; shift 2 ;;
    --keys-dir) KEYS_DIR=$2; shift 2 ;;
    --password-file) PASSWORD_FILE=$2; shift 2 ;;
    --map) MAP=$2; shift 2 ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done
[[ -d $TESTNET_DIR && -n $KEYS_DIR && -n $MAP ]] || { echo "need --testnet-dir, --keys-dir and --map" >&2; exit 1; }
command -v ansible-vault >/dev/null || { echo "ansible-vault not found (pip install ansible-core)" >&2; exit 1; }

umask 077
mkdir -p "$KEYS_DIR"

encrypt() { # src dest
  [[ -e $2 ]] && { echo "  exists, not overwriting: $2" >&2; return 0; }
  ansible-vault encrypt --output "$2" "$1" >/dev/null
  echo "  wrote $2"
}

for pair in $MAP; do
  host=${pair%%=*}
  node=${pair#*=}
  src="$TESTNET_DIR/$node"
  dst="$KEYS_DIR/$host"
  [[ -d $src ]] || { echo "missing $src" >&2; exit 1; }
  mkdir -p "$dst"
  echo "$host <- $node"
  if [[ -e $src/consensus_key.hex ]]; then
    echo "  REFUSED: $src/consensus_key.hex is an unencrypted seed. Re-run testnet init with --password-file." >&2
    exit 1
  fi
  if [[ -e $src/consensus_key.json ]]; then
    [[ -r $PASSWORD_FILE ]] || { echo "  --password-file is required for validators" >&2; exit 1; }
    encrypt "$src/consensus_key.json" "$dst/consensus_key.json.vault"
    encrypt "$PASSWORD_FILE" "$dst/password.vault"
  fi
  encrypt "$src/node_key.json" "$dst/node_key.json.vault"
done

genesis="$TESTNET_DIR/genesis.json"
if [[ -r $genesis ]]; then
  echo
  echo "genesis.json sha256 (xdevs_genesis_sha256): $(sha256sum "$genesis" | cut -d' ' -f1)"
  echo "Peer ids of the validators (p2p_peer_id in hosts.yml):"
  grep -o '"name"[^,]*\|/p2p/12D3Koo[1-9A-HJ-NP-Za-km-z]*' "$genesis" | paste - - | sed 's/"name"[: ]*//; s#/p2p/# #' || true
  echo "Full-node peer ids: see the 'peer id:' lines printed by xdevs-cli testnet init."
fi
