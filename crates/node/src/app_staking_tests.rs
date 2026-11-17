//! `ChainApp` with the staking-driven validator set (Phase 3): consensus data in
//! proposals, its validation rules, epoch-boundary sets and double-sign evidence, on two
//! app instances over a genesis with the real Staking predeploy.

use super::*;
use xdevs_consensus::{vote_sign_bytes, CommitSig, SignedVote, ValidatorKey, VoteType};
use xdevs_execution::staking::{
    calls, decode_validator_info, genesis_storage, GenesisStake, StakingParams, ValidatorStatus,
};
use xdevs_genesis::{Genesis, GenesisAccount, GenesisValidator, DEV_ACCOUNTS};
use xdevs_mempool::{FromState, PoolConfig, PoolParams};
use xdevs_primitives::{
    constants::{DEV_CHAIN_ID, GOVERNANCE_ADDRESS, STAKING_ADDRESS},
    U256,
};
use xdevs_storage::{DbReadExt, MemDb};

const N: usize = 4;
const EPOCH: u64 = 3;

fn ether(n: u64) -> U256 {
    U256::from(n) * U256::from(10u64).pow(U256::from(18u64))
}

fn sk(i: usize) -> ed25519_consensus::SigningKey {
    ed25519_consensus::SigningKey::from([i as u8 + 1; 32])
}

fn address(i: usize) -> ValidatorAddress {
    ValidatorKey(sk(i).verification_key().to_bytes()).address()
}

fn staking_code() -> Bytes {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/configs/predeploys-alloc.json"
    );
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let code = json["alloc"]["0x0000000000000000000000000000000000001000"]["code"]
        .as_str()
        .unwrap();
    xdevs_primitives::hex::decode(code.trim_start_matches("0x"))
        .unwrap()
        .into()
}

/// 4 validators with 1,000 tXDS each, registered in Staking; staking active from block 1,
/// 3-block epochs.
fn genesis() -> Genesis {
    let mut g = Genesis::dev(DEV_CHAIN_ID);
    g.validators = (0..N)
        .map(|i| GenesisValidator {
            name: Some(format!("val{i}")),
            consensus_key: sk(i).verification_key().to_bytes(),
            power: 1_000,
            p2p: None,
            extra: Default::default(),
        })
        .collect();
    g.config.staking_validators_block = Some(1);
    g.config.epoch_length = EPOCH;
    g.config.block_reward = ether(1);
    let stakes: Vec<GenesisStake> = (0..N)
        .map(|i| GenesisStake {
            operator: DEV_ACCOUNTS[i],
            consensus_key: B256::from(sk(i).verification_key().to_bytes()),
            commission_bps: 1_000,
            stake: ether(1_000),
        })
        .collect();
    let params = StakingParams {
        min_self_stake: ether(1_000),
        ..StakingParams::default()
    };
    let code = staking_code();
    let (balance, storage) = genesis_storage(
        &EvmExecutor::new(g.chain_config()),
        code.clone(),
        GOVERNANCE_ADDRESS,
        &params,
        &stakes,
    )
    .unwrap();
    g.alloc.insert(
        STAKING_ADDRESS,
        GenesisAccount {
            balance,
            code: Some(code),
            storage,
            nonce: 0,
        },
    );
    g.validate().unwrap();
    g
}

struct TestNode {
    db: Db,
    app: ChainApp,
    cfg: ChainConfig,
}

fn node(g: &Genesis, me: usize) -> TestNode {
    let db: Db = Arc::new(MemDb::new());
    chain::init_genesis(&*db, g).unwrap();
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
        pool,
        Arc::new(EvmExecutor::new(cfg.clone())),
        cfg.clone(),
        crate::consensus::genesis_validator_set(g).unwrap().unwrap(),
        ChainAppConfig {
            fee_recipient: Address::repeat_byte(0xfe),
            validator_address: Some(address(me)),
            ..Default::default()
        },
    );
    TestNode { db, app, cfg }
}

/// A certificate for `block` signed by validators `signers`.
fn cert(block: &Block, signers: &[usize]) -> CommitCertificate {
    let msg = vote_sign_bytes(
        DEV_CHAIN_ID,
        VoteType::Precommit,
        block.number(),
        0,
        Some(block.hash()),
    );
    let sigs = signers
        .iter()
        .map(|i| CommitSig {
            validator: address(*i),
            signature: sk(*i).sign(&msg).to_bytes(),
        })
        .collect();
    CommitCertificate::new(block.number(), 0, block.hash(), sigs)
}

fn far() -> Instant {
    Instant::now() + std::time::Duration::from_secs(30)
}

/// `nodes[proposer]` proposes the next block, every node validates it, every node commits
/// it with a certificate of validators 0, 1 and 2.
async fn step(nodes: &[&TestNode], proposer: usize) -> Block {
    let height = nodes[0].app.head().await.unwrap().0 + 1;
    let block = nodes[proposer].app.propose(height, 0, far()).await.unwrap();
    for n in nodes {
        assert_eq!(
            n.app
                .validate(height, &block, BlockOrigin::Consensus)
                .await
                .unwrap(),
            Validity::Valid,
            "height {height}"
        );
    }
    for n in nodes {
        n.app
            .commit(block.clone(), cert(&block, &[0, 1, 2]))
            .await
            .unwrap();
    }
    block
}

/// Re-seal `block` with other consensus data (the header commits to it again).
fn with_data(block: &Block, data: &BlockConsensusData) -> Block {
    let mut b = block.clone();
    b.consensus = Bytes::from(data.to_bytes());
    b.header.requests_hash = Some(consensus_data_hash(&b.consensus));
    b
}

async fn validity(n: &TestNode, block: &Block) -> Validity {
    n.app
        .validate(block.number(), block, BlockOrigin::Sync)
        .await
        .unwrap()
}

fn staking_info(n: &TestNode, operator: Address) -> xdevs_execution::staking::ValidatorInfo {
    let snap = n.db.snapshot().unwrap();
    let (_, head) = chain::latest_header(&*snap).unwrap();
    let state = DbStateProvider::new(&*snap);
    let exec = EvmExecutor::new(n.cfg.clone());
    let (out, _) = exec
        .system_call_at(
            &state,
            &head,
            STAKING_ADDRESS,
            calls::get_validator(operator),
        )
        .unwrap();
    decode_validator_info(&out).unwrap()
}

#[tokio::test]
async fn proposals_carry_verified_consensus_data() {
    let g = genesis();
    let (a, b) = (node(&g, 0), node(&g, 1));

    // Block 1: the proposer, no last commit (there is no block 0 certificate).
    let b1 = step(&[&a, &b], 0).await;
    let d1 = BlockConsensusData::from_bytes(&b1.consensus).unwrap();
    assert_eq!(d1.proposer, address(0));
    assert!(d1.last_commit.is_empty() && d1.evidence.is_empty());
    assert!(b1.consensus_matches_header());
    // Stored and served with the block.
    let (served, _) = a.app.decided(1).await.unwrap().unwrap();
    assert_eq!(served, b1);

    // Block 2 carries block 1's certificate.
    let b2 = b.app.propose(2, 0, far()).await.unwrap();
    let d2 = BlockConsensusData::from_bytes(&b2.consensus).unwrap();
    assert_eq!(d2.proposer, address(1));
    assert_eq!(d2.last_commit(), Some(&cert(&b1, &[0, 1, 2])));
    assert_eq!(validity(&a, &b2).await, Validity::Valid);

    // Invalid variants of block 2.
    let mut stripped = b2.clone();
    stripped.consensus = Bytes::new();
    assert_eq!(
        validity(&a, &stripped).await,
        Validity::Invalid,
        "data/header mismatch"
    );
    let mut no_data = stripped.clone();
    no_data.header.requests_hash = Some(consensus_data_hash(&[]));
    assert_eq!(
        validity(&a, &no_data).await,
        Validity::Invalid,
        "missing data"
    );
    let outsider = BlockConsensusData {
        proposer: ValidatorAddress([9; 20]),
        ..d2.clone()
    };
    assert_eq!(
        validity(&a, &with_data(&b2, &outsider)).await,
        Validity::Invalid
    );
    let short_commit = BlockConsensusData {
        last_commit: vec![cert(&b1, &[0, 1])],
        ..d2.clone()
    };
    assert_eq!(
        validity(&a, &with_data(&b2, &short_commit)).await,
        Validity::Invalid
    );
    let wrong_commit = BlockConsensusData {
        last_commit: vec![cert(&b2, &[0, 1, 2])],
        ..d2.clone()
    };
    assert_eq!(
        validity(&a, &with_data(&b2, &wrong_commit)).await,
        Validity::Invalid
    );
    let no_commit = BlockConsensusData {
        last_commit: vec![],
        ..d2.clone()
    };
    assert_eq!(
        validity(&a, &with_data(&b2, &no_commit)).await,
        Validity::Invalid
    );
    // Re-sealed with a different proposer of the set: the consensus data is valid, but
    // the header no longer matches the execution (other proposer bonus): invalid too.
    let other = BlockConsensusData {
        proposer: address(2),
        ..d2.clone()
    };
    assert_eq!(
        validity(&a, &with_data(&b2, &other)).await,
        Validity::Invalid
    );
}

#[tokio::test]
async fn epoch_boundary_fixes_the_next_set_and_evidence_slashes() {
    let g = genesis();
    let (a, b) = (node(&g, 0), node(&g, 1));
    let genesis_set = a.app.validator_set_at(1).await.unwrap();
    assert_eq!(genesis_set.len(), N);
    for h in 1..=EPOCH {
        step(&[&a, &b], (h % 2) as usize).await;
    }
    // Block 3 was the boundary: the Staking set (unchanged) governs 4..=6.
    for n in [&a, &b] {
        let set = n.app.validator_set_at(EPOCH + 1).await.unwrap();
        assert_eq!(set, genesis_set);
        assert_eq!(set.total_voting_power(), 4_000);
        assert!(
            n.app.validator_set_at(2 * EPOCH + 1).await.is_err(),
            "not known yet"
        );
        assert!(n
            .db
            .get::<xdevs_storage::tables::ValidatorSets>(&EPOCH)
            .unwrap()
            .is_some());
    }

    // Validator 3 double-signs at height 3.
    let sk3 = sk(3);
    let vote = |hash: Option<B256>| {
        let msg = vote_sign_bytes(DEV_CHAIN_ID, VoteType::Precommit, 3, 0, hash);
        SignedVote {
            vote_type: VoteType::Precommit,
            height: 3,
            round: 0,
            block_hash: hash,
            validator: address(3),
            signature: sk3.sign(&msg).to_bytes(),
        }
    };
    let evidence = ConflictingVotes {
        first: vote(Some(B256::repeat_byte(1))),
        second: vote(None),
    };
    let mut forged = evidence;
    forged.second.signature[0] ^= 1;
    assert!(a.app.submit_evidence(&forged.to_bytes()).await.is_err());
    assert!(a.app.submit_evidence(b"junk").await.is_err());
    let hash = a.app.submit_evidence(&evidence.to_bytes()).await.unwrap();
    assert_eq!(hash, evidence.canonical().hash());
    assert_eq!(a.app.pending_evidence(), 1);

    // a proposes block 4 with the evidence; b (without it in its pool) accepts it.
    let b4 = step(&[&a, &b], 0).await;
    let d4 = BlockConsensusData::from_bytes(&b4.consensus).unwrap();
    assert_eq!(d4.evidence, vec![evidence.canonical()]);
    assert_eq!(
        a.app.pending_evidence(),
        0,
        "included evidence leaves the pool"
    );
    for n in [&a, &b] {
        let info = staking_info(n, DEV_ACCOUNTS[3]);
        assert_eq!(info.status, ValidatorStatus::Tombstoned);
        assert_eq!(info.total_stake, ether(950));
    }
    // Non-canonical or duplicated evidence in a block is refused.
    let mut reversed = d4.clone();
    reversed.evidence = vec![ConflictingVotes {
        first: evidence.canonical().second,
        second: evidence.canonical().first,
    }];
    let next = a.app.propose(5, 0, far()).await.unwrap();
    assert_eq!(
        validity(
            &b,
            &with_data(
                &next,
                &BlockConsensusData {
                    evidence: reversed.evidence.clone(),
                    ..BlockConsensusData::from_bytes(&next.consensus).unwrap()
                }
            )
        )
        .await,
        Validity::Invalid
    );

    // Blocks 5 and 6: block 6 is the boundary, the tombstoned validator leaves.
    step(&[&a, &b], 1).await;
    step(&[&a, &b], 0).await;
    for n in [&a, &b] {
        let set = n.app.validator_set_at(2 * EPOCH + 1).await.unwrap();
        assert_eq!(set.len(), N - 1);
        assert!(set.get(&address(3)).is_none());
        assert_eq!(n.app.validator_set(2 * EPOCH).await.unwrap(), genesis_set);
    }
    // The next epoch's blocks are signed by the 3 remaining validators.
    step(&[&a, &b], 0).await;
}
