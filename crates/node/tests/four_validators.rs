//! Four in-process validator nodes (WP2 exit test): generated Ed25519 consensus keys, a
//! genesis with 4 equal-power validators, ephemeral ports and temp dirs. They produce
//! real EVM blocks through Malachite consensus; a transaction sent to node 1's RPC is
//! gossiped, included and visible on node 4; every node has the same block hash and
//! state root at every common height, and serves the commit certificates.
//!
//! Tx-gossip bootnodes are mutual (every node lists every other one), so early dials hit
//! ports that are not listening yet; the network retries them on a fresh local port.

use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use serde_json::Value;
use std::{
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};
use xdevs_consensus::ConsensusTimeouts;
use xdevs_crypto::{
    sign_tx_envelope, signer_from_hex, ConsensusKeystore, Ed25519Seed, ScryptParams,
};
use xdevs_genesis::{Genesis, GenesisValidator, DEV_PRIVATE_KEYS};
use xdevs_node::{Node, NodeConfig, NodeHandle, NodeMode};
use xdevs_primitives::{
    constants::{DEV_CHAIN_ID, GWEI},
    hex, Address, Bytes, Encodable2718, TxEip1559, TxKind, U256,
};

const N: usize = 4;
const BLOCK_TIME_MS: u64 = 500;

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

struct TestNode {
    handle: NodeHandle,
    rpc: HttpClient,
}

impl TestNode {
    async fn call(&self, method: &str, params: jsonrpsee::core::params::ArrayParams) -> Value {
        self.rpc
            .request(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    async fn head(&self) -> u64 {
        let v = self.call("eth_blockNumber", rpc_params![]).await;
        u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    async fn block(&self, n: u64) -> Value {
        self.call(
            "eth_getBlockByNumber",
            rpc_params![format!("{n:#x}"), false],
        )
        .await
    }
}

async fn heads(nodes: &[TestNode]) -> Vec<u64> {
    let mut out = Vec::new();
    for n in nodes {
        out.push(n.head().await);
    }
    out
}

async fn wait_heads(nodes: &[TestNode], height: u64, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        for (i, n) in nodes.iter().enumerate() {
            assert!(n.handle.consensus_running(), "node {i} consensus stopped");
        }
        let h = heads(nodes).await;
        if h.iter().all(|x| *x >= height) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "nodes did not reach height {height}: {h:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_validators_produce_identical_evm_blocks() {
    let started = Instant::now();
    let root = tempfile::tempdir().unwrap();
    let ports = free_ports(2 * N);
    let (p2p, gossip) = ports.split_at(N);
    let seeds: Vec<Ed25519Seed> = (0..N)
        .map(|i| Ed25519Seed::from_bytes(&[0x40 + i as u8; 32]))
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
        // Node 1 uses an encrypted keystore, the others a hex seed file.
        let key_file: PathBuf = home.join("consensus.key");
        let mut config = NodeConfig::default();
        if i == 0 {
            let ks = ConsensusKeystore::encrypt(&seeds[i], "secret", ScryptParams::LIGHT).unwrap();
            std::fs::write(&key_file, ks.to_json_string()).unwrap();
            let pw = home.join("password");
            std::fs::write(&pw, "secret\n").unwrap();
            config.validator.password_file = Some(pw);
        } else {
            std::fs::write(&key_file, hex::encode(seeds[i].as_bytes())).unwrap();
        }
        config.chain.genesis = Some(genesis_path.clone());
        config.chain.data_dir = home.join("data");
        config.validator.enabled = true;
        config.validator.key_file = Some(key_file);
        config.validator.fee_recipient = Address::repeat_byte(0xf0 + i as u8);
        config.consensus = ConsensusTimeouts::fast_for_tests(BLOCK_TIME_MS);
        config.p2p.listen = addr(p2p[i]);
        config.p2p.persistent_peers = (0..N).filter(|j| *j != i).map(|j| addr(p2p[j])).collect();
        config.gossip.network.listen = addr(gossip[i]);
        // Mutual bootnodes, including nodes that are not running yet.
        config.gossip.network.bootnodes = (0..N)
            .filter(|j| *j != i)
            .map(|j| addr(gossip[j]))
            .collect();
        config.rpc.http = "127.0.0.1:0".parse().unwrap();

        let node = Node::new(config).unwrap();
        assert_eq!(node.mode(), NodeMode::Validator);
        let handle = node.start().await.unwrap();
        let rpc = HttpClientBuilder::default()
            .request_timeout(Duration::from_secs(20))
            .build(handle.http_url())
            .unwrap();
        nodes.push(TestNode { handle, rpc });
    }

    wait_heads(&nodes, 2, Duration::from_secs(40)).await;
    // Tx gossip: every node sees the others on the tx topic.
    let deadline = Instant::now() + Duration::from_secs(20);
    while nodes.iter().any(|n| n.handle.gossip_topic_peers() < N - 1) {
        assert!(
            Instant::now() < deadline,
            "gossip mesh incomplete: {:?}",
            nodes
                .iter()
                .map(|n| n.handle.gossip_topic_peers())
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A transfer sent to node 1 is gossiped, included and visible on node 4.
    let to = Address::repeat_byte(0x77);
    let value = U256::from(10u64).pow(U256::from(18u64));
    let tx = sign_tx_envelope(
        &signer_from_hex(DEV_PRIVATE_KEYS[0]).unwrap(),
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to: TxKind::Call(to),
            value,
            input: Bytes::new(),
            access_list: Default::default(),
        },
    )
    .unwrap();
    let raw = format!("0x{}", hex::encode(tx.encoded_2718()));
    let hash = nodes[0]
        .call("eth_sendRawTransaction", rpc_params![raw])
        .await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let receipt = loop {
        let r = nodes[3]
            .call("eth_getTransactionReceipt", rpc_params![hash.clone()])
            .await;
        if !r.is_null() {
            break r;
        }
        assert!(Instant::now() < deadline, "tx not included on node 4");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(receipt["status"], "0x1");
    // It reached the other pools through tx gossip (validated pool admission).
    let gossiped: u64 = nodes[1..]
        .iter()
        .map(|n| {
            n.handle
                .gossip_stats()
                .unwrap()
                .accepted
                .load(std::sync::atomic::Ordering::Relaxed)
        })
        .sum();
    assert!(gossiped >= 1, "tx was not gossiped");
    let included = u64::from_str_radix(
        receipt["blockNumber"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let balance = nodes[3]
        .call("eth_getBalance", rpc_params![to, "latest"])
        .await;
    assert_eq!(balance.as_str().unwrap(), format!("{value:#x}"));

    wait_heads(&nodes, included + 2, Duration::from_secs(30)).await;

    // Same block hash and state root on every node at every common height.
    let min = *heads(&nodes).await.iter().min().unwrap();
    let mut proposers = std::collections::BTreeSet::new();
    for h in 1..=min {
        let reference = nodes[0].block(h).await;
        proposers.insert(reference["miner"].as_str().unwrap().to_string());
        for (i, n) in nodes.iter().enumerate().skip(1) {
            let b = n.block(h).await;
            assert_eq!(b["hash"], reference["hash"], "height {h}, node {}", i + 1);
            assert_eq!(
                b["stateRoot"],
                reference["stateRoot"],
                "height {h}, node {}",
                i + 1
            );
        }
    }
    assert!(proposers.len() > 1, "proposers rotate: {proposers:?}");

    // Commit certificates are served (>= 3 of 4 signatures, for the block's hash).
    let block = nodes[3].block(included).await;
    let cert = nodes[3]
        .call(
            "xdevs_getCommitCertificate",
            rpc_params![format!("{included:#x}")],
        )
        .await;
    assert_eq!(cert["blockHash"], block["hash"]);
    assert_eq!(cert["height"], format!("{included:#x}"));
    assert!(cert["signatures"].as_array().unwrap().len() >= 3, "{cert}");
    let genesis_cert = nodes[3]
        .call("xdevs_getCommitCertificate", rpc_params!["0x0"])
        .await;
    assert!(genesis_cert.is_null());

    // Peers are connected and nobody is syncing.
    for n in &nodes {
        let peers = n.call("net_peerCount", rpc_params![]).await;
        assert_ne!(peers, "0x0", "{peers}");
        assert_eq!(
            n.call("eth_syncing", rpc_params![]).await,
            Value::Bool(false)
        );
    }

    for n in nodes {
        assert!(n.handle.consensus_running());
        drop(n.rpc);
        n.handle.shutdown().await;
    }
    eprintln!(
        "4 validators: {min} common blocks in {:.1} s",
        started.elapsed().as_secs_f64()
    );
}

/// A consensus engine that stops with an error halts the node: the error surfaces
/// through `NodeHandle::consensus_halted` and nothing restarts the engine.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_error_halts_the_node() {
    let root = tempfile::tempdir().unwrap();
    let seed = Ed25519Seed::from_bytes(&[0x55; 32]);
    let mut genesis = Genesis::dev(DEV_CHAIN_ID);
    genesis.validators = vec![GenesisValidator {
        name: Some("solo".into()),
        consensus_key: seed.public_key(),
        power: 1,
        p2p: None,
        extra: Default::default(),
    }];
    let genesis_path = root.path().join("genesis.json");
    std::fs::write(&genesis_path, genesis.to_json_string()).unwrap();
    let key_file = root.path().join("consensus.key");
    std::fs::write(&key_file, hex::encode(seed.as_bytes())).unwrap();

    let mut config = NodeConfig::default();
    config.chain.genesis = Some(genesis_path);
    config.chain.data_dir = root.path().join("data");
    config.validator.enabled = true;
    config.validator.key_file = Some(key_file);
    config.gossip.enabled = false;
    config.rpc.http = "127.0.0.1:0".parse().unwrap();
    // The engine refuses this at start (fatal configuration error).
    config.p2p.listen = "not a multiaddr".into();

    let mut handle = Node::new(config).unwrap().start().await.unwrap();
    let err = tokio::time::timeout(Duration::from_secs(30), handle.consensus_halted())
        .await
        .expect("engine stops");
    let msg = format!("{err:#}");
    assert!(msg.contains("consensus engine halted"), "{msg}");
    assert!(msg.contains("p2p.listen"), "{msg}");
    assert!(!handle.consensus_running());
    handle.shutdown().await;
}
