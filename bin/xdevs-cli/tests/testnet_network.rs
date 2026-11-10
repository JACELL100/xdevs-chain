//! `xdevs-cli testnet init --validators 4 --full-nodes 1` produces a working network:
//! the real binary writes the files, then the 5 nodes are started in-process from those
//! exact `config.toml` files (`NodeConfig::load` + `Node::new`), reach height 5 and agree
//! on every block hash.

use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use serde_json::Value;
use std::{
    net::TcpListener,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use xdevs_node::{Node, NodeConfig, NodeHandle, NodeMode};

const VALIDATORS: usize = 4;
const FULL_NODES: usize = 1;
const TARGET_HEIGHT: u64 = 5;
/// Must match `testnet::PORTS_PER_NODE` and the per-node offsets (p2p, gossip, rpc, metrics).
const PORTS_PER_NODE: u16 = 10;
const USED_OFFSETS: [u16; 4] = [0, 1, 2, 3];

/// A base port whose per-node ports are all free right now.
fn free_base_port(nodes: usize) -> u16 {
    let start = 20_000 + (std::process::id() % 1_000) as u16 * 20;
    let mut base = start;
    loop {
        let ports: Vec<u16> = (0..nodes as u16)
            .flat_map(|k| USED_OFFSETS.map(|o| base + PORTS_PER_NODE * k + o))
            .collect();
        let listeners: Result<Vec<_>, _> = ports
            .iter()
            .map(|p| TcpListener::bind(("127.0.0.1", *p)))
            .collect();
        if listeners.is_ok() {
            return base;
        }
        base += 97;
        assert!(base < 60_000, "no free port range");
    }
}

async fn head(rpc: &HttpClient) -> u64 {
    let v: Value = rpc.request("eth_blockNumber", rpc_params![]).await.unwrap();
    u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

async fn block_hash(rpc: &HttpClient, n: u64) -> Value {
    let b: Value = rpc
        .request(
            "eth_getBlockByNumber",
            rpc_params![format!("{n:#x}"), false],
        )
        .await
        .unwrap();
    b["hash"].clone()
}

fn testnet_init(out: &Path, base_port: u16) {
    let output = Command::new(env!("CARGO_BIN_EXE_xdevs-cli"))
        .args(["testnet", "init", "--out"])
        .arg(out)
        .args([
            "--validators",
            &VALIDATORS.to_string(),
            "--full-nodes",
            &FULL_NODES.to_string(),
            "--block-time",
            "500ms",
            "--base-port",
            &base_port.to_string(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stderr.contains("DEV / TESTNET ONLY"), "{stderr}");
    assert!(
        stdout.contains(&format!("http://127.0.0.1:{}", base_port + 2)),
        "{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generated_testnet_produces_identical_blocks() {
    let started = Instant::now();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("testnet");
    let total = VALIDATORS + FULL_NODES;
    testnet_init(&out, free_base_port(total));

    let mut nodes: Vec<(NodeHandle, HttpClient)> = Vec::new();
    for k in 0..total {
        let config = NodeConfig::load(&out.join(format!("node{k}")).join("config.toml")).unwrap();
        let rpc_addr = config.rpc.http;
        let node = Node::new(config).unwrap();
        let expected = if k < VALIDATORS {
            NodeMode::Validator
        } else {
            NodeMode::FullNode
        };
        assert_eq!(node.mode(), expected, "node{k}");
        let handle = node.start().await.unwrap();
        assert_eq!(handle.http_addr(), rpc_addr);
        let rpc = HttpClientBuilder::default()
            .request_timeout(Duration::from_secs(20))
            .build(handle.http_url())
            .unwrap();
        nodes.push((handle, rpc));
    }

    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        for (k, (h, _)) in nodes.iter().enumerate() {
            assert!(h.consensus_running(), "node{k} consensus stopped");
        }
        let mut heads = Vec::new();
        for (_, rpc) in &nodes {
            heads.push(head(rpc).await);
        }
        if heads.iter().all(|h| *h >= TARGET_HEIGHT) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "nodes did not reach height {TARGET_HEIGHT}: {heads:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for h in 1..=TARGET_HEIGHT {
        let reference = block_hash(&nodes[0].1, h).await;
        assert!(reference.is_string(), "height {h}: {reference}");
        for (k, (_, rpc)) in nodes.iter().enumerate().skip(1) {
            assert_eq!(block_hash(rpc, h).await, reference, "height {h}, node{k}");
        }
    }

    for (handle, rpc) in nodes {
        drop(rpc);
        handle.shutdown().await;
    }
    eprintln!(
        "testnet init network: height {TARGET_HEIGHT} on {total} nodes in {:.1} s",
        started.elapsed().as_secs_f64()
    );
}
