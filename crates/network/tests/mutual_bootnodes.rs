//! Regression test: nodes that list EACH OTHER as bootnodes and start at different times
//! (so early dials hit ports that are not listening yet) must still all join the tx topic,
//! and a tx published before anyone was subscribed must not be lost.

use std::{
    net::TcpListener,
    sync::Arc,
    time::{Duration, Instant},
};
use xdevs_network::{start, Keypair, NetworkConfig, NetworkHandle, Verdict};
use xdevs_primitives::Bytes;

const NODES: usize = 4;
const CHAIN_ID: u64 = 7117;

/// A TCP port that was free a moment ago (the test binds it shortly after).
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mutual_bootnodes_with_staggered_start_form_a_full_mesh() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let keys: Vec<Keypair> = (0..NODES).map(|_| Keypair::generate_ed25519()).collect();
    let ports: Vec<u16> = (0..NODES).map(|_| free_port()).collect();
    let addr = |i: usize| {
        format!(
            "/ip4/127.0.0.1/tcp/{}/p2p/{}",
            ports[i],
            keys[i].public().to_peer_id()
        )
    };
    let accept_all: xdevs_network::TxValidator = Arc::new(|_: &[u8]| Verdict::Accept);

    let mut nodes: Vec<NetworkHandle> = Vec::new();
    // Each node's inbox is opened as soon as the node starts, so nothing delivered to it
    // later is missed (inbound() only sees txs that arrive after it is called).
    let mut inboxes = Vec::new();
    for i in 0..NODES {
        let config = NetworkConfig {
            listen: format!("/ip4/127.0.0.1/tcp/{}", ports[i]),
            // Every OTHER node, including ones that are not listening yet.
            bootnodes: (0..NODES).filter(|&j| j != i).map(addr).collect(),
            ..NetworkConfig::default()
        };
        let node = start(config, CHAIN_ID, keys[i].clone(), accept_all.clone())
            .await
            .expect("start node");
        inboxes.push(node.inbound());
        nodes.push(node);
        // Node 0 publishes before any peer exists: it must not be lost.
        if i == 0 {
            nodes[0]
                .publish_tx(Bytes::from_static(b"early-tx"))
                .expect("publish");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    while !nodes.iter().all(|n| n.topic_peer_count() == NODES - 1) {
        let counts: Vec<(usize, usize)> = nodes
            .iter()
            .map(|n| (n.peer_count(), n.topic_peer_count()))
            .collect();
        assert!(
            Instant::now() < deadline,
            "mesh incomplete, (connected peers, topic peers) per node: {counts:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A tx published now reaches every other node.
    nodes[3]
        .publish_tx(Bytes::from_static(b"late-tx"))
        .expect("publish");
    for (i, inbox) in inboxes.iter_mut().enumerate() {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        // The early tx is retried until the first peer subscribes (node 1) and is then
        // published once; gossip does not replay it to nodes that join later (syncing
        // pending txs to late joiners is a mempool-sync concern, not gossip's).
        let wanted: &[&[u8]] = match i {
            1 => &[b"early-tx", b"late-tx"],
            3 => &[],
            _ => &[b"late-tx"],
        };
        while !wanted
            .iter()
            .all(|w| seen.iter().any(|s: &Bytes| s.as_ref() == *w))
        {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, inbox.recv()).await {
                Ok(Some((_, tx))) => seen.push(tx),
                _ => panic!("node {i} did not receive {wanted:?}; got {seen:?}"),
            }
        }
    }

    for n in nodes {
        n.shutdown().await;
    }
}
