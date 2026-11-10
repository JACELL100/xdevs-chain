//! Phase 2 exit criteria (plan §27 Phase 2; research doc §6.8, §6.12), one test per
//! criterion, on in-process [`LocalNet`]s with 400 ms blocks:
//!
//! * (a) 4 validators produce blocks at about the target block time;
//! * (b) with 1 of 4 validators down the chain keeps producing blocks;
//! * (c) with 2 of 4 down no block is produced (safe halt, no fork); restarting them
//!   resumes the chain;
//! * (d) never a fork: identical block hash and state root at every height on every
//!   node, every stored commit certificate verifies (checked at the end of every test,
//!   and under tx load with a validator restart and a full node in (d) itself);
//! * (e) a full node started late (chain at height ≥ 20) syncs from genesis through
//!   ValueSync and follows the tip;
//! * (f) a restarted validator rejoins without double-signing (no SignGuard refusal, no
//!   equivocation evidence) and signs again; the chain continues;
//! * (g) transactions sent to any node's RPC (validators and a full node) are included.
//!
//! The nets of this binary run one at a time (`LocalNet` holds a process-wide lock).
//! Run with `RUST_LOG=xdevs=info` to see the node logs.

use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use xdevs_primitives::{Address, U256};
use xdevs_testutils::{LocalNet, LocalNetOptions};

const BLOCK_MS: u64 = 400;

fn opts(validators: usize, full_nodes: usize) -> LocalNetOptions {
    LocalNetOptions {
        validators,
        full_nodes,
        block_time_ms: BLOCK_MS,
    }
}

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ether(n: u64) -> U256 {
    U256::from(n) * U256::from(10u64).pow(U256::from(18u64))
}

/// (a) 4 validators produce blocks, at about the target block time, with rotating
/// proposers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_four_validators_produce_blocks_at_target_time() -> anyhow::Result<()> {
    let net = LocalNet::start(opts(4, 0)).await?;
    net.wait_for_height(3, secs(60)).await?;

    const BLOCKS: u64 = 15;
    let h0 = net.head(0).await?;
    let t0 = Instant::now();
    net.wait_node_height(0, h0 + BLOCKS, secs(60)).await?;
    let per_block = t0.elapsed() / BLOCKS as u32;
    eprintln!("(a) {BLOCKS} blocks, {per_block:?} per block (target {BLOCK_MS} ms)");
    let target = net.block_time();
    assert!(
        per_block >= target.mul_f64(0.8) && per_block <= target * 3,
        "block time {per_block:?}, target {target:?}"
    );

    let mut proposers = BTreeSet::new();
    for h in h0 + 1..=h0 + BLOCKS {
        let b = net.block(0, h).await?;
        proposers.insert(b["miner"].as_str().unwrap_or_default().to_string());
    }
    assert!(proposers.len() >= 3, "proposers rotate: {proposers:?}");

    net.wait_for_height(h0 + BLOCKS, secs(30)).await?;
    let checked = net.assert_no_fork().await?;
    eprintln!("(a) no fork over {checked} heights on 4 nodes");
    net.shutdown().await;
    Ok(())
}

/// (b) Kill 1 of 4 validators: the remaining 3 (> 2/3 of the power) keep deciding.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b_one_validator_down_chain_continues() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(4, 0)).await?;
    net.wait_for_height(3, secs(60)).await?;

    net.stop(3).await?;
    let h = net.max_head().await;
    net.wait_for_height(h + 10, secs(60)).await?;
    // The stopped validator signs none of the new blocks: 3 of 4 decide alone.
    let head = net.head(0).await?;
    let signed = net.signed_heights(0, 3, h + 2, head).await?;
    assert!(signed.is_empty(), "val4 is down but signed {signed:?}");
    for height in h + 2..=head {
        let cert = net.certificate(0, height).await?.expect("certificate");
        assert_eq!(cert.signatures.len(), 3, "height {height}");
    }
    let checked = net.assert_no_fork().await?;
    eprintln!("(b) 1 of 4 down: {h} -> {head}; no fork over {checked} heights");
    net.shutdown().await;
    Ok(())
}

/// (c) Kill 2 of 4: no quorum, so no new block (and no fork, engines keep running);
/// restart them: the chain resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c_two_validators_down_halts_then_resumes() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(4, 0)).await?;
    net.wait_for_height(3, secs(60)).await?;

    net.stop(2).await?;
    net.stop(3).await?;
    // A height already precommitted by a quorum may still finish; then nothing.
    tokio::time::sleep(secs(2)).await;
    let halted_at = net.max_head().await;
    tokio::time::sleep(secs(6)).await;
    assert_eq!(
        net.max_head().await,
        halted_at,
        "2 of 4 validators produced a block without a quorum"
    );
    net.check_engines()?; // halted, not crashed
    net.assert_no_fork().await?;

    net.restart(2).await?;
    net.restart(3).await?;
    let t = Instant::now();
    net.wait_for_height(halted_at + 5, secs(90)).await?;
    let checked = net.assert_no_fork().await?;
    eprintln!(
        "(c) halted at {halted_at}; resumed {:.1} s after the restart; no fork over \
         {checked} heights",
        t.elapsed().as_secs_f64()
    );
    net.shutdown().await;
    Ok(())
}

/// (d) Never a fork, under load: 4 validators + 1 full node, transfers sent to every
/// node while one validator is restarted; then every node holds the same hash and state
/// root at every height and every certificate verifies.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d_no_fork_under_load_and_restart() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(4, 1)).await?;
    net.wait_for_height(2, secs(60)).await?;
    net.wait_gossip_mesh(2, secs(30)).await?;

    let mut hashes: Vec<(usize, _)> = Vec::new();
    for round in 0..6u64 {
        if round == 2 {
            net.stop(1).await?;
        }
        if round == 4 {
            net.restart(1).await?;
        }
        let mut batch = Vec::new();
        for i in net.running() {
            // Account `i` only ever sends through node `i`: nonces are sequential.
            let to = Address::repeat_byte(0x50 + i as u8);
            let nonce = hashes.iter().filter(|(n, _)| *n == i).count() as u64;
            let hash = net.send_transfer(i, i, nonce, to, ether(1)).await?;
            hashes.push((i, hash));
            batch.push(hash);
        }
        // Included (seen by the full node) before the next round's stop / restart.
        for hash in batch {
            let r = net.wait_receipt(4, hash, secs(30)).await?;
            assert_eq!(r["status"], "0x1", "{r}");
        }
    }
    let tip = net.max_head().await.max(25);
    net.wait_for_height(tip, secs(60)).await?;
    let checked = net.assert_no_fork().await?;
    eprintln!(
        "(d) {} txs, no fork over {checked} heights on 5 nodes (hash, state root, \
         certificate)",
        hashes.len()
    );
    net.shutdown().await;
    Ok(())
}

/// (e) A full node started after height 20 syncs from genesis (ValueSync) and keeps up
/// with the tip.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e_late_full_node_syncs_from_genesis() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(4, 0)).await?;
    net.wait_for_height(20, secs(90)).await?;

    let full = net.add_full_node().await?;
    let t = Instant::now();
    let tip = net.max_head().await;
    net.wait_node_height(full, tip, secs(60)).await?;
    let synced = t.elapsed();
    // Then it keeps up with the live tip instead of trailing it by a constant lag
    // (regression: a catching-up node used to wait the block time at every height).
    let deadline = Instant::now() + secs(30);
    let mut lags = Vec::new();
    loop {
        let validators = net.head(0).await?;
        let lag = validators.saturating_sub(net.head(full).await?);
        lags.push(lag);
        let recent = &lags[lags.len().saturating_sub(8)..];
        if recent.len() == 8 && recent.iter().all(|l| *l <= 1) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "full node lags the tip: {lags:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let tip = net.max_head().await;
    net.wait_for_height(tip, secs(30)).await?;
    let syncing = net.call(full, "eth_syncing", vec![]).await?;
    assert_eq!(syncing, serde_json::Value::Bool(false), "{syncing}");
    let checked = net.assert_no_fork().await?;
    eprintln!(
        "(e) late full node: at the tip {:.1} s after its start, then lags {lags:?}; no fork \
         over {checked} heights on 5 nodes",
        synced.as_secs_f64()
    );
    net.shutdown().await;
    Ok(())
}

/// (f) A validator restarted mid-run (immediately, then another after missing blocks)
/// rejoins without a SignGuard refusal or equivocation and signs blocks again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_restarted_validator_rejoins_without_double_signing() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(4, 0)).await?;
    net.wait_for_height(5, secs(60)).await?;

    // Immediate restart: same height, WAL replay, same votes.
    net.stop(1).await?;
    net.restart(1).await?;
    let h = net.max_head().await;
    net.wait_for_height(h + 6, secs(60)).await?;
    let head = net.head(0).await?;
    let signed = net.signed_heights(0, 1, h + 2, head).await?;
    assert!(!signed.is_empty(), "val2 signs nothing after its restart");

    // Delayed restart: misses blocks, catches up, signs again.
    net.stop(2).await?;
    let h = net.max_head().await;
    net.wait_for_height(h + 5, secs(60)).await?;
    net.restart(2).await?;
    let h = net.max_head().await;
    net.wait_for_height(h + 6, secs(60)).await?;
    let head = net.head(0).await?;
    let signed = net.signed_heights(0, 2, h + 2, head).await?;
    assert!(!signed.is_empty(), "val3 signs nothing after its restart");

    assert_eq!(net.node(1).starts(), 2);
    assert_eq!(net.node(2).starts(), 2);
    let checked = net.assert_no_fork().await?; // includes: no SignGuard refusal
    eprintln!(
        "(f) 2 restarts; alerts {:?}; no fork over {checked} heights",
        net.safety_alerts()
    );
    net.shutdown().await;
    Ok(())
}

/// (g) A transaction sent to any node's RPC (each validator and a full node) is
/// gossiped, included and visible everywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g_txs_sent_to_any_node_are_included() -> anyhow::Result<()> {
    let net = LocalNet::start(opts(4, 1)).await?;
    net.wait_for_height(2, secs(60)).await?;
    net.wait_gossip_mesh(2, secs(30)).await?;

    let n = net.nodes().len();
    let mut sent = Vec::new();
    for i in 0..n {
        let to = Address::repeat_byte(0x70 + i as u8);
        sent.push((i, to, net.send_transfer(i, i, 0, to, ether(1)).await?));
    }
    let mut last = 0;
    for (i, _, hash) in &sent {
        // Read the receipt from another node than the one the tx was sent to.
        let r = net.wait_receipt((i + 1) % n, *hash, secs(30)).await?;
        assert_eq!(r["status"], "0x1", "tx via {}: {r}", net.node(*i).name());
        let included = u64::from_str_radix(
            r["blockNumber"].as_str().unwrap().trim_start_matches("0x"),
            16,
        )?;
        last = last.max(included);
    }
    net.wait_for_height(last, secs(30)).await?;
    for (_, to, _) in &sent {
        for j in 0..n {
            assert_eq!(net.balance(j, *to).await?, ether(1), "{to} on node {j}");
        }
    }
    let checked = net.assert_no_fork().await?;
    eprintln!("(g) {n} txs via {n} nodes included by block {last}; no fork over {checked} heights");
    net.shutdown().await;
    Ok(())
}
