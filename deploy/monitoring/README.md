# Monitoring: xDEVS Chain testnet

Plan §24.7. The pipeline is:

1. Each node serves Prometheus `/metrics` (`[metrics] enabled = true`,
   `listen = "<ip>:9100"`, rendered by `roles/xdevs_node`).
2. Grafana Alloy on the same host (`roles/monitoring_agent`) scrapes the node every 15 s and its
   own host exporter every 60 s.
3. Alloy remote-writes to **Grafana Cloud** (free tier: about 10k active series, 14-day
   retention) and ships the `xdevs` / `cloudflared` journal to Loki.
4. Alert rules run in Grafana Cloud. External checks run in **UptimeRobot**.

| File | What |
|---|---|
| [grafana/xdevs-overview.json](grafana/xdevs-overview.json) | Dashboard. Import it via Dashboards -> New -> Import and pick the Grafana Cloud Prometheus data source |
| [prometheus/alerts.yml](prometheus/alerts.yml) | Alert rules (Prometheus format; `mimirtool rules load` or Grafana's import) |
| [uptimerobot.md](uptimerobot.md) | External monitors and the public status page |

Labels on every series: `env` (`testnet`), `role` (`validator` | `rpc` | `bootnode`),
`instance` (inventory host name), and `job` (`xdevs` for node metrics, `node` for host
metrics). The label is `env`, not `network`, because `xdevs_peers` already carries
`network="consensus"|"gossip"`.

## Metric names (reconciliation table)

Names below are what `crates/metrics` registers today (prefix `xdevs_`, `prometheus-client`
0.23, OpenMetrics text). Counters gain `_total` and histograms gain `_bucket` / `_sum` /
`_count` when exposed. The dashboard and alerts use exactly these names. **If you rename a
metric in the crate, update this table, the dashboard and `alerts.yml` in the same PR.**

| Plan §24.7 name | Exported name(s) | Type | Labels | Used by |
|---|---|---|---|---|
| `xdevs_block_height` | `xdevs_block_height` | gauge | | dashboard, `XdevsNoNewBlock`, `XdevsNodeLagging`, snapshot preflight |
| `xdevs_block_time_seconds` | `xdevs_block_time_seconds_{bucket,sum,count}` | histogram (0.25 to 30 s) | | dashboard |
| `xdevs_finality_latency_seconds` | `xdevs_finality_latency_seconds_{bucket,sum,count}` | histogram | | dashboard |
| (consensus height) | `xdevs_consensus_height` | gauge | | dashboard, rolling-upgrade signing check |
| (current round) | `xdevs_consensus_round` | gauge | | dashboard, `XdevsConsensusRoundClimbing` |
| `xdevs_consensus_rounds_total` | `xdevs_consensus_rounds_total` | counter (extra rounds) | | dashboard, `XdevsConsensusExtraRounds` |
| `xdevs_tx_throughput` | `xdevs_transactions_total` | counter; throughput = `rate()` | | dashboard |
| `xdevs_exec_block_duration_seconds` | `xdevs_block_build_duration_seconds_*` (proposer) and `xdevs_block_validate_duration_seconds_*` (re-execution) | histograms | | dashboard |
| (commit time) | `xdevs_block_commit_duration_seconds_*` | histogram | | dashboard |
| `xdevs_state_root_duration_seconds` | `xdevs_state_root_duration_seconds_*` | histogram | | dashboard |
| `xdevs_peers` | `xdevs_peers` | gauge | `network` = `consensus` / `gossip` | dashboard, `XdevsLowPeers` |
| `xdevs_mempool_size` | `xdevs_mempool_size` | gauge | `state` = `pending` / `queued` | dashboard, `XdevsMempoolNearlyFull` |
| `xdevs_rpc_requests_total{method}` | `xdevs_rpc_requests_total` | counter | `method`, `result` (`ok` / error) | dashboard, `XdevsRpcErrorRate` |
| `xdevs_rpc_latency_seconds` | `xdevs_rpc_latency_seconds_*` | histogram | `method` | dashboard, `XdevsRpcSlow` |
| **`xdevs_missed_votes_total`** | **not exported yet** | proposed: counter | proposed: `validator` (address or moniker) | dashboard panel "PENDING", `XdevsValidatorMissedVotes` (evaluates to nothing until it exists) |
| (proposed) | `xdevs_votes_signed_total` | counter, this node's signed prevotes and precommits | | would make `rolling-upgrade.yml` prove signing (`xdevs_sign_check_metric`) |
| (proposed) | `xdevs_build_info{version}` | gauge = 1 | `version` | version rollout panel |
| (from snapshot.sh) | `xdevs_snapshot_last_success_timestamp_seconds`, `..._last_height`, `..._last_size_bytes`, `..._last_downtime_seconds`, `..._last_run_success` | gauges (Alloy textfile collector) | | dashboard, `XdevsSnapshotStale` |
| system | `node_cpu_seconds_total`, `node_memory_MemAvailable_bytes`, `node_memory_MemTotal_bytes`, `node_filesystem_{avail,size}_bytes`, `node_network_{receive,transmit}_bytes_total`, `node_timex_offset_seconds`, `node_timex_sync_status`, `node_boot_time_seconds`, `node_load1` | Alloy `prometheus.exporter.unix` | `job="node"` | dashboard, host alerts |

Malachite's own engine metrics (`malachitebft_core_consensus_*`) are not registered in the
node's registry today, so they are not exported. If they are added, extend the keep-list in
`roles/monitoring_agent/defaults/main.yml`. Its default is `xdevs_.*`, so names under the
`xdevs_` registry prefix flow through automatically.

**Cardinality.** `xdevs_rpc_*` carries a `method` label. The crate maps unknown or long method
names to `"other"`, which bounds the label values. 7 nodes x ~40 xdevs series, plus histograms
and ~20 host series each, stays well under the free 10k.

## Alerts (plan §24.7) and where they live

| Plan alert | Rule | Severity |
|---|---|---|
| No new block for 30 s | `XdevsNoNewBlock` (rule group interval 15 s, needs the 15 s scrape) | critical |
| Validator missed > 10% votes in last 1,000 blocks | `XdevsValidatorMissedVotes` (**pending metric**) | warning |
| Disk > 80% | `XdevsDiskUsageHigh` (+ `XdevsDiskAlmostFull` at 92%, critical) | warning |
| Peers < 3 | `XdevsLowPeers` | warning |
| RPC down | UptimeRobot (see [uptimerobot.md](uptimerobot.md)) | critical |
| (extra) > 1/3 of validators down | `XdevsQuorumLost` | critical |
| (extra) consensus rounds, round climbing, node lagging, clock skew, memory, snapshot stale, reboot, RPC error rate / latency, metrics absent | see `alerts.yml` | mixed |

Contact points (Grafana Cloud -> Alerting): one Discord webhook (`#ops-alerts`) for everything,
plus email for `severity=critical`. Webhook URLs are secrets. Keep them in Grafana only, never
in this repo.

## Importing

```bash
# dashboard: Grafana Cloud UI -> Dashboards -> New -> Import -> upload grafana/xdevs-overview.json
# alert rules (mimirtool: https://grafana.com/docs/mimir/latest/manage/tools/mimirtool/):
mimirtool rules check deploy/monitoring/prometheus/alerts.yml
mimirtool rules load deploy/monitoring/prometheus/alerts.yml --address="$GC_PROM_URL" --id="$GC_PROM_USER" --key="$GC_RULES_TOKEN"
```
