//! RPC DoS regressions (security review "RPC load can starve consensus"):
//!
//! * an `eth_call` that never ends (an endless loop with no gas cap) fails with
//!   "execution aborted (timeout = 5s)" after the EVM time limit;
//! * a call whose WebSocket client disconnects is cancelled and frees its work slot;
//! * a 4-validator net keeps producing blocks on time while one validator's RPC is
//!   flooded with heavy `eth_call` / `eth_estimateGas` requests over many WebSocket
//!   connections (consensus work runs on its own threads; RPC work is capped).

use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
    ws_client::WsClientBuilder,
};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use xdevs_consensus::ConsensusTimeouts;
use xdevs_crypto::Ed25519Seed;
use xdevs_genesis::{Genesis, GenesisValidator, DEV_ACCOUNTS};
use xdevs_node::{Node, NodeConfig, NodeHandle};
use xdevs_primitives::{constants::DEV_CHAIN_ID, hex};

/// Init code `JUMPDEST; PUSH1 0; JUMP`: loops until it runs out of gas (or forever
/// without a gas cap). Sent as a contract-creation call, so nothing is deployed.
const ENDLESS_LOOP: &str = "0x5b600056";

fn endless_call() -> Value {
    json!({ "from": DEV_ACCOUNTS[0], "data": ENDLESS_LOOP })
}

fn free_ports(n: usize) -> Vec<u16> {
    let listeners: Vec<_> = (0..n)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    listeners
        .iter()
        .map(|l| l.local_addr().unwrap().port())
        .collect()
}

fn addr(port: u16) -> String {
    format!("/ip4/127.0.0.1/tcp/{port}")
}

fn http(handle: &NodeHandle) -> HttpClient {
    HttpClientBuilder::default()
        .request_timeout(Duration::from_secs(30))
        .build(handle.http_url())
        .unwrap()
}

fn ws_url(handle: &NodeHandle) -> String {
    format!("ws://{}", handle.http_addr())
}

async fn head(rpc: &HttpClient) -> u64 {
    let v: Value = rpc.request("eth_blockNumber", rpc_params![]).await.unwrap();
    u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

/// A dev-mode node (local producer) with RPC on an ephemeral port.
async fn dev_node(dir: &std::path::Path, tune: impl FnOnce(&mut NodeConfig)) -> NodeHandle {
    let mut config = NodeConfig::default();
    config.chain.data_dir = dir.join("data");
    config.dev.block_time_ms = Some(500);
    config.rpc.http = "127.0.0.1:0".parse().unwrap();
    tune(&mut config);
    Node::new(config).unwrap().start().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endless_eth_call_hits_the_evm_time_limit() {
    let dir = tempfile::tempdir().unwrap();
    // No gas cap: only the time limit can stop the loop.
    let node = dev_node(dir.path(), |c| c.rpc.gas_cap = 0).await;
    let rpc = http(&node);
    let started = Instant::now();
    let err = rpc
        .request::<Value, _>("eth_call", rpc_params![endless_call(), "latest"])
        .await
        .unwrap_err();
    let took = started.elapsed();
    let msg = err.to_string();
    assert!(msg.contains("execution aborted (timeout = 5s)"), "{msg}");
    assert!(
        took >= Duration::from_millis(4_900) && took < Duration::from_secs(9),
        "{took:?}"
    );
    // The node is fine afterwards.
    assert!(head(&rpc).await < u64::MAX);
    drop(rpc);
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_client_cancels_its_eth_call() {
    let dir = tempfile::tempdir().unwrap();
    // No gas cap, no time limit and a single work slot: only the cancellation can free
    // the slot the endless call takes.
    let node = dev_node(dir.path(), |c| {
        c.rpc.gas_cap = 0;
        c.rpc.evm_timeout_ms = 0;
        c.rpc.max_concurrent_requests = 1;
    })
    .await;
    let ws = WsClientBuilder::default()
        .build(ws_url(&node))
        .await
        .unwrap();
    let ws = Arc::new(ws);
    let caller = ws.clone();
    let call = tokio::spawn(async move {
        caller
            .request::<Value, _>("eth_call", rpc_params![endless_call(), "latest"])
            .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!call.is_finished());
    // The client goes away.
    call.abort();
    drop(ws);

    let rpc = http(&node);
    let started = Instant::now();
    head(&rpc).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the work slot was not freed: {:?}",
        started.elapsed()
    );
    drop(rpc);
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_flood_does_not_stall_consensus() {
    const N: usize = 4;
    const BLOCK_MS: u64 = 400;
    const BLOCKS: u64 = 12;
    let root = tempfile::tempdir().unwrap();
    let ports = free_ports(2 * N);
    let (p2p, gossip) = ports.split_at(N);
    let seeds: Vec<Ed25519Seed> = (0..N)
        .map(|i| Ed25519Seed::from_bytes(&[0x60 + i as u8; 32]))
        .collect();
    let mut genesis = Genesis::dev(DEV_CHAIN_ID);
    genesis.validators = seeds
        .iter()
        .enumerate()
        .map(|(i, s)| GenesisValidator {
            name: Some(format!("val{}", i + 1)),
            consensus_key: s.public_key(),
            power: 1,
            p2p: Some(addr(p2p[i])),
            extra: Default::default(),
        })
        .collect();
    let genesis_path = root.path().join("genesis.json");
    std::fs::write(&genesis_path, genesis.to_json_string()).unwrap();

    let mut nodes = Vec::new();
    for i in 0..N {
        let home = root.path().join(format!("val{}", i + 1));
        std::fs::create_dir_all(&home).unwrap();
        let key_file = home.join("consensus.key");
        std::fs::write(&key_file, hex::encode(seeds[i].as_bytes())).unwrap();
        let mut config = NodeConfig::default();
        config.chain.genesis = Some(genesis_path.clone());
        config.chain.data_dir = home.join("data");
        config.validator.enabled = true;
        config.validator.key_file = Some(key_file);
        config.consensus = ConsensusTimeouts::fast_for_tests(BLOCK_MS);
        config.p2p.listen = addr(p2p[i]);
        config.p2p.persistent_peers = (0..N).filter(|j| *j != i).map(|j| addr(p2p[j])).collect();
        config.gossip.network.listen = addr(gossip[i]);
        config.gossip.network.bootnodes = (0..N)
            .filter(|j| *j != i)
            .map(|j| addr(gossip[j]))
            .collect();
        config.rpc.http = "127.0.0.1:0".parse().unwrap();
        nodes.push(Node::new(config).unwrap().start().await.unwrap());
    }
    let rpcs: Vec<HttpClient> = nodes.iter().map(http).collect();
    let deadline = Instant::now() + Duration::from_secs(60);
    while head(&rpcs[0]).await < 3 {
        assert!(Instant::now() < deadline, "the net did not start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Flood validator 1: 8 WebSocket connections, each firing 64 heavy calls at a time
    // (50M-gas endless loops and estimates) for as long as the measurement runs.
    let stop = Arc::new(AtomicBool::new(false));
    let (done, limited, aborted) = (
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
    );
    let mut flood = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let ws = Arc::new(
            WsClientBuilder::default()
                .request_timeout(Duration::from_secs(60))
                .max_concurrent_requests(1024)
                .build(ws_url(&nodes[0]))
                .await
                .unwrap(),
        );
        let (stop, done, limited, aborted) =
            (stop.clone(), done.clone(), limited.clone(), aborted.clone());
        flood.spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                let mut wave = tokio::task::JoinSet::new();
                for k in 0..64 {
                    let ws = ws.clone();
                    let method = if k % 2 == 0 {
                        "eth_call"
                    } else {
                        "eth_estimateGas"
                    };
                    wave.spawn(async move {
                        ws.request::<Value, _>(method, rpc_params![endless_call()])
                            .await
                    });
                }
                while let Some(r) = wave.join_next().await {
                    done.fetch_add(1, Ordering::Relaxed);
                    if let Ok(Err(e)) = r {
                        let msg = e.to_string();
                        if msg.contains("too many concurrent requests") {
                            limited.fetch_add(1, Ordering::Relaxed);
                        } else if msg.contains("aborted") || msg.contains("busy") {
                            aborted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        });
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Progress of the flooded validator's own engine (its RPC is busy by design).
    let height = || nodes[0].consensus_status().unwrap().height();
    let h0 = height();
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(60);
    loop {
        let h = height();
        if h >= h0 + BLOCKS {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "consensus stalled under RPC load: {h0} -> {h} in {:?}",
            t0.elapsed()
        );
        for (i, n) in nodes.iter().enumerate() {
            assert!(n.consensus_running(), "node {i} consensus stopped");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let per_block = t0.elapsed() / BLOCKS as u32;
    stop.store(true, Ordering::Relaxed);
    eprintln!(
        "under RPC flood: {BLOCKS} blocks, {per_block:?} per block (target {BLOCK_MS} ms); \
         {} calls answered, {} refused by the per-connection cap, {} aborted / busy",
        done.load(Ordering::Relaxed),
        limited.load(Ordering::Relaxed),
        aborted.load(Ordering::Relaxed),
    );
    assert!(
        per_block <= Duration::from_millis(BLOCK_MS * 3),
        "block time {per_block:?} under RPC load (target {BLOCK_MS} ms)"
    );
    assert!(
        limited.load(Ordering::Relaxed) > 0,
        "the flood never hit the per-connection cap"
    );
    // The flooded validator itself kept up with the others.
    let others = nodes[1].consensus_status().unwrap().height();
    assert!(height() + 2 >= others, "{} vs {others}", height());

    flood.abort_all();
    while flood.join_next().await.is_some() {}
    drop(rpcs);
    for n in nodes {
        n.shutdown().await;
    }
}
