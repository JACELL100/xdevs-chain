//! XIP-0001 (EOA lock) test vectors of `docs/specs/xip-0001-eoa-lock.md` §10, plus
//! EIP-7702 execution and the P-256 precompile.
//!
//! The canonical account implementation is not needed: tests install the EIP-7702
//! designator and the lock word directly in the state, and put a tiny mock at `0x…1005`
//! that writes `calldata[0..32]` into the lock slot when called with exactly 32 bytes of
//! calldata (a stand-in for `enableEoaLock`, a guardian `freeze`, a matured
//! `disableEoaLock`, ...). Any other call is a no-op.

use super::*;
use alloy_eips::eip7702::{Authorization, SignedAuthorization};
use xdevs_primitives::{
    xip0001::{ACCOUNT_IMPL, DESIGNATOR, DESIGNATOR_HASH, LOCK_SLOT},
    EoaLockStatus, TxEip7702,
};
use xdevs_state::StateProvider;

/// Dev keys #2 and #3 (guardian `G`, unlocked authority `C`).
const KEY2: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
const KEY3: &str = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
/// Some other address (recipient / token).
const F: Address = Address::repeat_byte(0xf1);
/// "Any contract" used as a delegation target.
const X: Address = Address::repeat_byte(0x58);

/// `A`: the 7702 account under test (dev key #0).
fn a() -> PrivateKeySigner {
    signer(KEY0)
}

/// Mock account implementation (see module docs).
fn mock_impl() -> Bytes {
    // CALLDATASIZE PUSH1 32 EQ PUSH1 8 JUMPI STOP JUMPDEST
    // PUSH1 0 CALLDATALOAD PUSH32 LOCK_SLOT SSTORE STOP
    let mut code = vec![
        0x36, 0x60, 0x20, 0x14, 0x60, 0x08, 0x57, 0x00, 0x5b, 0x60, 0x00, 0x35, 0x7f,
    ];
    code.extend_from_slice(LOCK_SLOT.as_slice());
    code.extend_from_slice(&[0x55, 0x00]);
    code.into()
}

fn set_code(st: &mut InMemoryState, addr: Address, code: Bytes, nonce: u64, balance: U256) {
    let code_hash = st.insert_code(code);
    st.insert_account(
        addr,
        AccountInfo {
            nonce,
            balance,
            code_hash,
        },
    );
}

/// Funded dev accounts, the mock at `0x…1005`, and `A` delegated to it with `word`.
fn state_with_word(word: U256) -> InMemoryState {
    let mut st = funded_state();
    set_code(&mut st, ACCOUNT_IMPL, mock_impl(), 1, U256::ZERO);
    set_code(
        &mut st,
        a().address(),
        Bytes::from_static(&DESIGNATOR),
        0,
        U256::from(100 * ETHER),
    );
    st.set_storage(a().address(), LOCK_SLOT, word);
    for k in [KEY2, KEY3] {
        st.insert_account(signer(k).address(), AccountInfo::new(0, U256::from(ETHER)));
    }
    st
}

fn locked_state(word: u64) -> InMemoryState {
    state_with_word(U256::from(word))
}

/// EIP-1559 tx with an explicit max fee (tip 0.1 gwei, capped by the max fee).
#[allow(clippy::too_many_arguments)]
fn tx(
    s: &PrivateKeySigner,
    nonce: u64,
    to: TxKind,
    value: u128,
    input: Bytes,
    gas_limit: u64,
    max_fee: u128,
) -> RecoveredTx {
    let tx = TxEip1559 {
        chain_id: cfg().chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas: max_fee,
        max_priority_fee_per_gas: (u128::from(GWEI) / 10).min(max_fee),
        to,
        value: U256::from(value),
        access_list: Default::default(),
        input,
    };
    let sig = s.sign_hash_sync(&tx.signature_hash()).unwrap();
    RecoveredTx::recover(Signed::new_unhashed(tx, sig).into()).unwrap()
}

fn legacy(s: &PrivateKeySigner, nonce: u64, to: Address, gas_price: u128) -> RecoveredTx {
    let tx = TxLegacy {
        chain_id: Some(cfg().chain_id),
        nonce,
        gas_price,
        gas_limit: 100_000,
        to: TxKind::Call(to),
        value: U256::ZERO,
        input: Bytes::new(),
    };
    let sig = s.sign_hash_sync(&tx.signature_hash()).unwrap();
    RecoveredTx::recover(Signed::new_unhashed(tx, sig).into()).unwrap()
}

const TWO_GWEI: u128 = 2 * GWEI as u128;

/// V2 shape: `to=A, value=0, gas=300000, maxFee=2 gwei, data=<execute(...)>`.
fn v2(nonce: u64) -> RecoveredTx {
    // Not 32 bytes long: a no-op for the mock (stands for `execute(F, 0.5 tXDS, "")`).
    let data = Bytes::from_static(&[0xb6, 0x1d, 0x27, 0xf6, 0, 0, 0]);
    tx(
        &a(),
        nonce,
        TxKind::Call(a().address()),
        0,
        data,
        300_000,
        TWO_GWEI,
    )
}

/// Self-call writing `word` into the lock slot (mock `enableEoaLock` & co.).
fn set_word_call(s: &PrivateKeySigner, nonce: u64, word_value: u64) -> RecoveredTx {
    tx(
        s,
        nonce,
        TxKind::Call(a().address()),
        0,
        word(word_value),
        100_000,
        TWO_GWEI,
    )
}

/// V4 shape: `to=F, value=0`.
fn v4(nonce: u64) -> RecoveredTx {
    tx(
        &a(),
        nonce,
        TxKind::Call(F),
        0,
        Bytes::new(),
        50_000,
        TWO_GWEI,
    )
}

/// Plain transfer of 1 tXDS from `A` to `F`.
fn a_transfer(nonce: u64) -> RecoveredTx {
    tx(
        &a(),
        nonce,
        TxKind::Call(F),
        ETHER,
        Bytes::new(),
        21_000,
        TWO_GWEI,
    )
}

fn cfg_xip(at: Option<u64>) -> ChainConfig {
    ChainConfig {
        xip0001_block: at,
        ..cfg()
    }
}

fn parent_at(number: u64) -> Header {
    let mut h = parent();
    h.number = number;
    h
}

/// Strict validity of `txs` as one block on top of `p`: the block is assembled by an
/// executor without XIP-0001 (it includes everything), then executed by `exec`.
fn strict_with(
    exec: &EvmExecutor,
    st: &InMemoryState,
    p: &Header,
    txs: Vec<RecoveredTx>,
) -> Result<ExecutionOutcome, ExecutionError> {
    let n = txs.len();
    let built = EvmExecutor::new(cfg_xip(None))
        .build(p, attrs(p), txs, st)
        .unwrap();
    assert_eq!(built.block.body.len(), n, "assembled without XIP-0001");
    exec.execute(p, &built.block, st)
}

fn strict(st: &InMemoryState, txs: Vec<RecoveredTx>) -> Result<ExecutionOutcome, ExecutionError> {
    strict_with(&EvmExecutor::new(cfg()), st, &parent(), txs)
}

/// `None` if valid, else the XIP-0001 reason (panics on any other error).
fn reason(r: Result<ExecutionOutcome, ExecutionError>) -> Option<&'static str> {
    match r {
        Ok(_) => None,
        Err(ExecutionError::Xip0001 { hash, error }) => {
            assert!(hash.is_some());
            Some(error.violation.reason())
        }
        Err(e) => panic!("unexpected error: {e}"),
    }
}

/// Builder result for `txs`: (included hashes, rejected hashes with reasons).
fn build(st: &InMemoryState, txs: Vec<RecoveredTx>) -> BuiltBlock {
    EvmExecutor::new(cfg())
        .build(&parent(), attrs(&parent()), txs, st)
        .unwrap()
}

fn included(b: &BuiltBlock) -> Vec<TxHash> {
    b.block.tx_hashes().collect()
}

fn rejected_reasons(b: &BuiltBlock) -> Vec<(TxHash, &'static str)> {
    b.rejected
        .iter()
        .map(|(h, e)| match e {
            ExecutionError::Xip0001 { error, .. } => (*h, error.violation.reason()),
            other => panic!("unexpected rejection {other}"),
        })
        .collect()
}

fn status_of(st: &InMemoryState, addr: Address) -> EoaLockStatus {
    st.eoa_lock_status(addr).unwrap()
}

fn code_of(st: &InMemoryState, addr: Address) -> Bytes {
    st.code(addr).unwrap()
}

fn nonce_of(st: &InMemoryState, addr: Address) -> u64 {
    st.accounts()
        .get(&addr)
        .map(|a| a.nonce)
        .unwrap_or_default()
}

fn delegation_to(target: Address) -> Bytes {
    let mut code = vec![0xef, 0x01, 0x00];
    code.extend_from_slice(target.as_slice());
    code.into()
}

// ---------------------------------------------------------------------------
// Classification from state
// ---------------------------------------------------------------------------

#[test]
fn status_from_state() {
    let addr = a().address();
    assert_eq!(status_of(&locked_state(0), addr), EoaLockStatus::Unlocked);
    assert_eq!(status_of(&locked_state(1), addr), EoaLockStatus::Locked);
    assert_eq!(status_of(&locked_state(2), addr), EoaLockStatus::Sealed);
    assert_eq!(
        status_of(&locked_state(0xdead), addr),
        EoaLockStatus::Locked
    );
    // Missing account / plain EOA with a word.
    let mut st = funded_state();
    st.set_storage(signer(KEY1).address(), LOCK_SLOT, U256::from(2u64));
    assert_eq!(
        status_of(&st, signer(KEY1).address()),
        EoaLockStatus::Unlocked
    );
    assert_eq!(status_of(&st, F), EoaLockStatus::Unlocked);
    // V11: a contract (real bytecode) with word 1 is unlocked.
    set_code(&mut st, F, mock_impl(), 1, U256::ZERO);
    st.set_storage(F, LOCK_SLOT, U256::from(1u64));
    assert_eq!(status_of(&st, F), EoaLockStatus::Unlocked);
    // V10: delegated elsewhere.
    let mut st = locked_state(1);
    set_code(
        &mut st,
        addr,
        delegation_to(Address::with_last_byte(6)),
        0,
        U256::from(ETHER),
    );
    assert_eq!(status_of(&st, addr), EoaLockStatus::Unlocked);
    assert_eq!(keccak(&DESIGNATOR), DESIGNATOR_HASH);
}

fn keccak(b: &[u8]) -> B256 {
    xdevs_primitives::keccak256(b)
}

// ---------------------------------------------------------------------------
// §10 transaction validity, single transaction (strict execution and builder)
// ---------------------------------------------------------------------------

#[test]
fn single_tx_vectors() {
    let gwei = u128::from(GWEI);
    let a_addr = a().address();
    let cases: Vec<(&str, u64, RecoveredTx, Option<&str>)> = vec![
        (
            "V1",
            0,
            tx(
                &a(),
                0,
                TxKind::Call(F),
                ETHER,
                Bytes::new(),
                21_000,
                100 * gwei,
            ),
            None,
        ),
        ("V2", 1, v2(0), None),
        (
            "V3",
            1,
            tx(
                &a(),
                0,
                TxKind::Call(a_addr),
                1,
                Bytes::new(),
                50_000,
                TWO_GWEI,
            ),
            Some("XIP0001_VALUE_NOT_ZERO"),
        ),
        ("V4", 1, v4(0), Some("XIP0001_NOT_SELF_CALL")),
        (
            "V5",
            1,
            tx(&a(), 0, TxKind::Create, 0, Bytes::new(), 100_000, TWO_GWEI),
            Some("XIP0001_NOT_SELF_CALL"),
        ),
        (
            "V7a",
            1,
            tx(
                &a(),
                0,
                TxKind::Call(a_addr),
                0,
                Bytes::new(),
                2_000_000,
                TWO_GWEI,
            ),
            None,
        ),
        (
            "V7b",
            1,
            tx(
                &a(),
                0,
                TxKind::Call(a_addr),
                0,
                Bytes::new(),
                2_000_001,
                TWO_GWEI,
            ),
            Some("XIP0001_GAS_LIMIT"),
        ),
        (
            "V8a",
            1,
            tx(
                &a(),
                0,
                TxKind::Call(a_addr),
                0,
                Bytes::new(),
                50_000,
                2_000_000_000,
            ),
            None,
        ),
        (
            "V8b",
            1,
            tx(
                &a(),
                0,
                TxKind::Call(a_addr),
                0,
                Bytes::new(),
                50_000,
                2_000_000_001,
            ),
            Some("XIP0001_FEE_CAP"),
        ),
        ("V8c-2gwei", 1, legacy(&a(), 0, a_addr, TWO_GWEI), None),
        (
            "V8c-3gwei",
            1,
            legacy(&a(), 0, a_addr, 3 * gwei),
            Some("XIP0001_FEE_CAP"),
        ),
        ("V13", 2, v2(0), Some("XIP0001_SEALED")),
        ("V13-transfer", 2, a_transfer(0), Some("XIP0001_SEALED")),
        ("V14-V2", 0xdead, v2(0), None),
        ("V14-V4", 0xdead, v4(0), Some("XIP0001_NOT_SELF_CALL")),
    ];
    for (name, w, t, expected) in cases {
        let st = locked_state(w);
        assert_eq!(
            reason(strict(&st, vec![t.clone()])),
            expected,
            "{name} strict"
        );
        let built = build(&st, vec![t.clone()]);
        match expected {
            None => {
                assert_eq!(included(&built), vec![t.hash()], "{name} built");
                assert!(built.rejected.is_empty(), "{name}");
            }
            // c3 is transient: skipped, not reported (stays in the pool).
            Some("XIP0001_FEE_CAP") => {
                assert!(included(&built).is_empty(), "{name}");
                assert!(built.rejected.is_empty(), "{name}");
            }
            Some(r) => {
                assert!(included(&built).is_empty(), "{name}");
                assert_eq!(rejected_reasons(&built), vec![(t.hash(), r)], "{name}");
            }
        }
    }
}

#[test]
fn v10_other_delegate_is_not_restricted() {
    let mut st = locked_state(1);
    set_code(
        &mut st,
        a().address(),
        delegation_to(Address::with_last_byte(6)),
        0,
        U256::from(100 * ETHER),
    );
    assert_eq!(reason(strict(&st, vec![a_transfer(0)])), None);
}

#[test]
fn locked_self_call_executes_and_pays_fees() {
    let exec = EvmExecutor::new(cfg());
    let mut st = locked_state(1);
    let built = build_and_apply(&exec, &mut st, &parent(), vec![v2(0)]);
    assert!(built.outcome.receipts[0].is_success());
    assert_eq!(nonce_of(&st, a().address()), 1);
    assert_eq!(status_of(&st, a().address()), EoaLockStatus::Locked);
}

// ---------------------------------------------------------------------------
// §10 same block (c1 / d)
// ---------------------------------------------------------------------------

#[test]
fn b1_one_tx_per_block_for_locked_sender() {
    let st = locked_state(1);
    // Priced at 1.5 x base fee (the wallet default of §5 e) so that tx2 still fits c3
    // after the base fee drops in the next (almost empty) block.
    let self_call = |nonce| {
        tx(
            &a(),
            nonce,
            TxKind::Call(a().address()),
            0,
            Bytes::new(),
            100_000,
            3 * u128::from(GWEI) / 2,
        )
    };
    let (t1, t2) = (self_call(0), self_call(1));
    assert_eq!(
        reason(strict(&st, vec![t1.clone(), t2.clone()])),
        Some("XIP0001_ONE_PER_BLOCK")
    );
    // The builder includes tx1 only; tx2 is skipped (not dropped)...
    let built = build(&st, vec![t1.clone(), t2.clone()]);
    assert_eq!(included(&built), vec![t1.hash()]);
    assert!(built.rejected.is_empty());
    // ...and goes into the next block.
    let exec = EvmExecutor::new(cfg());
    let mut st = st;
    let p = parent();
    let b1 = build_and_apply(&exec, &mut st, &p, vec![t1, t2.clone()]);
    let b2 = build_and_apply(&exec, &mut st, &b1.block.header, vec![t2.clone()]);
    assert_eq!(included(&b2), vec![t2.hash()]);
    // A tx priced at exactly 2 x the old base fee is above the cap of the next block
    // (base fee fell): skipped, not dropped (c3 is transient).
    let late = v2(2);
    let b3 = exec
        .build(&b2.block.header, attrs(&b2.block.header), vec![late], &st)
        .unwrap();
    assert!(b3.block.body.is_empty() && b3.rejected.is_empty());
    assert_eq!(nonce_of(&st, a().address()), 2);
}

#[test]
fn b2_lock_enabled_earlier_in_the_block() {
    let st = locked_state(0);
    let enable = set_word_call(&a(), 0, 1);
    let xfer = a_transfer(1);
    assert_eq!(
        reason(strict(&st, vec![enable.clone(), xfer.clone()])),
        Some("XIP0001_NOT_SELF_CALL")
    );
    let built = build(&st, vec![enable.clone(), xfer.clone()]);
    assert_eq!(included(&built), vec![enable.hash()]);
    assert_eq!(
        rejected_reasons(&built),
        vec![(xfer.hash(), "XIP0001_NOT_SELF_CALL")]
    );
    // The enabling tx itself was valid (unlocked at S0) and locked the account.
    let mut after = st.clone();
    after.apply(&built.outcome.state_diff);
    assert_eq!(status_of(&after, a().address()), EoaLockStatus::Locked);
}

#[test]
fn b3_earlier_unlocked_tx_counts_for_c1() {
    let st = locked_state(0);
    let enable = set_word_call(&a(), 0, 1);
    let t2 = v2(1);
    assert_eq!(
        reason(strict(&st, vec![enable.clone(), t2.clone()])),
        Some("XIP0001_ONE_PER_BLOCK")
    );
    let built = build(&st, vec![enable.clone(), t2]);
    assert_eq!(included(&built), vec![enable.hash()]);
    assert!(built.rejected.is_empty(), "c1 is transient");
}

#[test]
fn b4_b6_sealed_by_someone_else_in_the_block() {
    // B4: a guardian's freeze (word -> 2); B6: a recovery revoking the EOA key (-> 2).
    let st = locked_state(1);
    let g = signer(KEY2);
    let freeze = set_word_call(&g, 0, 2);
    let t2 = v2(0);
    assert_eq!(
        reason(strict(&st, vec![freeze.clone(), t2.clone()])),
        Some("XIP0001_SEALED")
    );
    let built = build(&st, vec![freeze.clone(), t2.clone()]);
    assert_eq!(included(&built), vec![freeze.hash()]);
    assert_eq!(
        rejected_reasons(&built),
        vec![(t2.hash(), "XIP0001_SEALED")]
    );
}

#[test]
fn b5_lock_lifted_earlier_in_the_block() {
    let st = locked_state(1);
    let g = signer(KEY2);
    let disable = set_word_call(&g, 0, 0);
    let xfer = a_transfer(0);
    assert_eq!(
        reason(strict(&st, vec![disable.clone(), xfer.clone()])),
        None
    );
    let built = build(&st, vec![disable.clone(), xfer.clone()]);
    assert_eq!(included(&built), vec![disable.hash(), xfer.hash()]);
    // Without the unlock in front, the transfer is invalid.
    assert_eq!(
        reason(strict(&st, vec![xfer])),
        Some("XIP0001_NOT_SELF_CALL")
    );
}

#[test]
fn own_effects_do_not_change_own_validity() {
    // A locked self-call that seals the account during its own execution was valid.
    let st = locked_state(1);
    let seal_self = set_word_call(&a(), 0, 2);
    let out = strict(&st, vec![seal_self]).unwrap();
    let mut after = st.clone();
    after.apply(&out.state_diff);
    assert_eq!(status_of(&after, a().address()), EoaLockStatus::Sealed);
}

// ---------------------------------------------------------------------------
// §10 activation
// ---------------------------------------------------------------------------

#[test]
fn f1_f2_activation_boundary() {
    let st = locked_state(1);
    let exec = EvmExecutor::new(cfg_xip(Some(100)));
    // Block 99 (parent 98): V4 valid. Block 100 (parent 99): V4 invalid.
    assert_eq!(
        reason(strict_with(&exec, &st, &parent_at(98), vec![v4(0)])),
        None
    );
    assert_eq!(
        reason(strict_with(&exec, &st, &parent_at(99), vec![v4(0)])),
        Some("XIP0001_NOT_SELF_CALL")
    );
    let built = exec
        .build(&parent_at(98), attrs(&parent_at(98)), vec![v4(0)], &st)
        .unwrap();
    assert_eq!(built.block.body.len(), 1);
    let built = exec
        .build(&parent_at(99), attrs(&parent_at(99)), vec![v4(0)], &st)
        .unwrap();
    assert!(built.block.body.is_empty());

    // F2: absent = never active, at any height, sealed or not.
    let exec = EvmExecutor::new(cfg_xip(None));
    for n in [0, 99, 1_000_000] {
        for w in [1, 2] {
            let st = locked_state(w);
            assert_eq!(
                reason(strict_with(&exec, &st, &parent_at(n), vec![v4(0)])),
                None
            );
        }
    }
}

// ---------------------------------------------------------------------------
// §10 authorizations (rule b) and EIP-7702 execution
// ---------------------------------------------------------------------------

fn auth(s: &PrivateKeySigner, chain_id: u64, address: Address, nonce: u64) -> SignedAuthorization {
    let auth = Authorization {
        chain_id: U256::from(chain_id),
        address,
        nonce,
    };
    let sig = s.sign_hash_sync(&auth.signature_hash()).unwrap();
    auth.into_signed(sig)
}

fn set_code_tx(
    s: &PrivateKeySigner,
    nonce: u64,
    to: Address,
    list: Vec<SignedAuthorization>,
) -> RecoveredTx {
    let tx = TxEip7702 {
        chain_id: cfg().chain_id,
        nonce,
        gas_limit: 300_000,
        max_fee_per_gas: TWO_GWEI,
        max_priority_fee_per_gas: u128::from(GWEI) / 10,
        to,
        value: U256::ZERO,
        access_list: Default::default(),
        authorization_list: list,
        input: Bytes::new(),
    };
    let sig = s.sign_hash_sync(&tx.signature_hash()).unwrap();
    RecoveredTx::recover(Signed::new_unhashed(tx, sig).into()).unwrap()
}

/// `R2`: a contract doing `BALANCE(A)` (cold 2600 / warm 100 gas), to detect warming.
const R2: Address = Address::repeat_byte(0x2e);

fn with_probe(mut st: InMemoryState) -> InMemoryState {
    let mut code = vec![0x73];
    code.extend_from_slice(a().address().as_slice());
    code.extend_from_slice(&[0x31, 0x50, 0x00]); // BALANCE POP STOP
    set_code(&mut st, R2, code.into(), 1, U256::ZERO);
    st
}

/// Run `tx` as the only tx of a block (builder + strict replay), return gas used.
fn run_one(st: &mut InMemoryState, t: RecoveredTx) -> u64 {
    let exec = EvmExecutor::new(cfg());
    let built = build_and_apply(&exec, st, &parent(), vec![t.clone()]);
    assert_eq!(included(&built), vec![t.hash()], "tx is valid and included");
    assert!(built.outcome.receipts[0].is_success());
    built.outcome.gas_used
}

#[test]
fn a1_to_a4_locked_authority_tuples_are_skipped() {
    let chain = cfg().chain_id;
    let relayer = signer(KEY1);
    let a_addr = a().address();
    // Reference: same tx shape with a tuple that fails EIP-7702 step 1 (wrong chain id).
    let mut reference = with_probe(locked_state(1));
    let gas_ref = run_one(
        &mut reference,
        set_code_tx(&relayer, 0, R2, vec![auth(&a(), chain + 1, X, 0)]),
    );

    let cases = [
        ("A1", 1, auth(&a(), chain, X, 0)),
        ("A2", 1, auth(&a(), chain, Address::ZERO, 0)),
        ("A3", 1, auth(&a(), 0, X, 0)),
        ("A4", 2, auth(&a(), chain, X, 0)),
        ("A1-impl", 1, auth(&a(), chain, ACCOUNT_IMPL, 0)),
    ];
    for (name, w, tuple) in cases {
        let mut st = with_probe(locked_state(w));
        let gas = run_one(&mut st, set_code_tx(&relayer, 0, R2, vec![tuple]));
        assert_eq!(
            code_of(&st, a_addr),
            Bytes::from_static(&DESIGNATOR),
            "{name}"
        );
        assert_eq!(nonce_of(&st, a_addr), 0, "{name}: nonce not bumped");
        // Same gas as a tuple that fails step 1: no refund, authority not warmed.
        assert_eq!(gas, gas_ref, "{name}: no refund, not warmed");
    }

    // A5 (unlocked): applied normally, nonce + 1, and the authority is warmed / refunded.
    let mut st = with_probe(locked_state(0));
    let gas = run_one(
        &mut st,
        set_code_tx(&relayer, 0, R2, vec![auth(&a(), chain, X, 0)]),
    );
    assert_eq!(code_of(&st, a_addr), delegation_to(X));
    assert_eq!(nonce_of(&st, a_addr), 1);
    assert!(gas < gas_ref, "applied tuple: warm authority + refund");
}

#[test]
fn v15_locked_sender_with_own_tuple() {
    // Type-4 tx from A to itself (value 0) carrying a tuple signed by A: tx valid,
    // tuple skipped, nonce incremented once (by the tx).
    let mut st = locked_state(1);
    let a_addr = a().address();
    let t = set_code_tx(&a(), 0, a_addr, vec![auth(&a(), 0, F, 1)]);
    run_one(&mut st, t);
    assert_eq!(code_of(&st, a_addr), Bytes::from_static(&DESIGNATOR));
    assert_eq!(nonce_of(&st, a_addr), 1);
    assert_eq!(status_of(&st, a_addr), EoaLockStatus::Locked);

    // The same tuple from an unlocked A re-delegates (nonce + 2).
    let mut st = locked_state(0);
    run_one(
        &mut st,
        set_code_tx(&a(), 0, a_addr, vec![auth(&a(), 0, F, 1)]),
    );
    assert_eq!(code_of(&st, a_addr), delegation_to(F));
    assert_eq!(nonce_of(&st, a_addr), 2);
}

#[test]
fn a6_only_the_locked_authority_is_skipped() {
    let chain = cfg().chain_id;
    let c = signer(KEY3);
    let mut st = locked_state(1);
    run_one(
        &mut st,
        set_code_tx(
            &signer(KEY1),
            0,
            F,
            vec![auth(&a(), chain, X, 0), auth(&c, chain, ACCOUNT_IMPL, 0)],
        ),
    );
    assert_eq!(code_of(&st, a().address()), Bytes::from_static(&DESIGNATOR));
    assert_eq!(code_of(&st, c.address()), Bytes::from_static(&DESIGNATOR));
    assert_eq!(nonce_of(&st, c.address()), 1);
    // C is now delegated with an empty slot: unlocked.
    assert_eq!(status_of(&st, c.address()), EoaLockStatus::Unlocked);
}

#[test]
fn a7_tuples_are_judged_against_s0() {
    let chain = cfg().chain_id;
    let d = signer(KEY3);
    let (x1, x2) = (Address::repeat_byte(0x61), Address::repeat_byte(0x62));
    let mut st = locked_state(0);
    set_code(
        &mut st,
        d.address(),
        delegation_to(x1),
        5,
        U256::from(ETHER),
    );
    st.set_storage(d.address(), LOCK_SLOT, U256::from(1u64)); // garbage of a previous delegate
    assert_eq!(status_of(&st, d.address()), EoaLockStatus::Unlocked);
    run_one(
        &mut st,
        set_code_tx(
            &signer(KEY1),
            0,
            F,
            vec![auth(&d, chain, ACCOUNT_IMPL, 5), auth(&d, chain, x2, 6)],
        ),
    );
    assert_eq!(code_of(&st, d.address()), delegation_to(x2));
    assert_eq!(nonce_of(&st, d.address()), 7);
    assert_eq!(status_of(&st, d.address()), EoaLockStatus::Unlocked);

    // After only the first tuple, D is LOCKED from the next transaction on.
    let mut st2 = locked_state(0);
    set_code(
        &mut st2,
        d.address(),
        delegation_to(x1),
        5,
        U256::from(ETHER),
    );
    st2.set_storage(d.address(), LOCK_SLOT, U256::from(1u64));
    run_one(
        &mut st2,
        set_code_tx(&signer(KEY1), 0, F, vec![auth(&d, chain, ACCOUNT_IMPL, 5)]),
    );
    assert_eq!(status_of(&st2, d.address()), EoaLockStatus::Locked);
}

#[test]
fn eip7702_receipt_and_relayed_delegation() {
    // Type-4 txs work end to end without XIP-0001 involvement.
    let exec = EvmExecutor::new(cfg());
    let mut st = funded_state();
    let c = signer(KEY3);
    st.insert_account(c.address(), AccountInfo::new(0, U256::ZERO));
    let t = set_code_tx(
        &signer(KEY1),
        0,
        F,
        vec![auth(&c, cfg().chain_id, ACCOUNT_IMPL, 0)],
    );
    let built = build_and_apply(&exec, &mut st, &parent(), vec![t]);
    assert_eq!(
        built.outcome.receipts[0].tx_type(),
        xdevs_primitives::TxType::Eip7702
    );
    assert_eq!(code_of(&st, c.address()), Bytes::from_static(&DESIGNATOR));
    assert_eq!(nonce_of(&st, c.address()), 1);
}

// ---------------------------------------------------------------------------
// §10 RPC-level rules on the executor (M3-M5)
// ---------------------------------------------------------------------------

#[test]
fn m3_m4_m5_estimate_and_call() {
    let exec = EvmExecutor::new(cfg());
    let header = parent();
    let a_addr = a().address();
    let req = |to: Address, value: u64| TransactionRequest {
        from: Some(a_addr),
        to: Some(TxKind::Call(to)),
        value: Some(U256::from(value)),
        ..Default::default()
    };
    let xip_reason = |r: Result<u64, ExecutionError>| match r {
        Err(ExecutionError::Xip0001 { hash: None, error }) => error.violation.reason(),
        other => panic!("expected an XIP-0001 error, got {other:?}"),
    };

    // M3
    let st = locked_state(1);
    assert_eq!(
        xip_reason(exec.estimate_gas(&st, &header, &req(F, 1), 0)),
        "XIP0001_NOT_SELF_CALL"
    );
    assert_eq!(
        xip_reason(exec.estimate_gas(&locked_state(2), &header, &req(a_addr, 0), 0)),
        "XIP0001_SEALED"
    );
    // M4: a valid shape gets a normal estimate (<= 2,000,000).
    let mut self_call = req(a_addr, 0);
    self_call.input = alloy_rpc_types_eth::TransactionInput::new(word(1));
    let est = exec.estimate_gas(&st, &header, &self_call, 0).unwrap();
    assert!(est > 21_000 && est <= 2_000_000, "{est}");
    // Fee fields are checked against c3 (2 x next base fee = 2 gwei) only if present.
    let mut pricey = self_call.clone();
    pricey.max_fee_per_gas = Some(3 * u128::from(GWEI));
    assert_eq!(
        xip_reason(exec.estimate_gas(&st, &header, &pricey, 0)),
        "XIP0001_FEE_CAP"
    );
    // M5: eth_call is never rejected, even when sealed.
    let sealed = locked_state(2);
    let out = exec.call(&sealed, &header, &req(F, 1), 0).unwrap();
    assert!(out.is_success());
    // Unlocked accounts are unaffected.
    assert_eq!(
        exec.estimate_gas(&locked_state(0), &header, &req(F, 1), 0)
            .unwrap(),
        21_000
    );
}

// ---------------------------------------------------------------------------
// P-256 (secp256r1) precompile at 0x100 (EIP-7951, Osaka)
// ---------------------------------------------------------------------------

/// `hash ‖ r ‖ s ‖ qx ‖ qy` for a deterministic (RFC 6979) P-256 signature.
pub(crate) fn p256_input(valid: bool) -> Bytes {
    use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
    let sk = SigningKey::from_bytes(&[0x11u8; 32].into()).unwrap();
    let hash = [0x42u8; 32];
    let sig: Signature = sk.sign_prehash(&hash).unwrap();
    let point = sk.verifying_key().to_encoded_point(false);
    let mut input = hash.to_vec();
    input.extend_from_slice(&sig.to_bytes());
    input.extend_from_slice(point.x().unwrap());
    input.extend_from_slice(point.y().unwrap());
    assert_eq!(input.len(), 160);
    if !valid {
        input[0] ^= 1; // signature over another hash
    }
    input.into()
}

#[test]
fn p256verify_precompile() {
    let p256 = xdevs_primitives::address!("0x0000000000000000000000000000000000000100");
    let exec = EvmExecutor::new(cfg());
    let st = funded_state();
    let header = parent();
    for valid in [true, false] {
        let input = p256_input(valid);
        let req = TransactionRequest {
            from: Some(signer(KEY0).address()),
            to: Some(TxKind::Call(p256)),
            input: alloy_rpc_types_eth::TransactionInput::new(input.clone()),
            ..Default::default()
        };
        let CallResult::Success {
            output, gas_used, ..
        } = exec.call(&st, &header, &req, 0).unwrap()
        else {
            panic!("precompile call must succeed");
        };
        if valid {
            assert_eq!(output, word(1), "valid signature -> 32-byte 1");
        } else {
            assert!(output.is_empty(), "invalid signature -> empty output");
        }
        // EIP-7951 cost: 6900 gas on top of the intrinsic cost (no EIP-7623 floor here).
        let zeros = input.iter().filter(|b| **b == 0).count() as u64;
        let intrinsic = 21_000 + 4 * zeros + 16 * (input.len() as u64 - zeros);
        assert_eq!(gas_used, intrinsic + 6_900, "valid = {valid}");
    }
}
