# Runbook: preparing and running the 24 h soak on the Oracle VMs

Pass criteria, `summary.json` and the chaos rules are in [soak.md](soak.md). This page covers
the **infrastructure side**: getting the 4 Oracle validators and a driver host ready with
Ansible, running `scripts/soak.sh --mode host`, and cleaning up.

> Run the soak on a **devnet genesis** (`xdevs-cli testnet init`), not the public testnet
> genesis. soak.sh's fallback load spends from the public Anvil dev accounts 5 to 9, which only
> a dev genesis funds. Anyone could spend them on a public network.

## 1. Network up

Follow [bootstrap-validator.md](bootstrap-validator.md) section A with the devnet genesis
(`xdevs_genesis_launch=true` for the first start). All four validators must be healthy in
Grafana before you start.

## 2. Open the validators' RPC to the private subnet (soak only)

`soak.sh` in host mode queries every node's JSON-RPC directly. Validators normally bind RPC to
127.0.0.1, so:

1. Add the "SOAK ONLY" 8545 rule to the **VCN security list** (it is in
   `deploy/ansible/oci/security-list-ingress.example.json`; source = the private subnet).
2. Run the soak playbook. It rebinds RPC to each validator's private IP and opens 8545 in
   nftables for the private subnet only, **one validator at a time**, with quorum and health
   checks. It also prepares the driver host (the `soak_driver` group; bootnode-1 by default):
   ```bash
   cd deploy/ansible
   ansible-playbook soak-driver.yml -e xdevs_version=vX.Y.Z
   ```
   On the driver this installs `jq`, `curl`, `tmux`, the repo's `scripts/` to
   `/opt/xdevs-soak/scripts`, `xdevs-cli` from the release, and `xdevs-loadgen` copied out of the
   GHCR image with podman (the release tarball ships only `xdevs` and `xdevs-cli`). It writes
   `/opt/xdevs-soak/env` with `XDEVS_RPC_URLS`.

   On a 1 GB micro driver, `xdevs-loadgen` at 20 TPS is fine. If you want more load, use a
   validator as the driver: put it in `soak_driver` in `hosts.yml`.
3. **Log collection** needs SSH from the driver to the validators (`--log-cmd`). Either give the
   driver a dedicated key that is allowed on the validators (add the driver's private IP to
   `vault_admin_ssh_cidrs` for the soak), or collect logs from your workstation after the run
   with Ansible (step 5). The second option is simpler.

## 3. Run it (on the driver)

```bash
ssh ubuntu@<driver>
tmux new -s soak
source /opt/xdevs-soak/env
cd /opt/xdevs-soak
scripts/check.sh --wait-height 1                      # all four reachable, no fork
scripts/soak.sh --mode host --duration 24h --tps 20 --stall-limit 30 \
  --loadgen-rpc "$XDEVS_LOADGEN_RPC" --artifacts ~/soak-$(date -u +%F)
# detach: Ctrl-b d     re-attach: tmux attach -t soak
```

Chaos during the run (from your workstation, at most **one validator at a time**, never two
overlapping):

```bash
cd deploy/ansible
ansible validator-3 -b -m shell -a "systemctl kill -s SIGKILL xdevs; sleep 60; systemctl start xdevs"
ansible validator-2 -b -m shell -a "IF=\$(ip route show default | awk '{print \$5; exit}'); tc qdisc add dev \$IF root netem delay 150ms 50ms loss 2%; sleep 600; tc qdisc del dev \$IF root"
```

Write down every fault with its UTC time for the report. Watch the Grafana dashboard: rounds,
peers, `build` / `validate` p95, CPU, RAM and disk.

## 4. Pass or fail

`~/soak-<date>/summary.json` must show `"pass": true` (criteria in [soak.md](soak.md)). If you
skipped `--log-cmd`, collect the logs now and run the log part of the check yourself:

```bash
ansible validators -b -m shell -a "journalctl -u xdevs --since '-25h' -o cat --no-pager" > node-logs.txt
grep -E -i 'sign guard:|equivocation|FORK:|consensus halted|panicked' node-logs.txt   # must print nothing
```

Archive the artifacts directory and node-logs.txt with the commit, TPS, faults and longest
stall.

## 5. Clean up

```bash
ansible-playbook soak-driver.yml -e soak_cleanup=true    # RPC back to 127.0.0.1, 8545 closed (one validator at a time)
```

Then remove the "SOAK ONLY" rule from the VCN security list and any temporary SSH CIDR. Before
the public testnet, re-launch with the real genesis (bootstrap-validator.md section A).
