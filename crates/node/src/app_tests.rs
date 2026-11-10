//! `ChainApp` unit tests: propose / validate / commit across two app instances, every
//! validation rule, idempotent commits with certificate upgrades, restart.

use super::*;
use crate::consensus::genesis_validator_set;
use xdevs_consensus::{vote_sign_bytes, CommitSig, ValidatorKey, VoteType};
use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
use xdevs_genesis::{Genesis, GenesisValidator, DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
use xdevs_mempool::{FromState, PoolConfig, PoolParams, TxOrigin, TxPool};
use xdevs_primitives::{
    constants::{DEV_CHAIN_ID, GWEI},
    xip0001, Bloom, RecoveredTx, TxEip1559, TxEnvelope, TxKind, U256,
};
use xdevs_storage::{MemDb, RedbDatabase};

const N: usize = 4;

fn seeds() -> Vec<[u8; 32]> {
    (0..N).map(|i| [i as u8 + 1; 32]).collect()
}

/// Dev genesis plus 4 equal-power validators; dev account 1 is XIP-0001 LOCKED.
fn genesis() -> Genesis {
    let mut g = Genesis::dev(DEV_CHAIN_ID);
    g.validators = seeds()
        .iter()
        .enumerate()
        .map(|(i, s)| GenesisValidator {
            name: Some(format!("val{i}")),
            consensus_key: ed25519_consensus::SigningKey::from(*s)
                .verification_key()
                .to_bytes(),
            power: 1,
            p2p: None,
            extra: Default::default(),
        })
        .collect();
    let locked = g.alloc.entry(DEV_ACCOUNTS[1]).or_default();
    locked.code = Some(Bytes::from(xip0001::DESIGNATOR.to_vec()));
    locked
        .storage
        .insert(xip0001::LOCK_SLOT, U256::from(xip0001::LOCKED));
    g
}

struct TestNode {
    db: Db,
    pool: Arc<Pool>,
    app: ChainApp,
}

fn node_on(db: Db, g: &Genesis) -> TestNode {
    chain::init_genesis(&*db, g).unwrap();
    chain::ensure_validator_set(&*db, &g.validators).unwrap();
    let cfg = g.chain_config();
    let (_, head) = chain::latest_header(&*db).unwrap();
    let pool = Arc::new(Pool::new(
        PoolParams::from_chain_config(&cfg),
        PoolConfig::default(),
        Arc::new(FromState(DbStateProvider::new(db.clone()))),
        next_base_fee(&head, &cfg),
    ));
    let app = ChainApp::new(
        db.clone(),
        pool.clone(),
        Arc::new(EvmExecutor::new(cfg.clone())),
        cfg,
        genesis_validator_set(g).unwrap().unwrap(),
        ChainAppConfig {
            fee_recipient: Address::repeat_byte(0xfe),
            ..Default::default()
        },
    );
    TestNode { db, pool, app }
}

fn node(g: &Genesis) -> TestNode {
    node_on(Arc::new(MemDb::new()), g)
}

/// A certificate for `block` signed by the first `signers` validators.
fn cert(block: &Block, signers: usize) -> CommitCertificate {
    let msg = vote_sign_bytes(
        DEV_CHAIN_ID,
        VoteType::Precommit,
        block.number(),
        0,
        Some(block.hash()),
    );
    let sigs = seeds()
        .iter()
        .take(signers)
        .map(|s| {
            let sk = ed25519_consensus::SigningKey::from(*s);
            CommitSig {
                validator: ValidatorKey(sk.verification_key().to_bytes()).address(),
                signature: sk.sign(&msg).to_bytes(),
            }
        })
        .collect();
    CommitCertificate::new(block.number(), 0, block.hash(), sigs)
}

fn transfer(key: usize, nonce: u64, value: u64) -> TxEnvelope {
    sign_tx_envelope(
        &signer_from_hex(DEV_PRIVATE_KEYS[key]).unwrap(),
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to: TxKind::Call(Address::repeat_byte(0x77)),
            value: U256::from(value),
            input: Bytes::new(),
            access_list: Default::default(),
        },
    )
    .unwrap()
}

fn far() -> Instant {
    Instant::now() + std::time::Duration::from_secs(30)
}

#[tokio::test]
async fn a_proposes_b_validates_and_both_commit_the_same_block() {
    let g = genesis();
    let (a, b) = (node(&g), node(&g));
    let tx = transfer(0, 0, 5);
    for n in [&a, &b] {
        n.pool
            .add(RecoveredTx::recover(tx.clone()).unwrap(), TxOrigin::Local)
            .unwrap();
    }
    assert!(
        a.app.propose(2, 0, far()).await.is_err(),
        "height must be head + 1"
    );
    let block = a.app.propose(1, 0, far()).await.unwrap();
    assert_eq!(block.number(), 1);
    assert_eq!(block.body, vec![tx]);
    assert_eq!(block.header.beneficiary, Address::repeat_byte(0xfe));
    // The proposer's verdict is cached; the other node re-executes.
    for n in [&a, &b] {
        assert_eq!(
            n.app
                .validate(1, &block, BlockOrigin::Consensus)
                .await
                .unwrap(),
            Validity::Valid
        );
    }
    assert!(
        b.app
            .validate(2, &block, BlockOrigin::Consensus)
            .await
            .is_err(),
        "a height that is not head + 1 is a local error, not a verdict"
    );

    // Not enough signatures: refused.
    assert!(b.app.commit(block.clone(), cert(&block, 2)).await.is_err());
    for n in [&a, &b] {
        n.app.commit(block.clone(), cert(&block, 3)).await.unwrap();
        assert_eq!(n.app.head().await.unwrap(), (1, block.hash()));
        assert!(n.pool.is_empty(), "included tx left the pool");
        chain::verify_head(&*n.db).unwrap();
    }
    let root = |n: &TestNode| chain::header(&*n.db, 1).unwrap().unwrap().state_root;
    assert_eq!(root(&a), root(&b));

    // Idempotent re-commit: a larger certificate replaces the stored one ...
    b.app.commit(block.clone(), cert(&block, 4)).await.unwrap();
    let (_, stored) = b.app.decided(1).await.unwrap().unwrap();
    assert_eq!(stored.signatures.len(), 4);
    // ... a smaller one does not.
    b.app.commit(block.clone(), cert(&block, 3)).await.unwrap();
    let (decided, stored) = b.app.decided(1).await.unwrap().unwrap();
    assert_eq!(stored.signatures.len(), 4);
    assert_eq!(decided, block);
    assert_eq!(b.app.head().await.unwrap(), (1, block.hash()));
    assert!(b.app.decided(2).await.unwrap().is_none());
    assert_eq!(b.app.earliest_height().await.unwrap(), 1);
    assert_eq!(b.app.validator_set(7).await.unwrap().len(), N);

    // A different block at a committed height is a fork: refused.
    let mut forked = b.app.propose(2, 0, far()).await.unwrap();
    forked.header.number = 1;
    forked.header.parent_hash = chain::header(&*b.db, 0).unwrap().unwrap().hash_slow();
    let err = b
        .app
        .commit(forked.clone(), cert(&forked, 3))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("FORK"), "{err:#}");
}

/// The invalid-block reason the validator finds (`None` = valid).
fn reason(n: &TestNode, block: &Block, origin: BlockOrigin) -> Option<String> {
    match n.app.inner.verdict(1, block, origin, unix_now()).unwrap() {
        Verdict::Valid(_) => None,
        Verdict::Invalid(r) => Some(r.to_string()),
    }
}

#[tokio::test]
async fn invalid_blocks_are_rejected_for_each_rule() {
    let g = genesis();
    let (a, b) = (node(&g), node(&g));
    a.pool
        .add(
            RecoveredTx::recover(transfer(0, 0, 5)).unwrap(),
            TxOrigin::Local,
        )
        .unwrap();
    let good = a.app.propose(1, 0, far()).await.unwrap();
    assert_eq!(reason(&b, &good, BlockOrigin::Consensus), None);
    let genesis_ts = chain::header(&*b.db, 0).unwrap().unwrap().timestamp;

    let mutate = |f: &dyn Fn(&mut Block)| {
        let mut blk = good.clone();
        f(&mut blk);
        blk
    };
    let cases: Vec<(&str, Block)> = vec![
        ("number", mutate(&|b| b.header.number = 2)),
        (
            "parent hash",
            mutate(&|b| b.header.parent_hash = B256::repeat_byte(1)),
        ),
        ("gas limit", mutate(&|b| b.header.gas_limit -= 1)),
        ("base fee", mutate(&|b| b.header.base_fee_per_gas = Some(7))),
        ("timestamp", mutate(&|b| b.header.timestamp = genesis_ts)),
        (
            "extraData",
            mutate(&|b| b.header.extra_data = Bytes::from(vec![0u8; 33])),
        ),
        ("compat", mutate(&|b| b.header.difficulty = U256::from(1))),
        ("prevRandao", mutate(&|b| b.header.mix_hash = B256::ZERO)),
        ("transactions root", mutate(&|b| b.body.clear())),
        ("state root", mutate(&|b| b.header.state_root = B256::ZERO)),
        (
            "receipts root",
            mutate(&|b| b.header.receipts_root = B256::ZERO),
        ),
        (
            "bloom",
            mutate(&|b| b.header.logs_bloom = Bloom::repeat_byte(1)),
        ),
        ("gas used", mutate(&|b| b.header.gas_used += 1)),
    ];
    for (expected, block) in cases {
        let r =
            reason(&b, &block, BlockOrigin::Sync).unwrap_or_else(|| panic!("{expected}: accepted"));
        assert!(r.contains(expected), "{expected}: {r}");
        assert_eq!(
            b.app.validate(1, &block, BlockOrigin::Sync).await.unwrap(),
            Validity::Invalid,
            "{expected}"
        );
    }

    // Body rules: a tx for another chain, a bad nonce, an XIP-0001 violation (the
    // LOCKED account sends value to someone else). The tx root is fixed up so only the
    // tx rule can fail.
    let with_body = |txs: Vec<TxEnvelope>| {
        let mut blk = good.clone();
        blk.body = txs;
        blk.header.transactions_root = blk.compute_transactions_root();
        blk
    };
    let wrong_chain = sign_tx_envelope(
        &signer_from_hex(DEV_PRIVATE_KEYS[2]).unwrap(),
        TxEip1559 {
            chain_id: DEV_CHAIN_ID + 1,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::repeat_byte(0x77)),
            value: U256::from(1),
            input: Bytes::new(),
            access_list: Default::default(),
        },
    )
    .unwrap();
    let r = reason(&b, &with_body(vec![wrong_chain]), BlockOrigin::Sync).unwrap();
    assert!(r.contains("chain id"), "{r}");
    let r = reason(&b, &with_body(vec![transfer(2, 9, 1)]), BlockOrigin::Sync).unwrap();
    assert!(r.to_lowercase().contains("nonce"), "{r}");
    let r = reason(&b, &with_body(vec![transfer(1, 0, 1)]), BlockOrigin::Sync).unwrap();
    assert!(r.contains("xip-0001"), "{r}");

    // Rule 3: a live proposal far in the future is untimely; the same block from sync
    // (already decided) only has to execute.
    let mut future = good.clone();
    future.header.timestamp = unix_now() + 3600;
    assert!(reason(&b, &future, BlockOrigin::Consensus)
        .unwrap()
        .contains("ahead of the local clock"));
    assert_eq!(reason(&b, &future, BlockOrigin::Sync), None);

    // The good block is still valid (verdicts are per block hash).
    assert_eq!(
        b.app
            .validate(1, &good, BlockOrigin::Consensus)
            .await
            .unwrap(),
        Validity::Valid
    );
}

#[tokio::test]
async fn restart_resumes_at_the_committed_head() {
    let g = genesis();
    let dir = tempfile::tempdir().unwrap();
    let open = || -> Db { Arc::new(RedbDatabase::open_dir(dir.path()).unwrap()) };
    let proposer = node(&g);
    let mut blocks = Vec::new();
    {
        let n = node_on(open(), &g);
        for h in 1..=2 {
            let block = proposer.app.propose(h, 0, far()).await.unwrap();
            proposer
                .app
                .commit(block.clone(), cert(&block, 3))
                .await
                .unwrap();
            // Cache miss on `n`: commit re-executes once.
            n.app.commit(block.clone(), cert(&block, 3)).await.unwrap();
            blocks.push(block);
        }
    }
    let n = node_on(open(), &g);
    assert_eq!(n.app.head().await.unwrap(), (2, blocks[1].hash()));
    let (b2, c2) = n.app.decided(2).await.unwrap().unwrap();
    assert_eq!((b2.hash(), c2.signatures.len()), (blocks[1].hash(), 3));
    chain::verify_head(&*n.db).unwrap();
    // Re-commit after the restart is still idempotent.
    n.app
        .commit(blocks[1].clone(), cert(&blocks[1], 3))
        .await
        .unwrap();
    // It continues at height 3.
    let b3 = n.app.propose(3, 0, far()).await.unwrap();
    assert_eq!(b3.header.parent_hash, blocks[1].hash());
    n.app.commit(b3.clone(), cert(&b3, 4)).await.unwrap();
    assert_eq!(n.app.head().await.unwrap(), (3, b3.hash()));

    // A different validator set on the same data dir is refused.
    drop(n);
    let mut other = g.clone();
    other.validators[0].power = 2;
    let db = open();
    let err = chain::ensure_validator_set(&*db, &other.validators).unwrap_err();
    assert!(
        err.to_string().contains("different genesis validator set"),
        "{err}"
    );
    assert!(chain::ensure_validator_set(&*db, &[]).is_err());
}

/// A consensus commit publishes the block to `eth_subscribe` once; an idempotent
/// re-commit (certificate upgrade) does not publish it again.
#[tokio::test]
async fn commit_publishes_chain_events_once() {
    let g = genesis();
    let n = node(&g);
    let events = Arc::new(ChainEvents::default());
    let app = n.app.clone().with_events(events.clone());
    let mut rx = events.subscribe_blocks();
    n.pool
        .add(
            RecoveredTx::recover(transfer(0, 0, 5)).unwrap(),
            TxOrigin::Local,
        )
        .unwrap();
    let block = app.propose(1, 0, far()).await.unwrap();
    app.commit(block.clone(), cert(&block, 3)).await.unwrap();
    app.commit(block.clone(), cert(&block, 4)).await.unwrap();
    let ev = rx.try_recv().unwrap();
    assert_eq!(ev.header.hash, block.hash());
    assert_eq!(ev.header.inner.number, 1);
    assert!(ev.logs.is_empty());
    assert!(rx.try_recv().is_err(), "re-commit must not publish again");
}
