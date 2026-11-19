//! Local fee markets in block execution (Phase 5, plan §15.2).

use super::*;
use xdevs_primitives::local_fee::{self, LOCAL_FEE_MARKET_ADDRESS, LOCAL_FEE_MIN};
use xdevs_state::StateProvider;

/// A contract that burns all the gas it gets (`JUMPDEST PUSH1 0 JUMP`).
const HOT: Address = Address::repeat_byte(0x55);
const OTHER: Address = Address::repeat_byte(0x77);

fn hot_state() -> InMemoryState {
    let mut st = funded_state();
    let code_hash = st.insert_code(Bytes::from_static(&[0x5b, 0x60, 0x00, 0x56]));
    st.insert_account(
        HOT,
        AccountInfo {
            nonce: 1,
            balance: U256::ZERO,
            code_hash,
        },
    );
    st
}

fn local_fee_at(st: &InMemoryState, target: Address, number: u64) -> u64 {
    let word = st
        .storage(LOCAL_FEE_MARKET_ADDRESS, local_fee::slot(target))
        .unwrap();
    local_fee::fee_at(word, number)
}

/// 10M gas (a third of the block) to the hot contract per block raises its local fee;
/// an unrelated transfer in the same blocks keeps paying exactly the global base fee;
/// the whole base part (global + local) is split burn / treasury; Block-STM agrees.
#[test]
fn hot_contract_pays_a_local_fee_others_the_global_one() {
    let cfg = ChainConfig {
        local_fee_markets_block: Some(1),
        ..cfg()
    };
    let exec = EvmExecutor::new(cfg.clone());
    let (a, b) = (signer(KEY0), signer(KEY1));
    let g = u128::from(GWEI);
    let mut st = hot_state();
    let mut parent = parent();
    let mut fees = Vec::new();
    for round in 0..4u64 {
        let number = parent.number + 1;
        let local = local_fee_at(&st, HOT, number);
        let hot = |nonce| {
            tx1559(
                &a,
                cfg.chain_id,
                nonce,
                TxKind::Call(HOT),
                U256::ZERO,
                Bytes::new(),
                5_000_000,
                g,
            )
        };
        let txs = vec![
            hot(2 * round),
            hot(2 * round + 1),
            transfer(&b, round, OTHER, 1),
        ];
        let before = (
            balance(&st, a.address()),
            balance(&st, b.address()),
            balance(&st, cfg.treasury),
        );
        let built = exec.build(&parent, attrs(&parent), txs, &st).unwrap();
        assert_eq!(built.block.body.len(), 3, "round {round}");
        let replay = exec.execute(&parent, &built.block, &st).unwrap();
        assert_eq!(replay, built.outcome);
        let (par, stats) = exec
            .execute_parallel(&parent, &built.block, &st, &StakingInputs::default(), 4)
            .unwrap();
        assert_eq!(par, built.outcome);
        assert_eq!(stats.fallback, None);
        st.apply(&built.outcome.state_diff);
        assert_eq!(st.state_root(), built.block.header.state_root);

        let base = u128::from(built.block.header.base_fee_per_gas.unwrap());
        let local = u128::from(local);
        // Hot sender: 10M gas at global + local + tip.
        assert_eq!(
            before.0 - balance(&st, a.address()),
            U256::from(10_000_000 * (base + local + g)),
            "round {round}"
        );
        // Unrelated transfer: global base fee + tip, nothing else.
        assert_eq!(
            before.1 - balance(&st, b.address()),
            U256::from(21_000 * (base + g) + 1),
            "round {round}"
        );
        // Treasury: its share of the whole base part (global + local).
        let base_part = U256::from(10_000_000 * (base + local) + 21_000 * base);
        assert_eq!(
            balance(&st, cfg.treasury) - before.2,
            cfg.split_base_fee(base_part).1
        );
        fees.push(local as u64);
        parent = built.block.header.clone();
    }
    // Block 1 starts without local fees; congestion in block 1 prices block 2 at the
    // minimum, then it keeps rising while the contract uses > 25% of each block.
    assert_eq!(fees[0], 0);
    assert_eq!(fees[1], LOCAL_FEE_MIN);
    assert!(fees[2] > fees[1] && fees[3] > fees[2], "{fees:?}");
    // The transfer target has no local fee.
    assert_eq!(local_fee_at(&st, OTHER, parent.number + 1), 0);

    // Idle blocks: the fee decays (lazily, the table is not rewritten).
    let peak = local_fee_at(&st, HOT, parent.number + 1);
    let idle = build_and_apply(&exec, &mut st, &parent, vec![transfer(&b, 4, OTHER, 1)]);
    assert!(local_fee_at(&st, HOT, idle.block.header.number + 1) < peak);
}

/// Before the activation nothing changes and the table account is never created.
#[test]
fn inactive_before_the_activation_block() {
    let cfg = ChainConfig {
        local_fee_markets_block: Some(100),
        ..cfg()
    };
    let exec = EvmExecutor::new(cfg.clone());
    let a = signer(KEY0);
    let mut st = hot_state();
    let txs = (0..4)
        .map(|n| {
            tx1559(
                &a,
                cfg.chain_id,
                n,
                TxKind::Call(HOT),
                U256::ZERO,
                Bytes::new(),
                5_000_000,
                u128::from(GWEI),
            )
        })
        .collect();
    let built = build_and_apply(&exec, &mut st, &parent(), txs);
    assert_eq!(built.block.body.len(), 4);
    assert!(st
        .basic_account(LOCAL_FEE_MARKET_ADDRESS)
        .unwrap()
        .is_none());
}
