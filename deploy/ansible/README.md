# Ansible: xDEVS Chain testnet

Infrastructure as code for the free-tier testnet in plan §24.2:

| Host | Where | Role |
|---|---|---|
| validator-1..4 | Oracle Always Free Ampere A1 (1 OCPU / 6 GB, Ubuntu 24.04 ARM64) | `validator` |
| bootnode-1 | Oracle E2.1.Micro (1 GB) | `bootnode` (non-validating full node) |
| bootnode-2 | Google Cloud e2-micro (optional) | `bootnode` |
| rpc-1 | Home machine, behind Cloudflare Tunnel | `rpc` (public JSON-RPC / WS) |

Runbooks: [docs/runbooks/](../../docs/runbooks/README.md). Monitoring: [deploy/monitoring/](../monitoring/README.md).
Cloudflare: [deploy/cloudflare/](../cloudflare/README.md).

## Layout

```text
ansible.cfg, requirements.yml
inventory/testnet/hosts.yml.example        -> copy to hosts.yml (gitignored)
inventory/testnet/group_vars/all/main.yml  shared settings, CHANGE_ME placeholders
inventory/testnet/group_vars/all/vault.yml.example -> vault.yml (gitignored, ansible-vault encrypted)
inventory/testnet/group_vars/{validators,rpc,bootnodes}.yml
inventory/testnet/files/genesis.json       (gitignored until published)
roles/common            §24.4: updates, xdevs user, SSH hardening, fail2ban, nftables, chrony, swap
roles/xdevs_node        binary (GitHub Release + SHA-256) or Docker (GHCR), config.toml, systemd
roles/validator_keys    copies PRE-GENERATED vault-encrypted keys (mode 600); never generates
roles/cloudflared       Cloudflare Tunnel on the RPC node
roles/monitoring_agent  Grafana Alloy -> Grafana Cloud (metrics + journal)
roles/snapshots         daily snapshot cron -> Oracle Object Storage (rclone), keep 3
site.yml                converge everything (validators one at a time)
rolling-upgrade.yml     new release / restart / reboot, one node at a time, quorum-checked
restore-from-snapshot.yml
soak-driver.yml         prepare the 24 h soak on the Oracle VMs
tasks/quorum_check.yml  "never take down >= 1/3 of the voting power"
oci/security-list-ingress.example.json     VCN security list rules (the other Oracle firewall)
```

## First-time setup

```bash
python3 -m pip install "ansible-core~=2.18.0"
cd deploy/ansible && ansible-galaxy collection install -r requirements.yml
cp inventory/testnet/hosts.yml.example inventory/testnet/hosts.yml       # fill in
cp inventory/testnet/group_vars/all/vault.yml.example inventory/testnet/group_vars/all/vault.yml
$EDITOR inventory/testnet/group_vars/all/vault.yml && ansible-vault encrypt inventory/testnet/group_vars/all/vault.yml
```

Then follow [bootstrap-validator.md](../../docs/runbooks/bootstrap-validator.md). In short:

1. Generate the network **offline**: `xdevs-cli testnet init --validators 4 --full-nodes 3
   --chain-id 717171 --block-time 2s --host 127.0.0.1 --password-file pw.txt --out ./xdevs-testnet`.
   Only the keys and `genesis.json` are used: Ansible renders every `config.toml` itself with
   the real per-host addresses and the standard ports (30333 p2p, 30334 gossip, 8545 RPC,
   9100 metrics), not testnet init's `base + 10k` ports.
2. Back up the keys ([backup-keys.md](../scripts/backup-keys.md)), then vault-encrypt them with
   [import-testnet-keys.sh](../scripts/import-testnet-keys.sh) into `$XDEVS_KEYS_DIR`.
3. Copy `genesis.json` to `inventory/testnet/files/`, and set `xdevs_genesis_sha256`,
   `xdevs_version` and the peer ids in `hosts.yml`.
4. Open the Oracle **VCN security list** ports (below).
5. `ansible-playbook site.yml -e xdevs_genesis_launch=true` (first start of the chain only), then
   use plain `ansible-playbook site.yml` from then on.

## Oracle has two firewalls

Traffic must pass **both**:

1. **VCN security list** (or a Network Security Group). This is cloud-side and configured in
   the console (Networking -> Virtual cloud networks -> your VCN -> Security Lists -> Default),
   or with the OCI CLI:
   ```bash
   oci network security-list update --security-list-id <ocid> \
     --ingress-security-rules file://oci/security-list-ingress.example.json   # edit the admin IP first
   ```
   `update` **replaces** the whole ingress list, so the file contains the full set (SSH, P2P,
   metrics, ICMP). Delete the "SOAK ONLY" 8545 rule outside soaks.
2. **OS firewall**. Oracle Ubuntu images ship iptables rules (`/etc/iptables/rules.v4`) that
   end in `REJECT`. `roles/common` installs an nftables table with the real policy and removes
   only that final INPUT `REJECT`. It keeps Oracle's `InstanceServices` egress rules, which
   protect the metadata service.

## Install methods

`xdevs_install_method: binary` (default) downloads `xdevs-<tag>-<target>.tar.gz` from the GitHub
Release for the host's CPU (`aarch64` / `x86_64`). It checks the tarball against the pinned
`xdevs_release_sha256` if you set one, or else against the release's `.sha256`, smoke-tests it,
and switches `/usr/local/bin/xdevs`. The previous 3 releases stay in `/opt/xdevs/releases` for
rollback. `docker` runs `ghcr.io/jacell100/xdevs-chain:<X.Y.Z>` (or a pinned digest in
`xdevs_docker_image_ref`) with host networking under the same systemd unit name.

## Safety rules built in

- Validators are only ever restarted one at a time (`serial: 1`). A quorum check runs before
  each one: offline power plus this validator must stay below 1/3. After each restart there is
  a health check and a back-in-consensus check (`tasks/quorum_check.yml`,
  `roles/xdevs_node/tasks/wait_*.yml`).
- Unattended upgrades never reboot, and needrestart never restarts `xdevs`.
- Keys are refused if plaintext, refused if two hosts share one, and never overwritten unless
  `validator_keys_force_replace=true` (key rotation runbook).
- `data/consensus/` (WAL + sign guard) is never part of a snapshot and never restored from one.
- `genesis.json` must match `xdevs_genesis_sha256` before it is copied anywhere.

## CI

[.github/workflows/deploy.yml](../../.github/workflows/deploy.yml) runs `site.yml` or
`rolling-upgrade.yml` on manual dispatch only. It stays disabled until the repository variable
`XDEVS_DEPLOY_ENABLED=true` and its secrets exist. Keys are never deployed from CI
(`--skip-tags keys`).
