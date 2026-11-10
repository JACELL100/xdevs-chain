//! Tx-gossip DoS regressions (security review, plan §12.5):
//!
//! * a peer flooding txs that are free to forge (they are only `Ignore`d, e.g. unfunded
//!   senders) is rate-limited and then banned, while an honest peer's tx still arrives;
//! * peers that dial us cannot take every connection slot: our own bootnode dial still
//!   gets through.

use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use xdevs_network::{start, Keypair, NetworkConfig, NetworkHandle, TxValidator, Verdict};
use xdevs_primitives::Bytes;

const CHAIN_ID: u64 = 7117;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn addr_of(node: &NetworkHandle) -> String {
    format!("{}/p2p/{}", node.listen_addrs()[0], node.local_peer_id())
}

fn local(bootnodes: Vec<String>) -> NetworkConfig {
    NetworkConfig {
        listen: "/ip4/127.0.0.1/tcp/0".into(),
        bootnodes,
        ..NetworkConfig::default()
    }
}

async fn wait_until(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tx_flood_from_one_peer_is_rate_limited_and_banned() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    // The victim ignores "junk" (like an unfunded sender: no gossip penalty by itself)
    // and counts how many txs it had to validate.
    let validated = Arc::new(AtomicUsize::new(0));
    let counter = validated.clone();
    let victim_validator: TxValidator = Arc::new(move |tx: &[u8]| {
        counter.fetch_add(1, Ordering::Relaxed);
        if tx.starts_with(b"junk") {
            Verdict::Ignore
        } else {
            Verdict::Accept
        }
    });
    let accept_all: TxValidator = Arc::new(|_: &[u8]| Verdict::Accept);
    let (rate, burst) = (20u32, 40u32);
    let victim = start(
        NetworkConfig {
            tx_rate_per_peer: rate,
            tx_burst_per_peer: burst,
            ..local(vec![])
        },
        CHAIN_ID,
        Keypair::generate_ed25519(),
        victim_validator,
    )
    .await
    .unwrap();
    let mut inbox = victim.inbound();
    let attacker = start(
        local(vec![addr_of(&victim)]),
        CHAIN_ID,
        Keypair::generate_ed25519(),
        accept_all.clone(),
    )
    .await
    .unwrap();
    let honest = start(
        local(vec![addr_of(&victim)]),
        CHAIN_ID,
        Keypair::generate_ed25519(),
        accept_all,
    )
    .await
    .unwrap();
    wait_until("the mesh", Duration::from_secs(20), || {
        victim.topic_peer_count() == 2
            && attacker.topic_peer_count() >= 1
            && honest.topic_peer_count() >= 1
    })
    .await;

    // 600 distinct junk txs as fast as the attacker's queue takes them.
    let started = Instant::now();
    let mut sent = 0;
    while sent < 600 {
        match attacker.publish_tx(Bytes::from(format!("junk-{sent}").into_bytes())) {
            Ok(()) => sent += 1,
            Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
        }
    }
    let stats = victim.stats();
    wait_until("the attacker's ban", Duration::from_secs(20), || {
        stats.banned.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert!(stats.rate_limited.load(Ordering::Relaxed) > 0);
    // The victim validated at most a burst plus what the rate refilled meanwhile.
    let budget = burst as f64 + rate as f64 * started.elapsed().as_secs_f64() + 5.0;
    let done = validated.load(Ordering::Relaxed);
    assert!(
        (done as f64) <= budget,
        "validated {done} txs, budget {budget:.0}"
    );
    assert!(done < 600 / 2, "validated {done} of 600 junk txs");
    assert_eq!(stats.accepted.load(Ordering::Relaxed), 0);

    // The honest peer is unaffected.
    honest.publish_tx(Bytes::from_static(b"honest-tx")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, inbox.recv()).await {
            Ok(Some((_, tx))) if tx.as_ref() == b"honest-tx" => break,
            Ok(Some(_)) => continue,
            other => panic!("honest tx never arrived: {other:?}"),
        }
    }
    assert_eq!(victim.topic_peer_count(), 1, "only the honest peer is left");

    for n in [victim, attacker, honest] {
        n.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_peers_cannot_take_the_bootnode_slot() {
    let accept_all: TxValidator = Arc::new(|_: &[u8]| Verdict::Accept);
    // The bootnode is not running yet: its identity and port are fixed up front.
    let boot_key = Keypair::generate_ed25519();
    let boot_port = free_port();
    let boot_addr = format!(
        "/ip4/127.0.0.1/tcp/{boot_port}/p2p/{}",
        boot_key.public().to_peer_id()
    );
    // 4 slots, 1 bootnode: at most 3 inbound connections.
    let victim = start(
        NetworkConfig {
            max_peers: 4,
            ..local(vec![boot_addr])
        },
        CHAIN_ID,
        Keypair::generate_ed25519(),
        accept_all.clone(),
    )
    .await
    .unwrap();
    let mut attackers = Vec::new();
    for _ in 0..5 {
        attackers.push(
            start(
                local(vec![addr_of(&victim)]),
                CHAIN_ID,
                Keypair::generate_ed25519(),
                accept_all.clone(),
            )
            .await
            .unwrap(),
        );
    }
    wait_until("inbound peers", Duration::from_secs(20), || {
        victim.peer_count() == 3
    })
    .await;
    // Long enough for the victim's first dial of the (absent) bootnode to fail.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(victim.peer_count(), 3, "inbound limit");

    // Now the bootnode comes up; the victim's periodic redial must reach it.
    let boot = start(
        NetworkConfig {
            listen: format!("/ip4/127.0.0.1/tcp/{boot_port}"),
            ..NetworkConfig::default()
        },
        CHAIN_ID,
        boot_key,
        accept_all,
    )
    .await
    .unwrap();
    wait_until("the bootnode dial", Duration::from_secs(20), || {
        boot.peer_count() >= 1
    })
    .await;
    assert!(victim.peer_count() <= 4);

    victim.shutdown().await;
    boot.shutdown().await;
    for a in attackers {
        a.shutdown().await;
    }
}
