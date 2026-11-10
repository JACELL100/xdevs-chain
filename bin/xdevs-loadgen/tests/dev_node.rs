//! Integration: run the load generator for ~10 s against an in-process dev node (same
//! setup as tests/e2e: temp data dir, ephemeral RPC port, fast blocks).

use std::time::Duration;
use xdevs_loadgen::{run, Config, LoadKind, SenderSource};
use xdevs_node::{Node, NodeConfig};
use xdevs_primitives::Address;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loadgen_against_dev_node() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = NodeConfig::default();
    cfg.chain.data_dir = dir.path().to_path_buf();
    cfg.dev.enabled = true;
    cfg.dev.block_time_ms = Some(500);
    cfg.dev.fee_recipient = Address::repeat_byte(0xfe);
    cfg.rpc.http = "127.0.0.1:0".parse().unwrap();
    let node = Node::new(cfg).unwrap().start().await.unwrap();

    let report = run(Config {
        rpc: vec![node.http_url()],
        tps: 25.0,
        duration: Duration::from_secs(10),
        senders: 5,
        source: SenderSource::Dev,
        mix: "transfer=70,erc20=20,storage=10".parse().unwrap(),
        storage_slots: 4,
        drain: Duration::from_secs(20),
        poll_interval: Duration::from_millis(50),
        ..Config::default()
    })
    .await
    .expect("load run");
    println!("{}", report.human());
    node.shutdown().await;

    assert!(!report.interrupted);
    assert!(report.sent >= 200, "sent only {} (target 250)", report.sent);
    assert!(
        report.included_accepted as f64 > 0.9 * report.sent as f64,
        "only {} of {} sent txs included",
        report.included_accepted,
        report.sent
    );
    assert_eq!(report.receipts_failed, 0, "every included tx succeeds");
    assert_eq!(report.receipts_unchecked, 0);
    assert_eq!(report.receipts_ok, report.included);
    assert_eq!(report.latency_ms.count as u64, report.included);
    assert!(report.latency_ms.p50 > 0.0 && report.latency_ms.p99 < 10_000.0);
    assert!(report.blocks_seen > 5);
    for kind in LoadKind::ALL {
        let k = &report.by_kind[kind.name()];
        assert!(k.sent > 0 && k.included > 0, "{kind}: {k:?}");
    }
    let v: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(v["included"], report.included);
}
