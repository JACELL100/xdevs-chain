//! Phase-1 end-to-end tests (plan §27 exit criteria): an in-process dev node with
//! 200 ms blocks, driven only through JSON-RPC like MetaMask / Foundry would.

use serde_json::{json, Value};
use xdevs_e2e::*;
use xdevs_primitives::{
    constants::{DEV_CHAIN_ID, GWEI, TREASURY_ADDRESS},
    reexports::alloy_consensus::TxEip4844,
    Address, Bytes, ChainConfig, TxEip1559, TxKind, TxLegacy, B256, U256,
};

fn fresh_address(byte: u8) -> Address {
    Address::repeat_byte(byte)
}

async fn start(dir: &std::path::Path, fee_recipient: Address) -> TestNode {
    TestNode::start(test_config(dir, fee_recipient, true))
        .await
        .expect("node starts")
}

/// (a) EIP-1559 transfer: exact balances, 20% of the base fee to the treasury, 80%
/// burned, tip to the fee recipient.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eip1559_transfer_pays_fees_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let fee_recipient = fresh_address(0xfe);
    let node = start(dir.path(), fee_recipient).await;
    let (signer, from) = dev_signer(0);
    let to = fresh_address(0x77);

    let before = [
        node.balance(from).await,
        node.balance(to).await,
        node.balance(TREASURY_ADDRESS).await,
        node.balance(fee_recipient).await,
    ];
    assert_eq!(before[1], U256::ZERO);
    assert_eq!(before[3], U256::ZERO);

    let value = U256::from(1_500_000_000_000_000_000u128); // 1.5 tXDS
    let tip = u128::from(GWEI) / 2;
    let tx = sign(
        &signer,
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce: node.pending_nonce(from).await,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: tip,
            to: TxKind::Call(to),
            value,
            input: Bytes::new(),
            access_list: Default::default(),
        },
    );
    let hash = node.send(&tx).await.expect("accepted");
    assert_eq!(hash, *tx.tx_hash());
    // Pending view before inclusion (may already be mined: just must not error).
    let _: Value = node.ok("eth_getTransactionByHash", vec![json!(hash)]).await;

    let receipt = node.wait_receipt(hash).await.unwrap();
    assert!(receipt.status(), "receipt status 1");
    assert_eq!(receipt.gas_used, 21_000);
    assert_eq!(receipt.from, from);
    assert_eq!(receipt.to, Some(to));
    let block = node
        .block(
            json!(format!("0x{:x}", receipt.block_number.unwrap())),
            false,
        )
        .await;
    let base_fee = u128::from(block.header.base_fee_per_gas.unwrap());
    assert_eq!(receipt.effective_gas_price, base_fee + tip);
    assert_eq!(block.header.beneficiary, fee_recipient);

    let gas = U256::from(21_000u64);
    let base_total = gas * U256::from(base_fee);
    let (burned, treasury_share) = ChainConfig::dev().split_base_fee(base_total);
    assert_eq!(treasury_share * U256::from(4), burned, "80/20 split");
    let tip_total = gas * U256::from(tip);

    let after = [
        node.balance(from).await,
        node.balance(to).await,
        node.balance(TREASURY_ADDRESS).await,
        node.balance(fee_recipient).await,
    ];
    assert_eq!(after[0], before[0] - value - base_total - tip_total);
    assert_eq!(after[1], value);
    assert_eq!(after[2], before[2] + treasury_share);
    assert_eq!(after[3], tip_total);
    let sum = |b: &[U256; 4]| b.iter().fold(U256::ZERO, |a, x| a + *x);
    assert_eq!(
        sum(&before) - sum(&after),
        burned,
        "supply shrinks by the burn"
    );

    let mined: Value = node.ok("eth_getTransactionByHash", vec![json!(hash)]).await;
    assert_eq!(mined["blockHash"], json!(block.header.hash));

    // State pinned to past blocks (Foundry fork mode / `cast balance --block`), read
    // while new blocks keep coming.
    let mined_in = receipt.block_number.unwrap();
    node.wait_block(mined_in + 2).await.unwrap();
    let balance_at =
        |who: Address, tag: Value| node.ok::<U256>("eth_getBalance", vec![json!(who), tag]);
    assert_eq!(
        balance_at(to, json!(format!("0x{:x}", mined_in - 1))).await,
        U256::ZERO
    );
    assert_eq!(
        balance_at(to, json!(format!("0x{mined_in:x}"))).await,
        value
    );
    assert_eq!(
        balance_at(from, json!({ "blockHash": block.header.hash })).await,
        after[0]
    );
    assert_eq!(balance_at(from, json!("earliest")).await, before[0]);
    let nonce_at = |n: u64| {
        node.ok::<U256>(
            "eth_getTransactionCount",
            vec![json!(from), json!(format!("0x{n:x}"))],
        )
    };
    assert_eq!(
        nonce_at(mined_in - 1).await + U256::from(1),
        nonce_at(mined_in).await
    );
    node.shutdown().await;
}

/// (b) Deploy a contract, eth_call it, emit an event and find it with eth_getLogs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contract_deploy_call_and_logs() {
    let dir = tempfile::tempdir().unwrap();
    let node = start(dir.path(), Address::ZERO).await;
    let (signer, from) = dev_signer(1);
    let nonce = node.pending_nonce(from).await;

    let gas: U256 = node
        .ok(
            "eth_estimateGas",
            vec![json!({ "from": from, "data": counter_init_code() })],
        )
        .await;
    let deploy = sign(
        &signer,
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce,
            gas_limit: gas.to::<u64>(),
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to: TxKind::Create,
            value: U256::ZERO,
            input: counter_init_code(),
            access_list: Default::default(),
        },
    );
    let receipt = node
        .wait_receipt(node.send(&deploy).await.unwrap())
        .await
        .unwrap();
    assert!(receipt.status(), "deploy succeeded with the estimated gas");
    let contract = receipt.contract_address.expect("contractAddress");
    assert_eq!(contract, from.create(nonce));

    let code: Bytes = node
        .ok("eth_getCode", vec![json!(contract), json!("latest")])
        .await;
    assert_eq!(code, counter_runtime());
    let out: Bytes = node
        .ok("eth_call", vec![json!({ "to": contract }), json!("latest")])
        .await;
    assert_eq!(out.as_ref(), word(42).as_slice());

    // Set the value to 7: stores it and emits LOG1(topic 0xaa..).
    let input = Bytes::copy_from_slice(word(7).as_slice());
    let gas: U256 = node
        .ok(
            "eth_estimateGas",
            vec![json!({ "from": from, "to": contract, "data": input })],
        )
        .await;
    let set = sign(
        &signer,
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce: nonce + 1,
            gas_limit: gas.to::<u64>(),
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to: TxKind::Call(contract),
            value: U256::ZERO,
            input: input.clone(),
            access_list: Default::default(),
        },
    );
    let set_hash = node.send(&set).await.unwrap();
    let receipt = node.wait_receipt(set_hash).await.unwrap();
    assert!(receipt.status());
    let logs = receipt.inner.logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].address(), contract);
    assert_eq!(logs[0].topics(), &[VALUE_SET_TOPIC]);

    let found: Vec<RpcLog> = node
        .ok(
            "eth_getLogs",
            vec![json!({ "fromBlock": "0x0", "toBlock": "latest", "address": contract })],
        )
        .await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].transaction_hash, Some(set_hash));
    assert_eq!(found[0].block_number, receipt.block_number);
    assert_eq!(found[0].data().data, input);
    assert_eq!(found[0].topics(), &[VALUE_SET_TOPIC]);
    // By block hash and by topic.
    let by_hash: Vec<RpcLog> = node
        .ok(
            "eth_getLogs",
            vec![json!({ "blockHash": receipt.block_hash, "topics": [VALUE_SET_TOPIC] })],
        )
        .await;
    assert_eq!(by_hash.len(), 1);
    let none: Vec<RpcLog> = node
        .ok(
            "eth_getLogs",
            vec![json!({ "fromBlock": "0x0", "topics": [B256::repeat_byte(0xbb)] })],
        )
        .await;
    assert!(none.is_empty());

    let out: Bytes = node
        .ok("eth_call", vec![json!({ "to": contract }), json!("latest")])
        .await;
    assert_eq!(out.as_ref(), word(7).as_slice());
    let slot: B256 = node
        .ok(
            "eth_getStorageAt",
            vec![json!(contract), json!("0x0"), json!("latest")],
        )
        .await;
    assert_eq!(slot, word(7));
    let block_receipts: Vec<TransactionReceipt> = node
        .ok(
            "eth_getBlockReceipts",
            vec![json!(format!("0x{:x}", receipt.block_number.unwrap()))],
        )
        .await;
    assert!(block_receipts
        .iter()
        .any(|r| r.transaction_hash == set_hash));
    node.shutdown().await;
}

/// (c) Legacy EIP-155 tx works; unprotected legacy and blob txs are rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_and_rejected_tx_types() {
    let dir = tempfile::tempdir().unwrap();
    let node = start(dir.path(), Address::ZERO).await;
    let (signer, from) = dev_signer(2);
    let to = fresh_address(0x55);
    let nonce = node.pending_nonce(from).await;

    let legacy = |chain_id: Option<u64>, nonce: u64| {
        sign(
            &signer,
            TxLegacy {
                chain_id,
                nonce,
                gas_price: 2 * u128::from(GWEI),
                gas_limit: 21_000,
                to: TxKind::Call(to),
                value: U256::from(1234u64),
                input: Bytes::new(),
            },
        )
    };

    let hash = node.send(&legacy(Some(DEV_CHAIN_ID), nonce)).await.unwrap();
    let receipt = node.wait_receipt(hash).await.unwrap();
    assert!(receipt.status());
    assert_eq!(node.balance(to).await, U256::from(1234u64));
    let tx: Value = node.ok("eth_getTransactionByHash", vec![json!(hash)]).await;
    assert_eq!(tx["type"], json!("0x0"));
    assert_eq!(tx["chainId"], json!("0x1bcd"));

    // Unprotected (pre-EIP-155) legacy tx.
    let err = node
        .send(&legacy(None, nonce + 1))
        .await
        .expect_err("unprotected tx must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("replay-protected"), "{msg}");

    // Wrong chain id.
    let err = node
        .send(&legacy(Some(1), nonce + 1))
        .await
        .expect_err("wrong chain id must be rejected");
    assert!(err.to_string().contains("chain id"), "{err}");

    // Blob (type 3) tx.
    let blob = sign(
        &signer,
        TxEip4844 {
            chain_id: DEV_CHAIN_ID,
            nonce: nonce + 1,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to,
            value: U256::ZERO,
            access_list: Default::default(),
            blob_versioned_hashes: vec![{
                let mut h = B256::repeat_byte(0x11);
                h.0[0] = 0x01;
                h
            }],
            max_fee_per_blob_gas: 1,
            input: Bytes::new(),
        },
    );
    let (code, msg) = node
        .err(
            "eth_sendRawTransaction",
            vec![json!(Bytes::from(
                xdevs_primitives::Encodable2718::encoded_2718(&blob)
            ))],
        )
        .await;
    assert_eq!(code, -32000);
    assert!(msg.contains("type not supported"), "{msg}");

    // Nothing of the rejected txs reached the pool: the nonce is still nonce + 1.
    assert_eq!(node.pending_nonce(from).await, nonce + 1);
    node.shutdown().await;
}

/// (d) Restart: same head, same state root, balances intact, production continues.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_keeps_chain_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let (signer, from) = dev_signer(3);
    let to = fresh_address(0x33);

    let node = start(dir.path(), Address::ZERO).await;
    let tx = sign(
        &signer,
        TxEip1559 {
            chain_id: DEV_CHAIN_ID,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 10 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(GWEI),
            to: TxKind::Call(to),
            value: U256::from(10u64).pow(U256::from(18u64)),
            input: Bytes::new(),
            access_list: Default::default(),
        },
    );
    let hash = node.send(&tx).await.unwrap();
    node.wait_receipt(hash).await.unwrap();
    let genesis = node.block(json!("earliest"), false).await;
    node.shutdown().await;

    // Reopen without the producer: the head must be exactly where it was.
    let node = TestNode::start(test_config(dir.path(), Address::ZERO, false))
        .await
        .expect("reopen");
    let head = node.block(json!("latest"), false).await;
    let head_number = head.header.number;
    assert!(head_number >= 1);
    let from_balance = node.balance(from).await;
    let to_balance = node.balance(to).await;
    assert_eq!(to_balance, U256::from(10u64).pow(U256::from(18u64)));
    assert_eq!(node.pending_nonce(from).await, 1);
    let receipt = node.wait_receipt(hash).await.unwrap();
    assert!(receipt.status());
    assert_eq!(
        node.block(json!("earliest"), false).await.header.hash,
        genesis.header.hash
    );
    node.shutdown().await;

    // Reopen again with the producer: same head and root, then the chain grows on it.
    let node = start(dir.path(), Address::ZERO).await;
    let again = node.block(json!(format!("0x{head_number:x}")), false).await;
    assert_eq!(again.header.hash, head.header.hash);
    assert_eq!(again.header.state_root, head.header.state_root);
    node.wait_block(head_number + 2).await.unwrap();
    let next = node
        .block(json!(format!("0x{:x}", head_number + 1)), false)
        .await;
    assert_eq!(next.header.parent_hash, head.header.hash);
    // Empty blocks do not change the state.
    assert_eq!(next.header.state_root, head.header.state_root);
    assert_eq!(node.balance(from).await, from_balance);
    assert_eq!(node.balance(to).await, to_balance);
    node.shutdown().await;
}

/// (e) Chain id, block shape and the fee RPCs wallets use.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_id_and_block_shape() {
    let dir = tempfile::tempdir().unwrap();
    let node = start(dir.path(), Address::ZERO).await;
    let chain_id: String = node.ok("eth_chainId", vec![]).await;
    assert_eq!(chain_id, "0x1bcd");
    let net_version: String = node.ok("net_version", vec![]).await;
    assert_eq!(net_version, "7117");

    node.wait_block(2).await.unwrap();
    let latest = node.block(json!("latest"), true).await;
    assert!(latest.header.number >= 2);
    assert!(latest.header.base_fee_per_gas.is_some());
    assert_eq!(latest.header.extra_data.as_ref(), b"xdevs");
    let raw: Value = node
        .ok("eth_getBlockByNumber", vec![json!("latest"), json!(false)])
        .await;
    assert_eq!(raw["withdrawals"], json!([]));
    let by_hash: RpcBlock = node
        .ok(
            "eth_getBlockByHash",
            vec![json!(latest.header.hash), json!(false)],
        )
        .await;
    assert_eq!(by_hash.header.number, latest.header.number);
    let parent = node
        .block(json!(format!("0x{:x}", latest.header.number - 1)), false)
        .await;
    assert_eq!(latest.header.parent_hash, parent.header.hash);
    assert!(latest.header.timestamp > parent.header.timestamp);
    let missing: Value = node
        .ok(
            "eth_getBlockByNumber",
            vec![json!("0xffffff"), json!(false)],
        )
        .await;
    assert!(missing.is_null());

    let gas_price: U256 = node.ok("eth_gasPrice", vec![]).await;
    let tip: U256 = node.ok("eth_maxPriorityFeePerGas", vec![]).await;
    assert!(gas_price > tip && tip > U256::ZERO);
    let history: alloy_rpc_types_eth::FeeHistory = node
        .ok(
            "eth_feeHistory",
            vec![json!("0x2"), json!("latest"), json!([25.0, 75.0])],
        )
        .await;
    assert_eq!(history.base_fee_per_gas.len(), 3);
    assert_eq!(history.gas_used_ratio.len(), 2);
    assert_eq!(history.reward.as_ref().map(Vec::len), Some(2));

    // Recent past state is kept (default window 10,000 blocks); unknown blocks fail.
    let (_, dev) = dev_signer(2);
    let at_genesis: U256 = node
        .ok("eth_getBalance", vec![json!(dev), json!("0x0")])
        .await;
    assert_eq!(at_genesis, node.balance(dev).await);
    let (code, msg) = node
        .err("eth_getBalance", vec![json!(dev), json!("0xffffff")])
        .await;
    assert_eq!(code, -32000);
    assert!(msg.contains("header not found"), "{msg}");
    node.shutdown().await;
}

/// (e2) With a small state-history window, blocks that fell out of it fail cleanly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pruned_state_history_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path(), Address::ZERO, true);
    config.chain.state_history_blocks = Some(2);
    let node = TestNode::start(config).await.expect("node starts");
    node.wait_block(5).await.unwrap();
    let head = node.block_number().await;
    let ok: U256 = node
        .ok(
            "eth_getBalance",
            vec![json!(Address::ZERO), json!(format!("0x{:x}", head - 1))],
        )
        .await;
    assert_eq!(ok, U256::ZERO);
    let (code, msg) = node
        .err("eth_getBalance", vec![json!(Address::ZERO), json!("0x0")])
        .await;
    assert_eq!(code, -32000);
    assert!(msg.contains("historical state not available"), "{msg}");
    node.shutdown().await;
}
