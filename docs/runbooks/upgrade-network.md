# Runbook: upgrading the network (non-fork and fork-height releases)

Runbook index item 3 (plan §24.5, §24.6). Tool: `deploy/ansible/rolling-upgrade.yml`, or the
manual **deploy** workflow (`.github/workflows/deploy.yml`).

> Hard rule: never take down 1/3 or more of the voting power. With 4 equal validators, that
> means **one validator at a time**, and only while the other three are healthy. The playbook
> enforces this with `tasks/quorum_check.yml` before every validator.

## Which kind of release?

| Kind | Examples | How |
|---|---|---|
| **Non-consensus** | RPC, performance, logging, metrics, CLI | Rolling upgrade, any time |
| **Consensus-compatible bug fix** | Fix that cannot change block validity or state roots | Rolling upgrade; a mixed old/new set must agree. Prove it on a devnet first |
| **Fork-height** | Any change to block validity, execution, gas, state root, system contracts | Ships with fork height N (`xdevs-vX at block N`). Released well before N, rolled out, then activates at N on every node at once |

If in doubt, it is a fork-height release. Determinism bugs found in production go to
[emergency-restart.md](emergency-restart.md).

## Before any rollout

- [ ] The release exists on GitHub (`release.yml`) with both `aarch64` and `x86_64` tarballs and
      `SHA256SUMS`. For the Docker method: the GHCR image `X.Y.Z`.
- [ ] It ran on a devnet: the CI soak (`soak.yml`) or a localnet ([localnet.md](localnet.md)).
      Fork-height releases also need an **upgrade test**: a devnet that crosses the fork height
      with all nodes on the new binary (plan §26 "Upgrade").
- [ ] Pin the hashes: set `xdevs_release_sha256` from `SHA256SUMS` in `group_vars/all/main.yml`.
- [ ] All 4 validators are healthy in Grafana: no alerts, rounds at 0, peers at 3 or more.
- [ ] No other maintenance running. Snapshot age < 24 h.

## Non-fork rolling upgrade

```bash
cd deploy/ansible
ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z --check --diff   # dry run
ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z
```

The playbook takes these steps:

1. **rpc-1 (canary)**: download, verify the SHA-256, smoke-test `xdevs --version`, switch, restart,
   wait for health.
2. **Each validator, in inventory order**: quorum check, then install (download before stopping),
   restart, then wait for health (head advancing, synced, peers >= 3). Then it checks that the
   node is back in consensus (`xdevs_consensus_height` advancing at the tip, and no
   `sign guard:` / equivocation / `FORK:` lines in the journal). Then a 30 s pause.
3. **Bootnodes**.

Any failure stops the run (`any_errors_fatal`), with at most one node affected. Fix it or roll
back that node, then re-run. Hosts already on the new version only get a health check.

**Rollback** (non-fork only): `-e xdevs_version=<previous tag>`. The last 3 releases stay in
`/opt/xdevs/releases`, so no download is needed.

Through GitHub: Actions -> **deploy** -> Run workflow: `rolling-upgrade.yml`, version, and
`check_mode` = true first, then false.

**Restarts and OS reboots** use the same safety:
`-e xdevs_force_restart=true` or `-e xdevs_rolling_reboot=true` (kernel updates; unattended
upgrades never reboot on their own).

## Fork-height release

1. **Choose N** with enough lead time for every operator (community validators later). At 2 s
   blocks there are ~43,200 blocks per day. On the testnet, pick N at least **3 days** out.
   Announce N with an approximate UTC time on Discord, GitHub Releases and the docs site.
2. Release the binary with the fork schedule built in. Check the fork height in the release
   notes and in the chain config the binary reports.
3. Roll it out like a non-fork release, but pass the fork height so the playbook refuses to start
   too close to it:
   ```bash
   ansible-playbook rolling-upgrade.yml -e xdevs_version=vX.Y.Z -e xdevs_fork_height=N
   ```
   (`xdevs_fork_margin_blocks`, default 5,400 ~ 3 h, is the minimum distance from N.) Before N
   the new binary follows the old rules, so a mixed network is fine.
4. **Before N**: every validator **and** the RPC node and bootnodes report the new version
   (`ansible all -b -m command -a "xdevs --version"`). Nodes still on the old binary will reject
   blocks after N and fall off the chain.
5. **At N**: watch Grafana. Height keeps advancing, rounds stay at 0, and there are no
   `XdevsNodeLagging` alerts. Run `scripts/check.sh --expect progress` from a VCN host to check
   for forks.
6. **If the chain halts at N**: [chain-halted.md](chain-halted.md), then
   [emergency-restart.md](emergency-restart.md). Do **not** roll back below N on some nodes only.

## After every rollout

- Write in the ops log: version, time, any failure, the longest stall seen.
- Update `xdevs_version` in `group_vars/all/main.yml` so `site.yml` keeps the new version.
