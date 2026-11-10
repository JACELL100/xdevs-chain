# Runbook: bootstrapping a validator (and launching the network)

Runbook index item 1 (plan §33.8). Tooling: [deploy/ansible](../../deploy/ansible/README.md).
Server checklist: plan §24.4. Key safety: plan §11.8.

There are two cases:

- **A. Network launch**: the first 4 validators, a brand-new genesis.
- **B. Adding or replacing one validator** on a running network.

> **Never run the same consensus key on two machines.** The sign guard
> (`data/consensus/consensus.redb`) protects one host only. Two hosts with one key will
> double-sign, and from Phase 3 that means slashing and tombstoning.

## Before you start (both cases)

- [ ] Oracle Always Free account in the right **home region**, with the A1 capacity actually
      obtained (see [oracle-always-free.md](oracle-always-free.md)). Ubuntu **24.04** ARM64 image.
      The release binaries need glibc 2.39.
- [ ] Your SSH key on the instance (`ubuntu` user). Your public IP as `vault_admin_ssh_cidrs`.
- [ ] VCN security list updated from
      [`oci/security-list-ingress.example.json`](../../deploy/ansible/oci/security-list-ingress.example.json).
      This is Oracle's **second** firewall; Ansible configures the OS one.
- [ ] DNS-only A record `xdevschain-valN.xdevs.in` -> public IP
      ([dns-records.md](../../deploy/cloudflare/dns-records.md)).
- [ ] A workstation with `ansible-core` 2.18, `xdevs-cli` from the same release, and an
      encrypted disk or live USB for key generation.

## A. Network launch

1. **Generate keys and genesis offline** (on the trusted workstation):
   ```bash
   umask 077
   head -c 32 /dev/urandom | base64 > keystore-password.txt   # or a long passphrase
   xdevs-cli testnet init --validators 4 --full-nodes 3 --chain-id 717171 --block-time 2s \
     --host 127.0.0.1 --password-file keystore-password.txt --out ./xdevs-testnet
   ```
   This writes `genesis.json` (4 equal-power validators), `nodeK/consensus_key.json`
   (encrypted keystores), `nodeK/node_key.json`, and configs. We use **only the keys and the
   genesis**: Ansible renders the real `config.toml` per host. Write down the `peer id:` lines
   that it prints.
   > `testnet init` currently funds the public Anvil dev accounts in genesis
   > (`Genesis::dev`). That is fine for a private devnet or soak. Before a **public** testnet,
   > the genesis allocation must come from a production template (open issue). Anyone can
   > spend from those well-known keys.
2. **Back up** the keys offline: [backup-keys.md](../../deploy/scripts/backup-keys.md).
3. **Vault-encrypt** them into the keys directory (outside the repo):
   ```bash
   export XDEVS_KEYS_DIR=~/xdevs-keys/testnet
   deploy/scripts/import-testnet-keys.sh --testnet-dir ./xdevs-testnet --keys-dir "$XDEVS_KEYS_DIR" \
     --password-file keystore-password.txt \
     --map "validator-1=node0 validator-2=node1 validator-3=node2 validator-4=node3 rpc-1=node4 bootnode-1=node5 bootnode-2=node6"
   ```
   Then shred the plaintext (backup-keys.md, step 6).
4. **Inventory**: copy `hosts.yml.example` to `hosts.yml`. Fill in `ansible_host`,
   `private_ip`, `p2p_host`, `p2p_peer_id` (from step 1) and `voting_power` (= 1 each).
   Copy `genesis.json` to `deploy/ansible/inventory/testnet/files/`, and set:
   ```yaml
   xdevs_version: v0.X.Y                       # the release you tested
   xdevs_genesis_sha256: <sha256sum genesis.json>
   oci_region: <home region>
   ```
   Pin `xdevs_release_sha256` from the release's `SHA256SUMS` (recommended).
5. **Converge, starting all validators together**:
   ```bash
   cd deploy/ansible
   ansible-playbook site.yml --ask-vault-pass -e xdevs_genesis_launch=true
   ```
   With `xdevs_genesis_launch=true` the validators are not converged one at a time, and the
   playbook does not wait for blocks per host. A new chain cannot produce blocks until more
   than 2/3 of the power is online.
6. **Verify**:
   ```bash
   ansible validators -b -m command -a "xdevs-check-health --chain-id 717171"   # OK on all 4
   XDEVS_RPC_URLS="v1=... " scripts/check.sh --wait-height 20                   # no fork (from a host on the VCN)
   ```
   Then check Grafana: all four at the same height, rounds at 0, peers at 3 or more.
7. From now on, run plain `ansible-playbook site.yml` (one validator at a time). Publish the
   genesis hash and the first trusted checkpoint (height:hash) on the docs site (plan §11.7).

## B. Adding or replacing one validator

**Phase 2** (static validator set from genesis): a validator that is not in `genesis.json` is
just a full node. "Adding" a validator means replacing the machine of an existing genesis
validator. Real additions come with Phase 3 staking (`xdevs-cli validator register`).

**Replacing the machine of validator-N:**

1. Stop the old machine **for good**:
   `ansible validator-N -b -m systemd -a "name=xdevs state=stopped enabled=false"`.
   Then terminate the instance, or at least delete `/var/lib/xdevs/keys`. Confirm in the
   Oracle console.
2. Quorum math: while validator-N is down, every other validator must stay up (1 of 4 down =
   25% < 1/3). Do not start this during any other maintenance.
3. Create the new instance, update `ansible_host` / `private_ip` / DNS, and run
   `ssh-keyscan` into your `known_hosts`.
4. Converge the new host:
   `ansible-playbook site.yml --limit validator-N`. This installs the OS hardening, the
   **same** vaulted keys, the node and monitoring. The node syncs from genesis via ValueSync.
5. Faster alternative: restore the chain data from a snapshot first.
   `ansible-playbook restore-from-snapshot.yml -e target=validator-N -e allow_empty_signguard=true`.
   This is allowed **only because step 1 guarantees the old key holder is dead**. See
   [restore-from-snapshot.md](restore-from-snapshot.md).
6. Check it: `xdevs-check-health` returns OK, and Grafana shows the node at the tip with rounds
   at 0.

**Adding a full node** (no key): add it under `rpc` or a new group, run
`xdevs-cli keygen --node-key` offline for a stable peer id, vault-encrypt it, and run
`site.yml --limit <host>`.
