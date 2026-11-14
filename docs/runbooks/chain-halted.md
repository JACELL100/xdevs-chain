# Runbook: chain halted (diagnosis checklist)

Runbook index item 4 (plan §33.8, §24.7, §25.1). Triggered by `XdevsNoNewBlock` (no block for
30 s), `XdevsQuorumLost`, `XdevsConsensusRoundClimbing`, or users reporting that the RPC is
stuck.

**Background.** Tendermint-style BFT (Malachite) finalizes a block only with precommits from
**more than 2/3 of the voting power**. With 4 equal validators, 3 must be up, connected, on the
same rules, and close enough in time. When that fails, the chain **halts safely**: no forks, no
reverted blocks. The fix is to restore the missing condition, not to "force" blocks.

Work top-down, and stop at the first cause you find. Write down times and findings as you go
(for the post-mortem).

## 0. First minute: confirm and scope

```bash
cd deploy/ansible
ansible validators -b -m command -a "xdevs-check-health --chain-id 717171 --min-peers 2" -o
deploy/scripts/check-health.sh --rpc https://rpc.xdevschain.tech --chain-id 717171 --min-peers 0
```

| What you see | Meaning | Go to |
|---|---|---|
| All nodes `CRIT head stuck at H` (same H) | Network-wide halt | 1 |
| One node stuck or behind, the others advance | Single-node problem, not a halt | 6 |
| Public RPC stuck, validators advance | rpc-1 or tunnel problem | 6 |
| Nodes stuck at **different** heights, or a node logs `FORK:` / state root mismatch | Possible determinism bug | **5 now** |

## 1. Is more than 1/3 of the power offline?

```bash
ansible validators -b -m command -a "systemctl is-active xdevs" -o
ansible validators -b -m shell -a "journalctl -u xdevs -n 50 --no-pager -o cat | tail -n 20"
```

- **Process down / crash loop**: read the last log lines before the exit.
  - `out of memory` / OOM kill (`journalctl -k | grep -i oom`): the A1 has 6 GB. Check mempool
    size and RPC load. Restart that one node.
  - Disk full (`df -h /`): free space (journal, old `/opt/xdevs/releases`, snapshot staging
    `/var/lib/xdevs-snapshots`), then restart.
  - Refuses to start with a head state-root mismatch: the database is corrupted or diverged.
    Go to 5 if more than one node shows it; else restore that node
    ([restore-from-snapshot.md](restore-from-snapshot.md)).
  - Panic: capture the log. Restart once. If it panics at the same height again, go to 5.
  - `node_safety_failure` / WAL error: usually disk. Fix the disk, then restart.
- **Host down / unreachable**: check the Oracle console. Was the instance stopped, reclaimed or
  under maintenance? See [oracle-always-free.md](oracle-always-free.md). Start it. The node
  starts on boot.
- Restart nodes **one by one**:
  `ansible-playbook rolling-upgrade.yml -e xdevs_force_restart=true --limit validator-N`.
  The quorum check would refuse a healthy-node restart while 2 are down. For the **down**
  nodes, `--limit` them individually: the target is already counted as offline.

When 3 of 4 run again, blocks resume by themselves within a few rounds.

## 2. Can they reach each other (peers)?

```bash
ansible validators -b -m shell -a "curl -s -H 'content-type: application/json' --data '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"net_peerCount\",\"params\":[]}' http://127.0.0.1:8545"
ansible validators -b -m shell -a "nft list table inet xdevs_filter | grep -E '30333|30334'"
```

- 0 or 1 peers everywhere: check the firewalls. **Both** Oracle layers: the VCN security list
  (console) and nftables on the host (`nft list ruleset`). After an OS image change, check that
  `/etc/iptables/rules.v4` lost its final INPUT REJECT (`roles/common` does this).
- The VCN private IPs changed (instance re-created): update `private_ip` in `hosts.yml` and run
  `site.yml` (it re-renders `persistent_peers`).
- Peer id mismatch: `p2p_peer_id` in the inventory must match that host's `node_key.json`.
  Check the logs for dial errors mentioning a peer id.
- A node behind NAT (rpc-1) only dials out. Fine for rpc-1, never for a validator.

## 3. Are the clocks right?

```bash
ansible validators -b -m command -a "chronyc tracking" | grep -E "SUCCESS|System time|Leap status"
```

The offset must be well under 100 ms, with `Leap status: Normal`. Proposals more than
`max_timestamp_drift_secs` (2 s) ahead of a validator's clock are rejected. A validator with a
fast clock sees every proposal as fine, but its own proposals are rejected by the others, so
rounds climb when it is the proposer. Fix: `systemctl restart chrony`, `chronyc makestep`, then
check the `chrony_servers` settings.

## 4. Stuck in rounds, but everyone is up?

Look at `xdevs_consensus_round` and `xdevs_consensus_rounds_total` in Grafana:

- Rounds climb only when one specific validator is the proposer: that proposer is slow or
  isolated. Check its `block_build_duration_seconds`. Proposer build, streaming and
  re-execution must fit within `timeout_propose` (1 s). Check CPU steal and load on the A1.
- Every round fails: votes are not reaching quorum (go back to 2), or validators reject every
  proposal. Look for `invalid proposal` / validation errors in the logs. If they appear on
  **every** node for the same block, go to 5.

## 5. State-root mismatch or suspected determinism bug: stop the line

Signs: `FORK:`, state root mismatch, or `sign guard:` refusals in the logs. Nodes disagree on a
block hash or root at the same height (`scripts/check.sh` exit 1). Or validators reject the same
proposal repeatedly for an execution reason.

1. **Do not restart nodes in a loop, and do not wipe data.** Preserve the evidence: copy the
   logs, and run `scripts/check.sh --samples 50` from a VCN host (with `XDEVS_RPC_URLS`) to find
   the first height of disagreement.
2. If honest validators disagree on execution: **emergency halt**. Stop all validators so that
   nothing more is signed on diverging state:
   ```bash
   ansible validators -b -m systemd -a "name=xdevs state=stopped"
   ```
   (This deliberately takes 100% down. It is the one sanctioned exception to the 1/3 rule.)
   Stop `cloudflared` on rpc-1 too, or post a status-page notice.
3. Keep a copy of `data/` from at least two nodes that disagree (`tar` it to the staging dir)
   before any recovery.
4. Open a private security advisory ([security-report.md](security-report.md)). Fix and test
   on a devnet: re-execute the recorded blocks on x86_64 **and** ARM64 (plan §26
   "Determinism").
5. Recover with [emergency-restart.md](emergency-restart.md).

## 6. Single node behind or stuck (network fine)

- `eth_syncing` true and catching up: wait. ValueSync re-executes every block.
- Stuck: restart that node. This is safe for 1 node while the others are healthy:
  `ansible-playbook rolling-upgrade.yml -e xdevs_force_restart=true --limit <host>`.
- Still stuck, or a corrupted DB: [restore-from-snapshot.md](restore-from-snapshot.md).
- rpc-1: also check `systemctl status cloudflared`, the home internet connection, and the
  Cloudflare dashboard (Zero Trust -> Tunnels shows the connector state).

## Afterwards

- Write a post-mortem: timeline, root cause, detection time, what the alerts missed.
- File issues for missing metrics or alerts. Update this checklist.
