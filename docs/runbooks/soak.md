# Runbook: soak tests

A soak test runs the chain under continuous load for hours. It watches for forks, state-root
mismatches and stalls. The plan's bar for "testnet ready" is 7 days on a devnet with continuous
load and random node kills, with zero consensus failures and zero root mismatches (plan §26).
The Phase-2 exit includes a 24 h soak.

Tooling:

- `scripts/soak.sh` sends load to a **running** network and runs periodic checks. It writes
  `summary.json` and exits non-zero when the run fails.
- `scripts/check.sh` runs the safety and liveness checks (see [localnet.md](localnet.md)).
- `xdevs-loadgen` is the load generator. If it is not available, or exits within 15 s, soak.sh
  falls back to one `xdevs-cli send --wait` loop per dev account 5 to 9, which gives about
  5 tx per block. The summary records which load ran (`load.mode`).

## Pass criteria

A soak **passes** only if all of these hold:

| Criterion | Measured by |
|---|---|
| **Zero forks**: every node has the same block hash at every checked height | check.sh every `--interval`, plus a final scan of 50 heights over the whole chain |
| **Zero state-root mismatches**: same `stateRoot` at every checked height | the same checks |
| **Zero certificate mismatches**: each commit certificate is for the block the node stores | the same checks |
| **No safety alerts in the logs**: no `sign guard:`, `equivocation evidence` or `FORK:` WARN/ERROR lines, no `consensus halted`, no panic | final log scan |
| **No stall longer than X s**: the longest time without a new block is at most `--stall-limit` (default **X = 30 s**, the plan's critical alert, §24.7; that is 15 to 30 block times at 1 to 2 s blocks) | the head is polled every `--poll` (5 s) |
| The final deep check passed, and the run was not aborted | `final_check.code == 0`, `aborted == null` |
| The load ran for the whole run (unless `--no-load`) | `load.ok` |

The run aborts early on the first safety failure, or after `--max-consecutive` (5) failed checks
in a row. A single progress window without a block is not a failure by itself; the stall limit
decides.

Restarted nodes also re-verify their head state root at startup, and refuse to start on a
mismatch. That makes every planned or chaos restart an extra root check.

## Short soak in CI (up to 5.5 h)

Actions → **soak** → *Run workflow*:

- `duration_minutes`: 1 to 330. GitHub-hosted jobs stop after 6 h, and the workflow shortens the
  soak if the image build used part of that budget.
- `tps`, `block_time`.
- `image`: a published tag (e.g. `ghcr.io/jacell100/xdevs-chain:main`, or `sha-<commit>`) skips
  the 15 to 30 minute build. Use it for long runs.

The job starts 4 validators (`scripts/localnet-up.sh --fresh`), runs `scripts/soak.sh`, writes a
pass/fail table to the job summary, and uploads `soak-<run id>` (`summary.json`, every check's
JSON in `checks.jsonl`, the load log, and `logs.tar.gz` with every node's log). The nightly
schedule exists but is commented out in `.github/workflows/soak.yml`. Uncomment the `schedule:`
block to enable it.

A CI soak runs all 4 validators on one 4-vCPU runner. That tests correctness under load, not
real-network performance.

## 24 h soak on the Oracle VMs

The real soak runs on the testnet topology (plan §24.2): 4 Ampere A1 validators on Oracle Cloud,
talking over the private VCN subnet.

1. **Deploy** the same release on all 4 VMs (GitHub Release tarball or the GHCR image) with the
   systemd unit in `deploy/systemd/`. Generate the network once with
   `xdevs-cli testnet init --validators 4 --host <...>`, or with per-VM `keygen` and a shared
   genesis. Each VM gets only its own `nodeK/` directory and keys.
2. **Let validators answer RPC on the private subnet only** (`[rpc] http = "10.0.0.1x:8545"`).
   Never expose it publicly (plan §24.3).
3. **Pick a driver host**: the bootnode micro VM, or any validator. It needs `jq`, `curl`, the
   repo's `scripts/`, and `xdevs-loadgen` plus `xdevs-cli` on `PATH` (from the release tarball).
   It also needs SSH access to the validators for log collection.
4. **Run** inside `tmux` so the run survives a dropped SSH session:

   ```bash
   export XDEVS_RPC_URLS="v1=http://10.0.0.11:8545 v2=http://10.0.0.12:8545 v3=http://10.0.0.13:8545 v4=http://10.0.0.14:8545"
   scripts/check.sh --wait-height 1            # sanity: all four reachable, no fork
   scripts/soak.sh --mode host --duration 24h --tps 20 --stall-limit 30 \
     --loadgen-rpc http://10.0.0.11:8545 --artifacts ~/soak-$(date -u +%F) \
     --log-cmd 'for h in v1 v2 v3 v4; do ssh "$h" journalctl -u xdevs --since "-25h" -o cat; done'
   ```

   Spread the load across validators by running a second `xdevs-loadgen` against
   `http://10.0.0.13:8545` by hand.
5. **Chaos during the soak.** For the Phase-2 exit and the plan's 7-day bar, inject faults while
   the soak runs. Restart **at most one validator at a time** (never take more than 1/3 of the
   power down):

   ```bash
   # e.g. every ~2 h from the driver host, for a random validator
   ssh v3 sudo systemctl kill -s SIGKILL xdevs; sleep 60; ssh v3 sudo systemctl start xdevs
   ssh v2 sudo tc qdisc add dev enp0s6 root netem delay 150ms 50ms loss 2%; sleep 600; ssh v2 sudo tc qdisc del dev enp0s6 root
   ```

   A restart can cause one short stall while the remaining 3 validators move through rounds.
   Raise `--stall-limit` (e.g. to 60) only if the run plan includes outages that are expected to
   take longer, and record that in the report.
6. **Monitoring.** Watch the Grafana dashboard (plan §24.7) during the run: block height, rounds
   > 0, peers, mempool size, `build_ms` / `validate_ms`, CPU, RAM and disk.
7. **Report.** Archive the artifacts directory (`summary.json`, `checks.jsonl`,
   `node-logs.txt`). Note the commit, the TPS, every injected fault with its time, and the
   longest stall. The run counts toward the exit criterion only if `summary.json` shows
   `"pass": true`.

## Reading summary.json

```json
{
  "pass": true,
  "elapsed_secs": 86400, "blocks": 43012, "avg_block_time_secs": 2.009,
  "forks": 0, "root_mismatches": 0, "cert_mismatches": 0,
  "max_stall_secs": 9, "stall_limit_secs": 30,
  "checks": 1410, "failed_checks": 0,
  "log_alerts": {"sign_guard_refusals": 0, "equivocations": 0, "forks": 0, "consensus_halted": 0, "panics": 0},
  "load": {"mode": "loadgen", "ok": true, "tps": 20, "detail": "..."},
  "final_check": {"code": 0, "checked_heights": [0, 843, 1686, "..."], "failures": []}
}
```

These numbers are illustrative. On a failure, start with `aborted`, then `final_check.failures`.
Then look in `checks.jsonl` for the first check with `"ok": false`. Its `failures` name the
height and the nodes that disagree.

**A fork or root mismatch is a stop-the-line bug.** Keep the data directories of the nodes that
disagree, file an issue with the heights and hashes, and follow the "chain halted / state-root
mismatch" checklist in [README.md](README.md).
