//! Phase 3 exit criteria for the staking-driven validator set (plan §27 Phase 3,
//! §11.4–11.6), on in-process [`LocalNet`]s whose genesis holds the real Staking
//! predeploy and a short epoch (20 blocks of 400 ms):
//!
//! * (a) a new validator registers through `Staking.register()` and joins consensus at
//!   the next epoch boundary on every node (and signs blocks);
//! * (b) a validator that is offline for a whole liveness window (= one epoch) is jailed
//!   by `recordParticipation` and leaves the set at the boundary; the chain continues;
//! * (c) a validator that double-signs (two conflicting precommits signed with its real
//!   consensus key) is slashed 5% and tombstoned through evidence included in a block,
//!   and leaves the set at the next boundary.
//!
//! Every test ends with the no-fork check: same blocks and state roots on every node,
//! every commit certificate verifies against the validator set of its height.
//! The nets of this binary run one at a time (`LocalNet` holds a process-wide lock).
//! Run with `RUST_LOG=xdevs=info` to see the node logs.

use serde_json::{json, Value};
use std::time::{Duration, Instant};
use xdevs_consensus::{vote_sign_bytes, ConflictingVotes, SignedVote, ValidatorKey, VoteType};
use xdevs_execution::staking::ValidatorStatus;
use xdevs_primitives::{hex, B256, U256};
use xdevs_testutils::{LocalNet, LocalNetOptions, StakingNet};

const BLOCK_MS: u64 = 400;
const EPOCH: u64 = 20;

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ether(n: u64) -> U256 {
    U256::from(n) * U256::from(10u64).pow(U256::from(18u64))
}

fn opts(validators: usize, staking: StakingNet) -> LocalNetOptions {
    LocalNetOptions {
        encrypted_mempool: None,
        validators,
        full_nodes: 0,
        block_time_ms: BLOCK_MS,
        staking: Some(staking),
        parallel_validators: 0,
        local_fee_markets_block: None,
    }
}

fn quantity(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().expect("quantity").trim_start_matches("0x"), 16)
        .expect("hex quantity")
}

/// Poll `status(operator of node v)` on node `i` until `want`.
async fn wait_status(
    net: &LocalNet,
    i: usize,
    v: usize,
    want: ValidatorStatus,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        net.check_engines()?;
        let info = net.staking_validator(i, net.operator(v)).await?;
        if info.status == want {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "validator {v} is {:?}, expected {want:?} within {timeout:?}",
            info.status
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// (a) A 5th validator joins via `register()` and is active from the next epoch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registered_validator_joins_at_the_next_epoch() -> anyhow::Result<()> {
    let net = LocalNet::start(opts(
        4,
        StakingNet {
            activation: 5,
            epoch_length: EPOCH,
            candidates: 1,
            ..StakingNet::default()
        },
    ))
    .await?;
    let joiner = 4;
    let address = net.node(joiner).validator_address().expect("validator");
    net.wait_for_height(6, secs(60)).await?;
    assert!(net.validator_set_at(0, 6).await?.get(&address).is_none());

    let stake = ether(1_000);
    let tx = net.register_validator(0, joiner, stake).await?;
    let receipt = net.wait_receipt(0, tx, secs(30)).await?;
    assert_eq!(receipt["status"], json!("0x1"), "register: {receipt}");
    let registered = quantity(&receipt["blockNumber"]);
    let info = net.staking_validator(0, net.operator(joiner)).await?;
    assert_eq!(info.status, ValidatorStatus::Candidate);
    assert_eq!(info.total_stake, stake);

    // The set changes at the first boundary after the registration block (a boundary
    // block runs onEpoch before its transactions).
    let boundary = net
        .chain_config()
        .next_epoch_boundary(registered + 1)
        .expect("staking scheduled");
    eprintln!("(a) registered at block {registered}, joins after epoch block {boundary}");
    net.wait_for_height(boundary + 12, secs(120)).await?;
    for i in net.running() {
        let before = net.validator_set_at(i, boundary).await?;
        let after = net.validator_set_at(i, boundary + 1).await?;
        assert_eq!(before.len(), 4, "node {i}");
        assert!(before.get(&address).is_none(), "node {i}");
        assert_eq!(after.len(), 5, "node {i}");
        assert_eq!(after.get(&address).map(|v| v.voting_power), Some(1_000));
    }
    assert_eq!(
        net.staking_validator(0, net.operator(joiner)).await?.status,
        ValidatorStatus::Active
    );
    // It signs from its first height on (never before: it was not a member).
    let signed = net.signed_heights(0, joiner, 1, boundary + 12).await?;
    assert!(
        !signed.is_empty() && signed.iter().all(|h| *h > boundary),
        "joiner signed {signed:?}"
    );
    net.assert_no_fork().await?;
    net.shutdown().await;
    Ok(())
}

/// (b) A validator offline for a whole liveness window is jailed and removed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b_an_offline_validator_is_jailed_and_removed() -> anyhow::Result<()> {
    let mut net = LocalNet::start(opts(
        4,
        StakingNet {
            activation: 1,
            epoch_length: EPOCH,
            downtime_window: EPOCH,
            ..StakingNet::default()
        },
    ))
    .await?;
    let offline = 3;
    let address = net.node(offline).validator_address().expect("validator");
    net.stop(offline).await?;

    // Window 1 = blocks 1..=20 closes in block 20, the first epoch boundary.
    net.wait_for_height(EPOCH + 2, secs(180)).await?;
    wait_status(&net, 0, offline, ValidatorStatus::Jailed, secs(10)).await?;
    let info = net.staking_validator(0, net.operator(offline)).await?;
    assert_eq!(
        info.total_stake,
        ether(1_000) - ether(1_000) / U256::from(10_000u64),
        "0.01% downtime slash"
    );
    assert!(info.jailed_until > 0);
    for v in 0..3 {
        assert_eq!(
            net.staking_validator(0, net.operator(v)).await?.status,
            ValidatorStatus::Active,
            "signer {v}"
        );
    }
    let set = net.validator_set_at(0, EPOCH + 1).await?;
    assert_eq!(set.len(), 3);
    assert!(set.get(&address).is_none());
    assert!(net
        .validator_set_at(0, EPOCH)
        .await?
        .get(&address)
        .is_some());

    // The three remaining validators keep the chain going on their own.
    let h = net.max_head().await;
    net.wait_for_height(h + 10, secs(60)).await?;
    net.assert_no_fork().await?;
    net.shutdown().await;
    Ok(())
}

/// Two conflicting precommits of validator `seed` at `(height, 0)`: a double sign.
fn double_sign(chain_id: u64, seed: [u8; 32], height: u64, hash: B256) -> ConflictingVotes {
    let sk = ed25519_consensus::SigningKey::from(seed);
    let validator = ValidatorKey(sk.verification_key().to_bytes()).address();
    let vote = |block_hash: Option<B256>| {
        let msg = vote_sign_bytes(chain_id, VoteType::Precommit, height, 0, block_hash);
        SignedVote {
            vote_type: VoteType::Precommit,
            height,
            round: 0,
            block_hash,
            validator,
            signature: sk.sign(&msg).to_bytes(),
        }
    };
    ConflictingVotes {
        first: vote(Some(hash)),
        second: vote(Some(B256::repeat_byte(0xbb))),
    }
}

/// (c) A double-signing validator is slashed, tombstoned and removed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c_a_double_signing_validator_is_slashed() -> anyhow::Result<()> {
    let net = LocalNet::start(opts(
        4,
        StakingNet {
            activation: 1,
            epoch_length: EPOCH,
            ..StakingNet::default()
        },
    ))
    .await?;
    let byzantine = 3;
    let address = net.node(byzantine).validator_address().expect("validator");
    let seed = net.node(byzantine).consensus_seed().expect("validator");
    net.wait_for_height(5, secs(60)).await?;

    // The validator signs a second, conflicting precommit for a decided height.
    let height = net.head(0).await?;
    let block = net.block(0, height).await?;
    let hash: B256 = block["hash"].as_str().expect("hash").parse()?;
    let evidence = double_sign(net.chain_id(), seed, height, hash);
    let raw = format!("0x{}", hex::encode(evidence.to_bytes()));
    // A forged second vote is refused.
    let mut forged = evidence;
    forged.second.signature[0] ^= 1;
    let refused = net
        .call(
            0,
            "xdevs_submitEvidence",
            vec![json!(format!("0x{}", hex::encode(forged.to_bytes())))],
        )
        .await;
    assert!(refused.is_err(), "forged evidence accepted: {refused:?}");
    for i in net.running() {
        let h = net
            .call(i, "xdevs_submitEvidence", vec![json!(raw)])
            .await?;
        assert_eq!(h, json!(evidence.canonical().hash()));
    }

    wait_status(&net, 0, byzantine, ValidatorStatus::Tombstoned, secs(60)).await?;
    let info = net.staking_validator(0, net.operator(byzantine)).await?;
    assert_eq!(info.total_stake, ether(950), "5% double-sign slash");
    assert_eq!(info.jailed_until, u64::MAX);
    let slashed_at = net.head(0).await?;
    let boundary = net
        .chain_config()
        .next_epoch_boundary(slashed_at)
        .expect("staking scheduled");
    eprintln!("(c) tombstoned by block {slashed_at}, removed after epoch block {boundary}");

    net.wait_for_height(boundary + 3, secs(120)).await?;
    for i in net.running() {
        let set = net.validator_set_at(i, boundary + 1).await?;
        assert_eq!(set.len(), 3, "node {i}");
        assert!(set.get(&address).is_none(), "node {i}");
    }
    // Same evidence again: accepted by the RPC (valid) but slashes nothing more.
    net.call(0, "xdevs_submitEvidence", vec![json!(raw)])
        .await?;
    net.wait_for_height(boundary + 8, secs(60)).await?;
    assert_eq!(
        net.staking_validator(0, net.operator(byzantine))
            .await?
            .total_stake,
        ether(950)
    );
    // The evidence itself is logged as equivocation: check consistency only.
    net.assert_consistent().await?;
    net.shutdown().await;
    Ok(())
}
