//! Simulator tests: the inspector's frame semantics and the asset / approval / vault
//! analysis, against hand-assembled contracts and the real WXDS predeploy bytecode.

use super::*;
use std::sync::{atomic::AtomicBool, Arc};
use xdevs_primitives::{
    constants::GWEI,
    hex, keccak256,
    safety::{
        abi::{address_word, u256_word},
        ApprovalKind, AssetStandard, AssetToken, VaultEventKind, TOPICS, WXDS,
    },
    xip0001::DESIGNATOR,
    AccountInfo, ChainConfig, Log, LogData, B256,
};
use xdevs_state::InMemoryState;

const ETHER: u128 = 1_000_000_000_000_000_000;
const ALICE: Address = Address::repeat_byte(0xa1);
const BOB: Address = Address::repeat_byte(0xb0);
const SPENDER: Address = Address::repeat_byte(0x5e);
const FORWARDER: Address = Address::repeat_byte(0xf0);
const REVERTER: Address = Address::repeat_byte(0xee);
const LOG2: Address = Address::repeat_byte(0x02);
const LOG3: Address = Address::repeat_byte(0x03);
const LOG4: Address = Address::repeat_byte(0x04);
const ACCOUNT: Address = Address::repeat_byte(0xac);

/// `CALL(gas, calldata[0], callvalue, 0, 0, 0, 0); STOP` (result ignored).
const FORWARDER_CODE: &[u8] = &[
    0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x34, 0x60, 0x00, 0x35, 0x5a, 0xf1, 0x50, 0x00,
];
/// `REVERT(0, 32)` with 42 in memory (not an `Error(string)`).
const REVERT_CODE: &[u8] = &[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd];
/// `LOG2(topic0 = cd[0], topic1 = cd[32], data = cd[64..])`.
const LOG2_CODE: &[u8] = &[
    0x36, 0x60, 0x40, 0x90, 0x03, 0x80, 0x60, 0x40, 0x60, 0x00, 0x37, 0x60, 0x20, 0x35, 0x90, 0x60,
    0x00, 0x35, 0x90, 0x60, 0x00, 0xa2, 0x00,
];
/// `LOG3(cd[0], cd[32], cd[64], data = cd[96..])`.
const LOG3_CODE: &[u8] = &[
    0x36, 0x60, 0x60, 0x90, 0x03, 0x80, 0x60, 0x60, 0x60, 0x00, 0x37, 0x60, 0x40, 0x35, 0x90, 0x60,
    0x20, 0x35, 0x90, 0x60, 0x00, 0x35, 0x90, 0x60, 0x00, 0xa3, 0x00,
];
/// `LOG4(cd[0], cd[32], cd[64], cd[96], data = cd[128..])`.
const LOG4_CODE: &[u8] = &[
    0x36, 0x60, 0x80, 0x90, 0x03, 0x80, 0x60, 0x80, 0x60, 0x00, 0x37, 0x60, 0x60, 0x35, 0x90, 0x60,
    0x40, 0x35, 0x90, 0x60, 0x20, 0x35, 0x90, 0x60, 0x00, 0x35, 0x90, 0x60, 0x00, 0xa4, 0x00,
];
/// Runtime `SSTORE(0, cd[0])`-style store contract (any small runtime will do).
const STORE_RUNTIME: &[u8] = &[0x60, 0x00, 0x35, 0x60, 0x00, 0x55, 0x00];

/// Runtime bytecode of a predeploy from the genesis artifacts.
fn predeploy_code(address: &str) -> Bytes {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/configs/predeploys-alloc.json"
    );
    let json = std::fs::read_to_string(path).expect("read predeploys-alloc.json");
    let alloc = json.find("\"alloc\"").expect("alloc");
    let entry = alloc
        + json[alloc..]
            .find(&format!("\"{address}\""))
            .expect("predeploy entry");
    let marker = "\"code\": \"";
    let start = entry + json[entry..].find(marker).expect("code") + marker.len();
    let end = start + json[start..].find('"').expect("end of code");
    hex::decode(json[start..end].trim_start_matches("0x"))
        .expect("hex code")
        .into()
}

fn initcode(runtime: &[u8]) -> Bytes {
    let n = u8::try_from(runtime.len()).unwrap();
    let mut code = vec![
        0x60, n, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, n, 0x60, 0x00, 0xf3,
    ];
    code.extend_from_slice(runtime);
    code.into()
}

fn with_code(st: &mut InMemoryState, address: Address, code: Bytes) {
    let code_hash = st.insert_code(code);
    st.insert_account(
        address,
        AccountInfo {
            nonce: 1,
            balance: U256::ZERO,
            code_hash,
        },
    );
}

fn state() -> InMemoryState {
    let mut st = InMemoryState::new();
    st.insert_account(ALICE, AccountInfo::new(0, U256::from(100 * ETHER)));
    with_code(&mut st, FORWARDER, Bytes::from_static(FORWARDER_CODE));
    with_code(&mut st, REVERTER, Bytes::from_static(REVERT_CODE));
    with_code(&mut st, LOG2, Bytes::from_static(LOG2_CODE));
    with_code(&mut st, LOG3, Bytes::from_static(LOG3_CODE));
    with_code(&mut st, LOG4, Bytes::from_static(LOG4_CODE));
    with_code(
        &mut st,
        WXDS,
        predeploy_code("0x0000000000000000000000000000000000001009"),
    );
    st
}

fn header() -> Header {
    Header {
        number: 5,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(GWEI),
        timestamp: 1_700_000_000,
        ..Default::default()
    }
}

fn request(to: Address, value: u128, input: Vec<u8>) -> TransactionRequest {
    TransactionRequest {
        from: Some(ALICE),
        to: Some(TxKind::Call(to)),
        value: Some(U256::from(value)),
        input: TransactionInput::new(input.into()),
        ..Default::default()
    }
}

fn words(ws: &[B256]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.0).collect()
}

fn run(st: &InMemoryState, req: &TransactionRequest) -> SimulationReport {
    let exec = EvmExecutor::new(ChainConfig::dev());
    let options = SimulationOptions {
        gas_cap: 50_000_000,
        native_symbol: "tXDS".into(),
    };
    simulate(&exec, st, &header(), req, &options, &CallLimits::default()).unwrap()
}

#[test]
fn native_moves_follow_frame_success() {
    let st = state();
    // Inner call to a reverting contract: its value move is dropped, the outer one stays.
    let r = run(
        &st,
        &request(FORWARDER, 5, words(&[address_word(REVERTER)])),
    );
    assert!(r.success);
    assert_eq!(r.asset_changes.len(), 1, "{:?}", r.asset_changes);
    let c = &r.asset_changes[0];
    assert_eq!(
        (c.token, c.from, c.to),
        (AssetToken::Native, ALICE, FORWARDER)
    );
    assert_eq!(c.raw_amount, U256::from(5));
    assert_eq!(c.symbol.as_deref(), Some("tXDS"));

    // Inner call to an EOA: both moves; the forwarder nets to zero.
    let r = run(&st, &request(FORWARDER, 7, words(&[address_word(BOB)])));
    let moves: Vec<_> = r.asset_changes.iter().map(|c| (c.from, c.to)).collect();
    assert_eq!(moves, vec![(ALICE, FORWARDER), (FORWARDER, BOB)]);
    let net: Vec<_> = r
        .balance_changes
        .iter()
        .map(|b| (b.address, b.raw_delta.as_str()))
        .collect();
    assert_eq!(net, vec![(ALICE, "-7"), (BOB, "7")]);
    assert!(r.disclaimer.starts_with("advice, not fact"));
}

#[test]
fn reverted_transaction_has_no_effects() {
    let st = state();
    let r = run(&st, &request(REVERTER, 9, Vec::new()));
    assert!(!r.success);
    assert_eq!(r.revert_reason.as_deref(), Some("execution reverted"));
    assert!(r.asset_changes.is_empty() && r.approvals.is_empty());
    assert_eq!(r.warnings[0].code, codes::REVERT);
    assert_eq!(r.warnings[0].severity, Severity::High);
}

#[test]
fn erc20_transfer_and_unlimited_approval_from_logs() {
    let st = state();
    let t = &*TOPICS;
    let r = run(
        &st,
        &request(
            LOG3,
            0,
            words(&[
                t.transfer,
                address_word(ALICE),
                address_word(BOB),
                u256_word(U256::from(1_000)),
            ]),
        ),
    );
    assert_eq!(r.asset_changes.len(), 1);
    let c = &r.asset_changes[0];
    assert_eq!(c.token, AssetToken::Contract(LOG3));
    assert_eq!(c.standard, AssetStandard::Erc20);
    // The logger has no `decimals()`: raw amount, unknown symbol.
    assert_eq!(
        (c.amount.as_str(), c.decimals, c.symbol.as_ref()),
        ("1000", None, None)
    );

    let r = run(
        &st,
        &request(
            LOG3,
            0,
            words(&[
                t.approval,
                address_word(ALICE),
                address_word(SPENDER),
                u256_word(U256::MAX),
            ]),
        ),
    );
    assert_eq!(r.approvals.len(), 1);
    let a = &r.approvals[0];
    assert_eq!(
        (a.kind, a.owner, a.spender),
        (ApprovalKind::Erc20, ALICE, SPENDER)
    );
    assert!(a.unlimited && a.amount == "unlimited" && a.expires.is_none());
    assert_eq!(r.warnings[0].code, codes::UNLIMITED_APPROVAL);
    assert_eq!(r.warnings[0].severity, Severity::High);
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["approvals"][0]["amount"], "unlimited");
    assert_eq!(json["warnings"][0]["severity"], "high");
}

#[test]
fn nft_transfer_and_operator_approval() {
    let st = state();
    let t = &*TOPICS;
    let r = run(
        &st,
        &request(
            LOG4,
            0,
            words(&[
                t.transfer,
                address_word(ALICE),
                address_word(BOB),
                u256_word(U256::from(77)),
            ]),
        ),
    );
    let c = &r.asset_changes[0];
    assert_eq!(c.standard, AssetStandard::Erc721);
    assert_eq!(c.token_id, Some(U256::from(77)));
    assert_eq!(c.amount, "1");

    let r = run(
        &st,
        &request(
            LOG3,
            0,
            words(&[
                t.approval_for_all,
                address_word(ALICE),
                address_word(SPENDER),
                u256_word(U256::from(1)),
            ]),
        ),
    );
    let a = &r.approvals[0];
    assert_eq!(a.kind, ApprovalKind::Erc721All);
    assert_eq!(a.amount, "all");
    assert_eq!(r.warnings[0].code, codes::SET_APPROVAL_FOR_ALL);
}

#[test]
fn wxds_deposit_and_approve_use_token_metadata() {
    let st = state();
    let r = run(&st, &request(WXDS, ETHER, hex::decode("d0e30db0").unwrap()));
    assert!(r.success, "{:?}", r.revert_reason);
    assert_eq!(r.asset_changes.len(), 2, "{:?}", r.asset_changes);
    let (native, wrapped) = (&r.asset_changes[0], &r.asset_changes[1]);
    assert_eq!(
        (native.token, native.amount.as_str()),
        (AssetToken::Native, "1")
    );
    assert_eq!(wrapped.token, AssetToken::Contract(WXDS));
    assert_eq!((wrapped.from, wrapped.to), (Address::ZERO, ALICE));
    assert_eq!(wrapped.symbol.as_deref(), Some("WXDS"));
    assert_eq!((wrapped.decimals, wrapped.amount.as_str()), (Some(18), "1"));

    let mut data = abi::selector("approve(address,uint256)").to_vec();
    data.extend(words(&[address_word(SPENDER), u256_word(U256::MAX)]));
    let r = run(&st, &request(WXDS, 0, data));
    let a = &r.approvals[0];
    assert_eq!(a.symbol.as_deref(), Some("WXDS"));
    assert!(a.unlimited);
    assert!(
        r.warnings[0].message.contains("ALL your WXDS"),
        "{}",
        r.warnings[0].message
    );
}

#[test]
fn contract_creation_is_reported() {
    let st = state();
    let req = TransactionRequest {
        from: Some(ALICE),
        to: Some(TxKind::Create),
        input: TransactionInput::new(initcode(STORE_RUNTIME)),
        ..Default::default()
    };
    let r = run(&st, &req);
    assert!(r.success, "{:?}", r.revert_reason);
    assert_eq!(r.contracts_created.len(), 1);
    assert_eq!(r.contracts_created[0].address, ALICE.create(0));
    assert_eq!(r.contracts_created[0].creator, ALICE);
    assert!(r
        .warnings
        .iter()
        .any(|w| w.code == codes::CONTRACT_CREATION));
}

#[test]
fn guardian_vault_queue_and_instant_paths() {
    let st = state();
    // The LOG2 contract stands for a smart account emitting `VaultActionQueued(7, ...)`.
    let execute_after = 1_700_086_400u64;
    let r = run(
        &st,
        &request(
            LOG2,
            0,
            words(&[
                TOPICS.vault_queued,
                u256_word(U256::from(7)),
                address_word(BOB),
                u256_word(U256::from(ETHER)),
                u256_word(U256::from(128)),
                u256_word(U256::from(execute_after)),
                u256_word(U256::ZERO),
            ]),
        ),
    );
    let v = r.vault.as_ref().expect("vault outcome");
    assert!(v.queued);
    assert_eq!((v.account, v.action_id), (LOG2, Some(U256::from(7))));
    assert_eq!(v.release_at, Some(execute_after));
    assert!(r.asset_changes.is_empty());
    let w = r
        .warnings
        .iter()
        .find(|w| w.code == codes::VAULT_QUEUED)
        .unwrap();
    assert!(w.message.contains("2023-11-15 22:13 UTC"), "{}", w.message);

    // An EIP-7702 account delegated to the implementation, executing instantly.
    let mut st = state();
    let code_hash = st.insert_code(Bytes::from_static(&DESIGNATOR));
    st.insert_account(
        ACCOUNT,
        AccountInfo {
            nonce: 1,
            balance: U256::ZERO,
            code_hash,
        },
    );
    let r = run(&st, &request(ACCOUNT, 0, Vec::new()));
    let v = r.vault.expect("smart account target");
    assert!(!v.queued && v.account == ACCOUNT && v.release_at.is_none());
}

#[test]
fn deterministic_and_never_mutates_state() {
    let st = state();
    let root = st.state_root();
    let req = request(FORWARDER, 3, words(&[address_word(BOB)]));
    assert_eq!(run(&st, &req), run(&st, &req));
    assert_eq!(st.state_root(), root);
}

#[test]
fn limits_abort_and_missing_from_is_rejected() {
    let st = state();
    let exec = EvmExecutor::new(ChainConfig::dev());
    let cancel = Arc::new(AtomicBool::new(true));
    let limits = CallLimits::default().with_cancel(cancel);
    let err = simulate(
        &exec,
        &st,
        &header(),
        &request(BOB, 1, Vec::new()),
        &SimulationOptions::default(),
        &limits,
    )
    .unwrap_err();
    assert!(matches!(err, ExecutionError::Aborted(_)), "{err:?}");

    let mut req = request(BOB, 1, Vec::new());
    req.from = None;
    let err = simulate(
        &exec,
        &st,
        &header(),
        &req,
        &SimulationOptions::default(),
        &CallLimits::default(),
    )
    .unwrap_err();
    assert!(matches!(err, ExecutionError::InvalidRequest(_)), "{err:?}");
}

#[test]
fn high_value_and_insufficient_funds_warnings() {
    let st = state();
    let r = run(&st, &request(BOB, 60 * ETHER, Vec::new()));
    assert!(r
        .warnings
        .iter()
        .any(|w| w.code == codes::HIGH_VALUE_TRANSFER));
    let mut req = request(BOB, 99 * ETHER, Vec::new());
    req.gas = Some(21_000);
    req.max_fee_per_gas = Some(u128::MAX / 2);
    let r = run(&st, &req);
    assert!(r.success, "fees are not simulated");
    assert!(r
        .warnings
        .iter()
        .any(|w| w.code == codes::INSUFFICIENT_FUNDS));
}

// ---------------------------------------------------------------------------
// Pure analysis
// ---------------------------------------------------------------------------

fn log(address: Address, topics: Vec<B256>, data: Vec<u8>) -> TraceEvent {
    TraceEvent::Log(Log {
        address,
        data: LogData::new_unchecked(topics, data.into()),
    })
}

fn ctx(from: Address) -> ReportContext<'static> {
    ReportContext {
        from,
        to: None,
        sender_balance: U256::from(ETHER),
        native_symbol: "tXDS",
        to_is_smart_account: false,
    }
}

#[test]
fn xdrc20_permit2_and_erc1155_decoding() {
    let t = &*TOPICS;
    let token = Address::repeat_byte(0x70);
    let nft = Address::repeat_byte(0x71);
    let events = vec![
        // XDRC-20: `Approval` then `ApprovalWithExpiry` for the same pair.
        log(
            token,
            vec![t.approval, address_word(ALICE), address_word(SPENDER)],
            words(&[u256_word(U256::from(2_500_000))]),
        ),
        log(
            token,
            vec![
                t.approval_with_expiry,
                address_word(ALICE),
                address_word(SPENDER),
            ],
            words(&[
                u256_word(U256::from(2_500_000)),
                u256_word(U256::from(1_800_000_000u64)),
            ]),
        ),
        // Permit2Expiring: unlimited, expiring.
        log(
            xdevs_primitives::safety::PERMIT2_EXPIRING,
            vec![
                t.permit2_approval,
                address_word(ALICE),
                address_word(token),
                address_word(SPENDER),
            ],
            words(&[
                u256_word(xdevs_primitives::safety::UNLIMITED_THRESHOLD),
                u256_word(U256::from(1_800_000_000u64)),
            ]),
        ),
        // ERC-1155 batch of two ids.
        log(
            nft,
            vec![
                t.transfer_batch,
                address_word(ALICE),
                address_word(ALICE),
                address_word(BOB),
            ],
            words(&[
                u256_word(U256::from(64)),
                u256_word(U256::from(160)),
                u256_word(U256::from(2)),
                u256_word(U256::from(1)),
                u256_word(U256::from(2)),
                u256_word(U256::from(2)),
                u256_word(U256::from(10)),
                u256_word(U256::from(20)),
            ]),
        ),
        // Operator approval on the ERC-1155 collection.
        log(
            nft,
            vec![
                t.approval_for_all,
                address_word(ALICE),
                address_word(SPENDER),
            ],
            words(&[u256_word(U256::from(1))]),
        ),
        // Ownership of some contract moves away from the sender.
        log(
            Address::repeat_byte(0x99),
            vec![
                t.ownership_transferred,
                address_word(ALICE),
                address_word(BOB),
            ],
            Vec::new(),
        ),
    ];
    let a = analyze(&events);
    assert_eq!(a.approvals.len(), 3, "{:?}", a.approvals);
    let mut infos = BTreeMap::new();
    infos.insert(
        token,
        TokenInfo {
            symbol: Some("USDX".into()),
            decimals: Some(6),
            erc1155: false,
        },
    );
    infos.insert(
        nft,
        TokenInfo {
            symbol: None,
            decimals: None,
            erc1155: true,
        },
    );
    let parts = report(&a, &ctx(ALICE), &infos);
    let x = &parts.approvals[0];
    assert_eq!(x.kind, ApprovalKind::Xdrc20);
    assert_eq!((x.amount.as_str(), x.expires), ("2.5", Some(1_800_000_000)));
    let p = &parts.approvals[1];
    assert_eq!((p.kind, p.token), (ApprovalKind::Permit2, token));
    assert!(p.unlimited && p.expires.is_some());
    assert_eq!(parts.approvals[2].kind, ApprovalKind::Erc1155All);
    // Unlimited but expiring: medium, not high.
    let w = parts
        .warnings
        .iter()
        .find(|w| w.code == codes::UNLIMITED_APPROVAL)
        .unwrap();
    assert_eq!(w.severity, Severity::Medium);
    assert!(parts
        .warnings
        .iter()
        .any(|w| w.code == codes::OWNERSHIP_TRANSFER && w.severity == Severity::High));
    let batch: Vec<_> = parts
        .asset_changes
        .iter()
        .map(|c| (c.standard, c.token_id, c.amount.clone()))
        .collect();
    assert_eq!(
        batch,
        vec![
            (
                AssetStandard::Erc1155,
                Some(U256::from(1)),
                "10".to_string()
            ),
            (
                AssetStandard::Erc1155,
                Some(U256::from(2)),
                "20".to_string()
            ),
        ]
    );
}

#[test]
fn vault_execute_and_cancel_ids() {
    let t = &*TOPICS;
    let events = vec![
        log(
            ACCOUNT,
            vec![t.vault_executed, u256_word(U256::from(3))],
            Vec::new(),
        ),
        log(
            ACCOUNT,
            vec![t.vault_cancelled, u256_word(U256::from(4))],
            words(&[address_word(ALICE)]),
        ),
    ];
    let a = analyze(&events);
    let mut c = ctx(ALICE);
    c.to = Some(ACCOUNT);
    c.to_is_smart_account = true;
    let v = report(&a, &c, &BTreeMap::new()).vault.unwrap();
    assert!(!v.queued);
    assert_eq!(v.executed, vec![U256::from(3)]);
    assert_eq!(v.cancelled, vec![U256::from(4)]);
    // Decoded as vault events too (same decoder as the subscription).
    let TraceEvent::Log(l) = &events[0] else {
        unreachable!()
    };
    assert_eq!(
        xdevs_primitives::safety::decode_vault_event(l)
            .unwrap()
            .kind,
        VaultEventKind::Executed
    );
}

#[test]
fn utc_formatting_and_revert_decoding() {
    assert_eq!(analyze::format_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(analyze::format_utc(1_700_000_000), "2023-11-14 22:13 UTC");
    assert_eq!(analyze::format_utc(951_782_400), "2000-02-29 00:00 UTC");
    // Error("nope")
    let mut data = vec![0x08, 0xc3, 0x79, 0xa0];
    data.extend(words(&[
        u256_word(U256::from(32)),
        u256_word(U256::from(4)),
    ]));
    let mut s = [0u8; 32];
    s[..4].copy_from_slice(b"nope");
    data.extend_from_slice(&s);
    assert_eq!(decode_revert(&data).as_deref(), Some("nope"));
    let mut panic = vec![0x4e, 0x48, 0x7b, 0x71];
    panic.extend(words(&[u256_word(U256::from(0x11))]));
    assert_eq!(decode_revert(&panic).as_deref(), Some("panic code 0x11"));
    assert_eq!(
        keccak256(DESIGNATOR),
        xdevs_primitives::xip0001::DESIGNATOR_HASH
    );
}
