//! End-to-end executor tests on [`InMemoryState`].

use crate::*;
use alloy_consensus::{Eip658Value, SignableTransaction};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use xdevs_primitives::AccountInfo;
use xdevs_primitives::{
    b256, compute_receipts_root, constants::GWEI, set_compat_header_fields, Bloom, LogData,
    Receipt, ReceiptWithBloom, Signed, TxEip1559, TxEnvelope, TxKind, TxLegacy, U256,
};
use xdevs_state::InMemoryState;

const KEY0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const KEY1: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const BENEFICIARY: Address = Address::repeat_byte(0xbe);
const ETHER: u128 = 1_000_000_000_000_000_000;

fn signer(key: &str) -> PrivateKeySigner {
    key.parse().unwrap()
}

fn cfg() -> ChainConfig {
    ChainConfig::dev()
}

#[allow(clippy::too_many_arguments)]
fn tx1559(
    s: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    to: TxKind,
    value: U256,
    input: Bytes,
    gas_limit: u64,
    tip: u128,
) -> RecoveredTx {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas: 3 * u128::from(GWEI),
        max_priority_fee_per_gas: tip,
        to,
        value,
        access_list: Default::default(),
        input,
    };
    let sig = s.sign_hash_sync(&tx.signature_hash()).unwrap();
    let env: TxEnvelope = Signed::new_unhashed(tx, sig).into();
    RecoveredTx::recover(env).unwrap()
}

fn transfer(s: &PrivateKeySigner, nonce: u64, to: Address, value: u64) -> RecoveredTx {
    tx1559(
        s,
        cfg().chain_id,
        nonce,
        TxKind::Call(to),
        U256::from(value),
        Bytes::new(),
        21_000,
        u128::from(GWEI),
    )
}

/// State with the two dev accounts funded with 10,000 ether each.
fn funded_state() -> InMemoryState {
    let mut st = InMemoryState::new();
    for k in [KEY0, KEY1] {
        st.insert_account(
            signer(k).address(),
            AccountInfo::new(0, U256::from(10_000 * ETHER)),
        );
    }
    st
}

/// A parent at exactly the gas target, so the child base fee stays at 1 gwei.
fn parent() -> Header {
    let mut h = Header {
        number: 0,
        gas_limit: 30_000_000,
        gas_used: 15_000_000,
        base_fee_per_gas: Some(GWEI),
        timestamp: 1_000,
        ..Default::default()
    };
    set_compat_header_fields(&mut h);
    h
}

fn attrs(parent: &Header) -> BlockAttributes {
    BlockAttributes {
        timestamp: parent.timestamp + 2,
        beneficiary: BENEFICIARY,
        prev_randao: B256::repeat_byte(0x42),
        extra_data: Bytes::from_static(b"xdevs"),
        gas_limit: 30_000_000,
    }
}

fn balance(st: &InMemoryState, a: Address) -> U256 {
    st.accounts().get(&a).map(|i| i.balance).unwrap_or_default()
}

fn total_supply(st: &InMemoryState) -> U256 {
    st.accounts().values().map(|i| i.balance).sum()
}

/// Build a block, re-execute it strictly and apply it; returns the block.
fn build_and_apply(
    exec: &EvmExecutor,
    st: &mut InMemoryState,
    parent: &Header,
    txs: Vec<RecoveredTx>,
) -> BuiltBlock {
    let built = exec.build(parent, attrs(parent), txs, &*st).unwrap();
    let replay = exec.execute(parent, &built.block, &*st).unwrap();
    assert_eq!(
        replay, built.outcome,
        "strict execution must reproduce the build"
    );
    st.apply(&built.outcome.state_diff);
    assert_eq!(st.state_root(), built.block.header.state_root);
    built
}

/// `PUSH1 n PUSH1 12 PUSH1 0 CODECOPY PUSH1 n PUSH1 0 RETURN` + runtime.
fn initcode(runtime: &[u8]) -> Bytes {
    let n = u8::try_from(runtime.len()).unwrap();
    let mut code = vec![
        0x60, n, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, n, 0x60, 0x00, 0xf3,
    ];
    code.extend_from_slice(runtime);
    code.into()
}

/// Storage contract: calldata non-empty → `sstore(0, calldataload(0))`, else return `sload(0)`.
const STORE_RUNTIME: [u8; 24] = [
    0x36, 0x15, 0x60, 0x0c, 0x57, 0x60, 0x00, 0x35, 0x60, 0x00, 0x55, 0x00, 0x5b, 0x60, 0x00, 0x54,
    0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
];
/// Always reverts with the 32-byte word 42.
const REVERT_RUNTIME: [u8; 10] = [0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd];

fn word(v: u64) -> Bytes {
    B256::from(U256::from(v)).0.to_vec().into()
}

#[test]
fn value_transfer_fees_add_up() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let (a, b) = (signer(KEY0), signer(KEY1));
    let supply_before = total_supply(&st);
    let a_before = balance(&st, a.address());
    let b_before = balance(&st, b.address());
    let parent = parent();

    let value = 1_234_567u64;
    let built = build_and_apply(
        &exec,
        &mut st,
        &parent,
        vec![transfer(&a, 0, b.address(), value)],
    );
    let header = &built.block.header;
    assert_eq!(header.number, 1);
    assert_eq!(header.parent_hash, parent.hash_slow());
    assert_eq!(header.base_fee_per_gas, Some(GWEI));
    assert_eq!(header.gas_used, 21_000);
    assert_eq!(header.beneficiary, BENEFICIARY);
    assert_eq!(header.mix_hash, B256::repeat_byte(0x42));
    assert_eq!(
        header.withdrawals_root,
        Some(xdevs_primitives::EMPTY_ROOT_HASH)
    );
    assert_eq!(
        header.transactions_root,
        xdevs_primitives::compute_transactions_root(&built.block.body)
    );
    assert_eq!(built.senders, vec![a.address()]);
    assert!(built.rejected.is_empty());

    let r = &built.outcome.receipts[0];
    assert!(r.is_success());
    assert_eq!(r.cumulative_gas_used(), 21_000);
    assert_eq!(r.tx_type(), xdevs_primitives::TxType::Eip1559);

    let gas = U256::from(21_000u64);
    let base = U256::from(GWEI);
    let tip = U256::from(GWEI);
    let base_total = gas * base;
    let treasury_share = base_total * U256::from(2_000u64) / U256::from(10_000u64);
    let burned = base_total - treasury_share;

    assert_eq!(
        balance(&st, a.address()),
        a_before - U256::from(value) - gas * (base + tip)
    );
    assert_eq!(st.accounts()[&a.address()].nonce, 1);
    assert_eq!(balance(&st, b.address()), b_before + U256::from(value));
    assert_eq!(balance(&st, BENEFICIARY), gas * tip);
    assert_eq!(balance(&st, cfg().treasury), treasury_share);
    // Everything paid is accounted for: tip + treasury + burn.
    assert_eq!(total_supply(&st), supply_before - burned);
    assert_eq!(
        burned,
        base_total * U256::from(8_000u64) / U256::from(10_000u64)
    );
}

#[test]
fn deploy_store_and_call() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let a = signer(KEY0);
    let contract = a.address().create(0);
    let p0 = parent();

    let deploy = tx1559(
        &a,
        cfg().chain_id,
        0,
        TxKind::Create,
        U256::ZERO,
        initcode(&STORE_RUNTIME),
        200_000,
        0,
    );
    let set = tx1559(
        &a,
        cfg().chain_id,
        1,
        TxKind::Call(contract),
        U256::ZERO,
        word(42),
        100_000,
        0,
    );
    let b1 = build_and_apply(&exec, &mut st, &p0, vec![deploy, set]);
    assert_eq!(b1.block.body.len(), 2);
    assert!(b1.outcome.receipts.iter().all(|r| r.is_success()));
    let r0 = b1.outcome.receipts[0].cumulative_gas_used();
    let r1 = b1.outcome.receipts[1].cumulative_gas_used();
    assert!(r1 > r0 && r1 == b1.block.header.gas_used);

    // Code and storage are part of the state diff / state.
    let info = st.accounts()[&contract];
    assert_eq!(info.nonce, 1);
    assert_eq!(st.code(contract).unwrap().as_ref(), &STORE_RUNTIME[..]);
    assert_eq!(st.storage(contract, B256::ZERO).unwrap(), U256::from(42u64));
    assert!(b1.outcome.state_diff.codes.contains_key(&info.code_hash));

    // eth_call: read back the stored value.
    let header = &b1.block.header;
    let get = TransactionRequest {
        to: Some(TxKind::Call(contract)),
        ..Default::default()
    };
    match exec.call(&st, header, &get, 50_000_000).unwrap() {
        CallResult::Success {
            output, gas_used, ..
        } => {
            assert_eq!(output, word(42));
            assert!(gas_used > 21_000);
        }
        other => panic!("unexpected {other:?}"),
    }

    // estimate_gas: minimal limit that succeeds (SSTORE needs the EIP-2200 stipend margin).
    let set_req = TransactionRequest {
        from: Some(a.address()),
        to: Some(TxKind::Call(contract)),
        input: word(7).into(),
        ..Default::default()
    };
    let est = exec
        .estimate_gas(&st, header, &set_req, 50_000_000)
        .unwrap();
    let with = |gas: u64| {
        let req = TransactionRequest {
            gas: Some(gas),
            ..set_req.clone()
        };
        exec.call(&st, header, &req, 50_000_000).unwrap()
    };
    assert!(with(est).is_success());
    assert!(!with(est - 1).is_success());

    // Second block on top of the first: overwrite the value.
    let set2 = tx1559(
        &a,
        cfg().chain_id,
        2,
        TxKind::Call(contract),
        U256::ZERO,
        word(7),
        est,
        0,
    );
    let b2 = build_and_apply(&exec, &mut st, header, vec![set2]);
    assert_eq!(b2.block.header.number, 2);
    assert!(b2.outcome.receipts[0].is_success());
    assert_eq!(st.storage(contract, B256::ZERO).unwrap(), U256::from(7u64));
}

#[test]
fn revert_gives_status_zero_and_still_charges() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let a = signer(KEY0);
    let before = balance(&st, a.address());
    // Initcode that reverts: the deployment fails.
    let bad_deploy = tx1559(
        &a,
        cfg().chain_id,
        0,
        TxKind::Create,
        U256::ZERO,
        Bytes::from_static(&REVERT_RUNTIME),
        100_000,
        0,
    );
    let b = build_and_apply(&exec, &mut st, &parent(), vec![bad_deploy]);
    let r = &b.outcome.receipts[0];
    assert!(!r.is_success());
    assert!(r.logs().is_empty());
    let used = r.cumulative_gas_used();
    assert!(used > 21_000 && used < 100_000);
    assert_eq!(st.accounts()[&a.address()].nonce, 1);
    assert_eq!(
        balance(&st, a.address()),
        before - U256::from(used) * U256::from(GWEI)
    );
    assert!(!st.accounts().contains_key(&a.address().create(0)));
}

#[test]
fn call_and_estimate_report_reverts() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let target = Address::repeat_byte(0x77);
    let hash = st.insert_code(Bytes::from_static(&REVERT_RUNTIME));
    st.insert_account(
        target,
        AccountInfo {
            nonce: 1,
            balance: U256::ZERO,
            code_hash: hash,
        },
    );
    let header = parent();
    let req = TransactionRequest {
        to: Some(TxKind::Call(target)),
        ..Default::default()
    };
    match exec.call(&st, &header, &req, 1_000_000).unwrap() {
        CallResult::Revert { output, .. } => assert_eq!(output, word(42)),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        exec.estimate_gas(&st, &header, &req, 1_000_000),
        Err(ExecutionError::Reverted { output: word(42) })
    );
    // A plain transfer estimates to exactly 21000.
    let xfer = TransactionRequest {
        from: Some(signer(KEY0).address()),
        to: Some(TxKind::Call(Address::repeat_byte(0x55))),
        value: Some(U256::from(1u64)),
        ..Default::default()
    };
    assert_eq!(exec.estimate_gas(&st, &header, &xfer, 0).unwrap(), 21_000);
}

#[test]
fn build_skips_and_rejects() {
    let exec = EvmExecutor::new(cfg());
    let st = funded_state();
    let (a, b) = (signer(KEY0), signer(KEY1));
    let to = Address::repeat_byte(9);
    let wrong_chain = tx1559(
        &b,
        1,
        0,
        TxKind::Call(to),
        U256::ZERO,
        Bytes::new(),
        21_000,
        0,
    );
    let gap = transfer(&a, 5, to, 1); // nonce ahead of state: skipped, not rejected
    let ok = transfer(&a, 0, to, 1); // same sender after a skip: skipped too
    let built = exec
        .build(
            &parent(),
            attrs(&parent()),
            vec![wrong_chain.clone(), gap, ok],
            &st,
        )
        .unwrap();
    assert!(built.block.body.is_empty());
    assert_eq!(built.rejected.len(), 1);
    assert_eq!(built.rejected[0].0, wrong_chain.hash());
    assert_eq!(built.block.header.gas_used, 0);
    assert_eq!(
        built.block.header.receipts_root,
        xdevs_primitives::EMPTY_ROOT_HASH
    );
    assert_eq!(built.block.header.state_root, st.state_root());
    // Nothing executed: no fee payouts, empty diff.
    assert!(built.outcome.state_diff.is_empty());

    // Nonce too low is a permanent rejection.
    let mut st2 = st.clone();
    st2.insert_account(a.address(), AccountInfo::new(3, U256::from(ETHER)));
    let built = exec
        .build(
            &parent(),
            attrs(&parent()),
            vec![transfer(&a, 0, to, 1)],
            &st2,
        )
        .unwrap();
    assert_eq!(built.rejected.len(), 1);
}

#[test]
fn strict_execute_rejects_bad_blocks() {
    let exec = EvmExecutor::new(cfg());
    let st = funded_state();
    let a = signer(KEY0);
    let p = parent();
    let built = exec
        .build(
            &p,
            attrs(&p),
            vec![transfer(&a, 0, Address::repeat_byte(9), 1)],
            &st,
        )
        .unwrap();

    // Wrong base fee in header.
    let mut blk = built.block.clone();
    blk.header.base_fee_per_gas = Some(GWEI + 1);
    assert!(matches!(
        exec.execute(&p, &blk, &st),
        Err(ExecutionError::InvalidHeader(_))
    ));

    // Same tx twice: the second one has a stale nonce.
    let mut blk = built.block.clone();
    blk.body.push(blk.body[0].clone());
    assert!(matches!(
        exec.execute(&p, &blk, &st),
        Err(ExecutionError::InvalidTransaction { .. })
    ));

    // Gas limit exceeded.
    let mut blk = built.block;
    blk.header.gas_limit = 20_000;
    blk.header.gas_used = 0;
    assert!(matches!(
        exec.execute(&p, &blk, &st),
        Err(ExecutionError::BlockGasLimitExceeded { .. })
    ));
}

#[test]
fn legacy_tx_and_tip_capped_by_fee_cap() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let a = signer(KEY0);
    let gas_price = 1_500_000_000u128; // 1.5 gwei: tip = 0.5 gwei
    let tx = TxLegacy {
        chain_id: Some(cfg().chain_id),
        nonce: 0,
        gas_price,
        gas_limit: 21_000,
        to: TxKind::Call(Address::repeat_byte(3)),
        value: U256::from(5u64),
        input: Bytes::new(),
    };
    let sig = a.sign_hash_sync(&tx.signature_hash()).unwrap();
    let env: TxEnvelope = Signed::new_unhashed(tx, sig).into();
    let b = build_and_apply(
        &exec,
        &mut st,
        &parent(),
        vec![RecoveredTx::recover(env).unwrap()],
    );
    assert_eq!(
        b.outcome.receipts[0].tx_type(),
        xdevs_primitives::TxType::Legacy
    );
    assert_eq!(
        balance(&st, BENEFICIARY),
        U256::from(21_000u64) * U256::from(gas_price - u128::from(GWEI))
    );
}

#[test]
fn receipts_root_known_vectors() {
    // alloy/reth vector: one EIP-2930 receipt with one empty log and a fixed bloom.
    let mut bloom = Bloom::ZERO;
    bloom.0[255] = 1;
    let receipt = ReceiptEnvelope::Eip2930(ReceiptWithBloom {
        receipt: Receipt {
            status: Eip658Value::success(),
            cumulative_gas_used: 102_068,
            logs: vec![Log {
                address: Address::ZERO,
                data: LogData::new_unchecked(vec![], Default::default()),
            }],
        },
        logs_bloom: bloom,
    });
    assert_eq!(
        compute_receipts_root(&[receipt]),
        b256!("0xfe70ae4a136d98944951b2123859698d59ad251a381abc9960fa81cae3d0d4a0")
    );

    // Real geth devnet block 0x315 (gasUsed 0x5208, blobGasUsed 0x20000): its only tx is
    // a successful type-3 tx without logs. Receipt encoding is the same for all types
    // apart from the type byte, so this checks the typed-receipt trie end to end.
    let only = ReceiptEnvelope::Eip4844(
        Receipt {
            status: Eip658Value::success(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        }
        .with_bloom(),
    );
    assert_eq!(
        compute_receipts_root(&[only]),
        b256!("0xeaa8c40899a61ae59615cf9985f5e2194f8fd2b57d273be63bde6733e89b12ab")
    );
}

#[test]
fn logs_fill_receipt_and_header_bloom() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let a = signer(KEY0);
    let emitter = a.address().create(0);
    // mstore(0, 42); log0(0, 32); stop
    let runtime = [
        0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xa0, 0x00,
    ];
    let deploy = tx1559(
        &a,
        cfg().chain_id,
        0,
        TxKind::Create,
        U256::ZERO,
        initcode(&runtime),
        200_000,
        0,
    );
    let call = tx1559(
        &a,
        cfg().chain_id,
        1,
        TxKind::Call(emitter),
        U256::ZERO,
        Bytes::new(),
        100_000,
        0,
    );
    let b = build_and_apply(&exec, &mut st, &parent(), vec![deploy, call]);
    let r = &b.outcome.receipts[1];
    assert_eq!(r.logs().len(), 1);
    assert_eq!(r.logs()[0].address, emitter);
    assert_eq!(r.logs()[0].data.data, word(42));
    assert_eq!(*r.logs_bloom(), xdevs_primitives::logs_bloom(r.logs()));
    assert_ne!(b.block.header.logs_bloom, Bloom::ZERO);
    assert_eq!(b.block.header.logs_bloom, b.outcome.logs_bloom);
    assert!(b.block.header.logs_bloom.contains_input(
        xdevs_primitives::reexports::alloy_primitives::BloomInput::Raw(emitter.as_slice())
    ));

    let req = TransactionRequest {
        to: Some(TxKind::Call(emitter)),
        ..Default::default()
    };
    match exec.call(&st, &b.block.header, &req, 0).unwrap() {
        CallResult::Success { logs, .. } => assert_eq!(logs.len(), 1),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn eip161_and_selfdestruct_leave_no_accounts() {
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let a = signer(KEY0);
    let fresh = Address::repeat_byte(0x99);
    let zero_xfer = transfer(&a, 0, fresh, 0);
    // Constructor: selfdestruct(caller) -> created and destroyed in one tx (EIP-6780).
    let suicide = tx1559(
        &a,
        cfg().chain_id,
        1,
        TxKind::Create,
        U256::from(5u64),
        Bytes::from_static(&[0x33, 0xff]),
        100_000,
        0,
    );
    let b = build_and_apply(&exec, &mut st, &parent(), vec![zero_xfer, suicide]);
    assert!(b.outcome.receipts.iter().all(|r| r.is_success()));
    assert!(!st.accounts().contains_key(&fresh));
    assert!(!st.accounts().contains_key(&a.address().create(1)));
    assert_eq!(st.accounts()[&a.address()].nonce, 2);
    // The zero-value transfer paid a 1 gwei tip; the zero-tip create paid none.
    assert_eq!(
        balance(&st, BENEFICIARY),
        U256::from(21_000u64) * U256::from(GWEI)
    );
    let changed = &b.outcome.state_diff.accounts;
    assert!(changed[&a.address()].is_some() && changed[&cfg().treasury].is_some());
    assert!(!changed.contains_key(&fresh));
}
