//! End-to-end tests of EIP-7702 set-code transactions, XIP-0001 (EOA lock) enforcement
//! and the P-256 precompile, driven only through JSON-RPC.
//!
//! The locked / sealed accounts are installed directly in the genesis alloc (code =
//! `0xef0100 ‖ 0x…1005`, `storage[LOCK_SLOT]` = 1 or 2): the node rule only looks at the
//! code and the slot, so the account implementation itself is not needed.

use serde_json::{json, Value};
use std::path::Path;
use xdevs_crypto::sign_authorization;
use xdevs_e2e::*;
use xdevs_genesis::{Genesis, DEV_ACCOUNTS};
use xdevs_primitives::{
    address,
    constants::{DEV_CHAIN_ID, GWEI, MIN_BASE_FEE},
    reexports::alloy_eips::eip7702::Authorization,
    xip0001::{ACCOUNT_IMPL, DESIGNATOR, LOCK_SLOT},
    Address, Bytes, Encodable2718, TxEip1559, TxEip7702, TxEnvelope, TxKind, U256,
};

/// Dev account #5: delegated to `0x…1005`, lock word 1 (LOCKED).
const LOCKED_IDX: usize = 5;
/// Dev account #6: delegated to `0x…1005`, lock word 2 (SEALED).
const SEALED_IDX: usize = 6;
/// Other address (recipient / "token").
const F: Address = Address::repeat_byte(0xf1);
/// Any contract address used as a delegation target.
const X: Address = Address::repeat_byte(0x58);
/// The base fee stays at the 0.1 gwei floor: the genesis starts there and blocks are
/// nearly empty. A locked sender may pay at most 2 x base fee.
const BASE: u128 = MIN_BASE_FEE as u128;
const LOCKED_MAX_FEE: u128 = 3 * BASE / 2;

/// Dev genesis (XIP-0001 active at 0) starting at the base-fee floor, with the locked
/// and sealed 7702 accounts.
fn write_genesis(path: &Path) {
    let mut g = Genesis::dev(DEV_CHAIN_ID);
    assert_eq!(g.config.xip0001_block, Some(0));
    g.config.base_fee.initial = MIN_BASE_FEE;
    for (idx, word) in [(LOCKED_IDX, 1u64), (SEALED_IDX, 2)] {
        let acc = g.alloc.get_mut(&DEV_ACCOUNTS[idx]).expect("dev account");
        acc.code = Some(Bytes::from_static(&DESIGNATOR));
        acc.storage.insert(LOCK_SLOT, U256::from(word));
    }
    std::fs::write(path, g.to_json_string()).expect("write genesis");
}

async fn start_locked_chain(dir: &Path) -> TestNode {
    let genesis = dir.join("genesis.json");
    write_genesis(&genesis);
    let mut config = test_config(&dir.join("data"), Address::repeat_byte(0xfe), true);
    config.chain.genesis = Some(genesis);
    TestNode::start(config).await.expect("node starts")
}

fn eip1559(
    signer: &xdevs_crypto::PrivateKeySigner,
    nonce: u64,
    to: Address,
    value: u64,
    gas_limit: u64,
    max_fee: u128,
) -> TxEnvelope {
    sign(
        signer,
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce,
            gas_limit,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(to),
            value: U256::from(value),
            input: Bytes::new(),
            access_list: Default::default(),
        },
    )
}

fn raw(tx: &TxEnvelope) -> Value {
    json!(Bytes::from(tx.encoded_2718()))
}

/// `eth_sendRawTransaction` that must fail with -32003; returns `data.reason`.
async fn rejected_reason(node: &TestNode, tx: &TxEnvelope) -> String {
    let (code, msg, data) = node.err_data("eth_sendRawTransaction", vec![raw(tx)]).await;
    assert_eq!(code, -32003, "{msg}");
    assert!(msg.starts_with("xip-0001: sender "), "{msg}");
    data["reason"].as_str().expect("data.reason").to_string()
}

async fn code_of(node: &TestNode, a: Address) -> Bytes {
    node.ok("eth_getCode", vec![json!(a), json!("latest")])
        .await
}

async fn nonce_of(node: &TestNode, a: Address) -> u64 {
    let n: U256 = node
        .ok("eth_getTransactionCount", vec![json!(a), json!("latest")])
        .await;
    n.to::<u64>()
}

/// M1 + V2 + c1 + M3 / M4: a locked account's transfer is rejected with -32003, its
/// value-0 self-calls are accepted and mined, one per block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_account_only_self_calls() {
    let dir = tempfile::tempdir().unwrap();
    let node = start_locked_chain(dir.path()).await;
    let (signer, a) = dev_signer(LOCKED_IDX);
    let nonce = nonce_of(&node, a).await;
    assert_eq!(code_of(&node, a).await, Bytes::from_static(&DESIGNATOR));

    let lock: Value = node
        .ok("xdevs_getEoaLock", vec![json!(a), json!("latest")])
        .await;
    assert_eq!(
        lock,
        json!({
            "active": true,
            "status": "locked",
            "word": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "delegatedTo": "0x0000000000000000000000000000000000001005",
        })
    );

    // M1 / V4: plain value transfer; V3 / V7b / V8b shapes.
    let transfer = eip1559(&signer, nonce, F, 1, 21_000, LOCKED_MAX_FEE);
    assert_eq!(
        rejected_reason(&node, &transfer).await,
        "XIP0001_NOT_SELF_CALL"
    );
    let with_value = eip1559(&signer, nonce, a, 1, 50_000, LOCKED_MAX_FEE);
    assert_eq!(
        rejected_reason(&node, &with_value).await,
        "XIP0001_VALUE_NOT_ZERO"
    );
    let heavy = eip1559(&signer, nonce, a, 0, 2_000_001, LOCKED_MAX_FEE);
    assert_eq!(rejected_reason(&node, &heavy).await, "XIP0001_GAS_LIMIT");
    let pricey = eip1559(&signer, nonce, a, 0, 50_000, 2 * BASE + 1);
    assert_eq!(rejected_reason(&node, &pricey).await, "XIP0001_FEE_CAP");
    assert_eq!(node.balance(F).await, U256::ZERO);

    // V2: value-0 self-calls are accepted and mined, at most one per block (c1).
    let first = eip1559(&signer, nonce, a, 0, 100_000, LOCKED_MAX_FEE);
    let second = eip1559(&signer, nonce + 1, a, 0, 100_000, LOCKED_MAX_FEE);
    let h1 = node.send(&first).await.expect("self-call accepted");
    let h2 = node.send(&second).await.expect("second self-call accepted");
    let r1 = node.wait_receipt(h1).await.unwrap();
    let r2 = node.wait_receipt(h2).await.unwrap();
    assert!(r1.status() && r2.status());
    assert_eq!((r1.from, r1.to), (a, Some(a)));
    assert!(
        r2.block_number.unwrap() > r1.block_number.unwrap(),
        "one tx per block for a locked sender"
    );
    assert_eq!(nonce_of(&node, a).await, nonce + 2);
    assert_eq!(code_of(&node, a).await, Bytes::from_static(&DESIGNATOR));

    // M3: estimateGas of a transfer is rejected; M4: a self-call is estimated normally.
    let (code, _, data) = node
        .err_data(
            "eth_estimateGas",
            vec![json!({ "from": a, "to": F, "value": "0x1" })],
        )
        .await;
    assert_eq!(code, -32003);
    assert_eq!(data["reason"], "XIP0001_NOT_SELF_CALL");
    let est: U256 = node
        .ok("eth_estimateGas", vec![json!({ "from": a, "to": a })])
        .await;
    assert_eq!(est, U256::from(21_000u64));
    node.shutdown().await;
}

/// V13 / M2 / M5: a sealed account cannot send anything, but eth_call still works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sealed_account_cannot_send_anything() {
    let dir = tempfile::tempdir().unwrap();
    let node = start_locked_chain(dir.path()).await;
    let (signer, s) = dev_signer(SEALED_IDX);
    let nonce = nonce_of(&node, s).await;

    let self_call = eip1559(&signer, nonce, s, 0, 100_000, LOCKED_MAX_FEE);
    assert_eq!(rejected_reason(&node, &self_call).await, "XIP0001_SEALED");
    let transfer = eip1559(&signer, nonce, F, 1, 21_000, 10 * BASE);
    assert_eq!(rejected_reason(&node, &transfer).await, "XIP0001_SEALED");
    let (code, _, data) = node
        .err_data("eth_estimateGas", vec![json!({ "from": s, "to": s })])
        .await;
    assert_eq!(
        (code, data["reason"].clone()),
        (-32003, json!("XIP0001_SEALED"))
    );

    // M5: simulations are never rejected.
    let out: Bytes = node
        .ok(
            "eth_call",
            vec![
                json!({ "from": s, "to": F, "value": "0x1" }),
                json!("latest"),
            ],
        )
        .await;
    assert!(out.is_empty());

    let lock: Value = node
        .ok("xdevs_getEoaLock", vec![json!(s), json!("latest")])
        .await;
    assert_eq!(lock["status"], "sealed");
    assert_eq!(lock["active"], true);
    // A plain EOA: unlocked, no delegation.
    let (_, plain) = dev_signer(0);
    let lock: Value = node.ok("xdevs_getEoaLock", vec![json!(plain)]).await;
    assert_eq!(lock["status"], "unlocked");
    assert_eq!(lock["delegatedTo"], Value::Null);

    // Nothing was ever included from the sealed account.
    node.wait_block(node.block_number().await + 2)
        .await
        .unwrap();
    assert_eq!(nonce_of(&node, s).await, nonce);
    assert_eq!(node.balance(F).await, U256::ZERO);
    node.shutdown().await;
}

/// A1 + A6 + EIP-7702 RPC shapes: a relayed type-4 tx carrying a tuple signed by a
/// locked authority is mined, but the authority's code and nonce are unchanged; the
/// tuple of an unlocked authority in the same list is applied.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_code_tx_skips_locked_authority() {
    let dir = tempfile::tempdir().unwrap();
    let node = start_locked_chain(dir.path()).await;
    let (relayer, from) = dev_signer(0);
    let (locked, a) = dev_signer(LOCKED_IDX);
    let (fresh, c) = dev_signer(7);
    let a_nonce = nonce_of(&node, a).await;

    let auth = |s, address, nonce| {
        sign_authorization(
            s,
            Authorization {
                chain_id: U256::from(DEV_CHAIN_ID),
                address,
                nonce,
            },
        )
        .unwrap()
    };
    let tx = sign(
        &relayer,
        TxEip7702 {
            chain_id: DEV_CHAIN_ID,
            nonce: node.pending_nonce(from).await,
            gas_limit: 200_000,
            max_fee_per_gas: u128::from(GWEI),
            max_priority_fee_per_gas: 1,
            to: F,
            value: U256::ZERO,
            access_list: Default::default(),
            authorization_list: vec![auth(&locked, X, a_nonce), auth(&fresh, ACCOUNT_IMPL, 0)],
            input: Bytes::new(),
        },
    );
    let hash = node.send(&tx).await.expect("type-4 tx accepted");
    let receipt = node.wait_receipt(hash).await.unwrap();
    assert!(receipt.status());

    // The locked authority's tuple was skipped.
    assert_eq!(code_of(&node, a).await, Bytes::from_static(&DESIGNATOR));
    assert_eq!(nonce_of(&node, a).await, a_nonce);
    // The unlocked one was applied.
    assert_eq!(code_of(&node, c).await, Bytes::from_static(&DESIGNATOR));
    assert_eq!(nonce_of(&node, c).await, 1);
    let lock: Value = node.ok("xdevs_getEoaLock", vec![json!(c)]).await;
    assert_eq!(lock["status"], "unlocked");
    assert_eq!(lock["delegatedTo"], json!(ACCOUNT_IMPL));

    // RPC shapes of a type-4 tx.
    let rpc_tx: Value = node.ok("eth_getTransactionByHash", vec![json!(hash)]).await;
    assert_eq!(rpc_tx["type"], "0x4");
    let list = rpc_tx["authorizationList"]
        .as_array()
        .expect("authorizationList");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["address"], json!(X));
    assert_eq!(list[1]["address"], json!(ACCOUNT_IMPL));
    let rpc_receipt: Value = node
        .ok("eth_getTransactionReceipt", vec![json!(hash)])
        .await;
    assert_eq!(rpc_receipt["type"], "0x4");
    let block: Value = node
        .ok(
            "eth_getBlockByNumber",
            vec![rpc_receipt["blockNumber"].clone(), json!(true)],
        )
        .await;
    assert_eq!(block["transactions"][0]["type"], "0x4");

    // An empty authorization list is refused by the pool.
    let empty = sign(
        &relayer,
        TxEip7702 {
            chain_id: DEV_CHAIN_ID,
            nonce: node.pending_nonce(from).await,
            gas_limit: 100_000,
            max_fee_per_gas: u128::from(GWEI),
            max_priority_fee_per_gas: 1,
            to: F,
            value: U256::ZERO,
            access_list: Default::default(),
            authorization_list: vec![],
            input: Bytes::new(),
        },
    );
    let err = node.send(&empty).await.expect_err("empty auth list");
    assert!(err.to_string().contains("empty auth list"), "{err}");
    node.shutdown().await;
}

/// P256VERIFY (EIP-7951) at `0x100` through `eth_call`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p256verify_precompile_via_eth_call() {
    use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
    let dir = tempfile::tempdir().unwrap();
    let node = TestNode::start(test_config(dir.path(), Address::repeat_byte(0xfe), true))
        .await
        .unwrap();
    let precompile = address!("0x0000000000000000000000000000000000000100");

    let sk = SigningKey::from_bytes(&[0x22u8; 32].into()).unwrap();
    let hash = [0x5au8; 32];
    let sig: Signature = sk.sign_prehash(&hash).unwrap();
    let point = sk.verifying_key().to_encoded_point(false);
    let mut input = hash.to_vec();
    input.extend_from_slice(&sig.to_bytes());
    input.extend_from_slice(point.x().unwrap());
    input.extend_from_slice(point.y().unwrap());

    let call = |data: Vec<u8>| {
        vec![
            json!({ "to": precompile, "data": Bytes::from(data) }),
            json!("latest"),
        ]
    };
    let out: Bytes = node.ok("eth_call", call(input.clone())).await;
    assert_eq!(out, Bytes::from(word(1).to_vec()), "valid signature");
    let mut bad = input.clone();
    bad[40] ^= 0xff; // corrupt r
    let out: Bytes = node.ok("eth_call", call(bad)).await;
    assert!(out.is_empty(), "invalid signature -> empty output");
    let out: Bytes = node.ok("eth_call", call(input[..159].to_vec())).await;
    assert!(out.is_empty(), "wrong input length -> empty output");
    node.shutdown().await;
}
