//! Differential tests of Block-STM (plan §10.5, §26 "Differential"): every generated
//! block is executed sequentially and in parallel (several thread counts) and the
//! outcomes — receipts, logs, blooms, gas, state diff, state root, or the error of an
//! invalid block — must be identical.

mod workload;

use workload::{Mix, Workload, BENEFICIARY, COUNTER, SELFDESTRUCT_TO_COINBASE_INIT};
use xdevs_primitives::{AccountInfo, Block, TxKind, B256, U256};
use xdevs_state::StateProvider;

/// Property test: random mixed workloads (transfers, hot slots, self-calls, storage
/// collisions, reverts, out-of-gas, deployments called in the same block, constructors
/// self-destructing to COINBASE, contracts reading the beneficiary / treasury balance)
/// over many seeds and chained blocks.
#[test]
fn random_mixed_blocks_match_sequential() {
    let mut parallel_blocks = 0;
    let mut reexecutions = 0;
    for seed in 0..24u64 {
        let mut w = Workload::new(seed, 12);
        for _ in 0..3 {
            let count = 30 + w.rng.below(90) as usize;
            let txs = w.random_txs(Mix::Mixed, count);
            let block = w.build(txs);
            let mut outcome = None;
            for threads in [1, 2, 3 + (seed as usize % 6)] {
                let (o, stats) = w.check(&block, threads).expect("valid block");
                if stats.fallback.is_none() {
                    parallel_blocks += 1;
                }
                reexecutions += stats.executions.saturating_sub(stats.transactions);
                outcome = Some(o);
            }
            w.advance(&block, &outcome.expect("executed"));
        }
    }
    // The workload produces conflicts, and they are resolved by re-execution.
    assert!(
        parallel_blocks > 150,
        "only {parallel_blocks} parallel blocks"
    );
    assert!(reexecutions > 0, "no conflict was ever detected");
}

#[test]
fn hot_spot_erc20_and_independent_blocks_match() {
    for mix in [Mix::HotSpot, Mix::Erc20, Mix::Independent] {
        let mut w = Workload::new(7, 16);
        for round in 0..2 {
            let txs = w.random_txs(mix, 120);
            let block = w.build(txs);
            let (outcome, stats) = w.check(&block, 4).expect("valid block");
            assert_eq!(stats.fallback, None, "{mix:?}");
            assert_eq!(stats.transactions, 120);
            w.advance(&block, &outcome);
            if mix == Mix::HotSpot {
                // Every increment of the single hot slot is there.
                let counter = w.state.storage(COUNTER, B256::ZERO).unwrap();
                assert_eq!(counter, U256::from(120 * (round + 1)));
            }
        }
    }
}

#[test]
fn deterministic_for_every_thread_count() {
    let mut w = Workload::new(99, 10);
    let txs = w.random_txs(Mix::Mixed, 150);
    let block = w.build(txs);
    let (first, _) = w.check(&block, 1).unwrap();
    for threads in 2..=12 {
        for _ in 0..3 {
            let (o, _) = w.check(&block, threads).unwrap();
            assert_eq!(o, first);
        }
    }
}

#[test]
fn empty_existing_beneficiary_falls_back() {
    let mut w = Workload::new(3, 6);
    // Sequential execution deletes it (EIP-161) when a zero tip touches it.
    w.state.insert_account(BENEFICIARY, AccountInfo::default());
    let txs = w.random_txs(Mix::Mixed, 40);
    let block = w.build(txs);
    let (_, stats) = w.check(&block, 4).unwrap();
    assert_eq!(stats.fallback, Some("beneficiary exists but is empty"));
}

#[test]
fn destroyed_beneficiary_falls_back() {
    let mut w = Workload::new(4, 6);
    // The beneficiary is the address of a contract whose constructor self-destructs to
    // COINBASE, i.e. to itself: sequential execution drops that tx's tip.
    let creator = w.signers[0].address();
    let mut txs = w.random_txs(Mix::Independent, 5);
    let doomed = creator.create(w.nonces[0]);
    txs.push(w.tx(
        0,
        TxKind::Create,
        U256::from(9u64),
        SELFDESTRUCT_TO_COINBASE_INIT.to_vec().into(),
        100_000,
    ));
    txs.extend(w.random_txs(Mix::Independent, 5));
    let mut attrs = w.attrs();
    attrs.beneficiary = doomed;
    let built = w
        .exec
        .build_with(&w.parent, attrs, txs, &w.state, &Default::default())
        .unwrap();
    let (_, stats) = w.check(&built.block, 4).unwrap();
    assert_eq!(
        stats.fallback,
        Some("a transaction destroyed the beneficiary")
    );
}

#[test]
fn invalid_blocks_fail_identically() {
    let mut w = Workload::new(5, 8);
    let txs = w.random_txs(Mix::Mixed, 30);
    let replayed = txs[3].tx.clone();
    let block = w.build(txs);

    // A replayed transaction (nonce too low) in the middle of the block.
    let mut body = block.body.clone();
    body.insert(20, replayed);
    let bad = Block::new(block.header.clone(), body);
    let err = w.check(&bad, 4).unwrap_err();
    assert!(err.contains("nonce"), "{err}");

    // Not enough block gas for all transactions.
    let mut header = block.header.clone();
    header.gas_limit = 200_000;
    header.gas_used = 0;
    let bad = Block::new(header, block.body.clone());
    let err = w.check(&bad, 4).unwrap_err();
    assert!(err.contains("gas"), "{err}");
}

#[test]
fn state_is_readable_through_trait_object() {
    // `execute_with` dispatches on the executor's configuration.
    let mut w = Workload::new(11, 6);
    let txs = w.random_txs(Mix::Erc20, 20);
    let block = w.build(txs);
    let seq = w
        .exec
        .execute_with(&w.parent, &block, &w.state, &Default::default())
        .unwrap();
    let par_exec = w.exec.clone().with_parallel(3);
    assert_eq!(par_exec.parallel_threads(), Some(3));
    let state: &(dyn StateProvider + Sync) = &w.state;
    let par = par_exec
        .execute_with(&w.parent, &block, state, &Default::default())
        .unwrap();
    assert_eq!(seq, par);
    assert!(xdevs_execution::parallel::parallel_blocks_executed() > 0);
}
