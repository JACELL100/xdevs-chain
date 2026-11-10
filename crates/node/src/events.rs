//! Chain events for `eth_subscribe` (plan §19.1): the node owns one
//! [`ChainEvents`] broadcaster, publishes every committed block into it (dev producer
//! and consensus commit, under their commit locks, so in commit order) and every
//! transaction hash admitted to the mempool (RPC and tx gossip). The RPC backend hands
//! out receivers. Publishing is a no-op without subscribers.

use std::sync::Arc;
use xdevs_mempool::{Pool, TxPool};
use xdevs_network::{TxValidator, Verdict};
use xdevs_primitives::{keccak256, Block, ReceiptEnvelope, B256};
use xdevs_rpc::{convert, ChainEvents};

/// Publish committed block `block` (hash `hash`, `receipts[i]` of tx `i`).
pub(crate) fn publish_block(
    events: Option<&ChainEvents>,
    block: &Block,
    hash: B256,
    receipts: &[ReceiptEnvelope],
) {
    if let Some(events) = events.filter(|e| e.has_block_subscribers()) {
        events.publish_block(convert::block_event(block, hash, receipts));
    }
}

/// Wrap the tx-gossip validator so gossiped transactions newly admitted to the pool are
/// published as pending (already-known ones are not published again).
pub(crate) fn publishing_gossip_validator(
    inner: TxValidator,
    pool: Arc<Pool>,
    events: Arc<ChainEvents>,
) -> TxValidator {
    Arc::new(move |raw: &[u8]| {
        if !events.has_pending_tx_subscribers() {
            return inner(raw);
        }
        // The gossip payload is the EIP-2718 encoding: its hash is the tx hash.
        let hash = keccak256(raw);
        let known = pool.contains(&hash);
        let verdict = inner(raw);
        if verdict == Verdict::Accept && !known && pool.contains(&hash) {
            events.publish_pending_tx(hash);
        }
        verdict
    })
}

#[cfg(test)]
mod tests {
    //! A dev-mode node with a WebSocket client: `newHeads`, `logs` and
    //! `newPendingTransactions` end to end, notifications decoded into the alloy types.

    use crate::{Node, NodeConfig};
    use jsonrpsee::{
        core::client::{ClientT, Subscription, SubscriptionClientT},
        rpc_params,
        ws_client::WsClientBuilder,
    };
    use serde_json::Value;
    use std::time::Duration;
    use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
    use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
    use xdevs_primitives::{
        constants::{DEV_CHAIN_ID, GWEI},
        hex, Bytes, Encodable2718, TxEip1559, TxKind, B256, U256,
    };
    use xdevs_rpc::types::{RpcBlock, RpcHeader, RpcLog};

    const TIMEOUT: Duration = Duration::from_secs(20);

    async fn next(s: &mut Subscription<Value>) -> Value {
        tokio::time::timeout(TIMEOUT, s.next())
            .await
            .expect("notification in time")
            .expect("subscription open")
            .expect("valid JSON")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dev_node_subscriptions() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = NodeConfig::default();
        config.chain.data_dir = dir.path().join("data");
        config.rpc.http = "127.0.0.1:0".parse().unwrap();
        config.dev.block_time_ms = Some(200);
        let handle = Node::new(config).unwrap().start().await.unwrap();

        let ws = WsClientBuilder::default()
            .request_timeout(TIMEOUT)
            .build(format!("ws://{}", handle.http_addr()))
            .await
            .unwrap();
        let mut heads: Subscription<Value> = ws
            .subscribe("eth_subscribe", rpc_params!["newHeads"], "eth_unsubscribe")
            .await
            .unwrap();
        let mut pending: Subscription<Value> = ws
            .subscribe(
                "eth_subscribe",
                rpc_params!["newPendingTransactions"],
                "eth_unsubscribe",
            )
            .await
            .unwrap();

        // A contract creation whose init code emits LOG1(topic) and returns nothing:
        // PUSH32 topic, PUSH1 0, PUSH1 0, LOG1, STOP.
        let topic = B256::repeat_byte(0x5e);
        let mut init = vec![0x7f];
        init.extend_from_slice(topic.as_slice());
        init.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xa1, 0x00]);
        let created = DEV_ACCOUNTS[0].create(0);
        let mut logs: Subscription<Value> = ws
            .subscribe(
                "eth_subscribe",
                rpc_params![
                    "logs",
                    serde_json::json!({"address": created, "topics": [topic]})
                ],
                "eth_unsubscribe",
            )
            .await
            .unwrap();

        // newHeads: consecutive blocks, each identical to eth_getBlockByNumber minus txs.
        let first: RpcHeader = serde_json::from_value(next(&mut heads).await).unwrap();
        let second_json = next(&mut heads).await;
        let second: RpcHeader = serde_json::from_value(second_json.clone()).unwrap();
        assert_eq!(second.inner.number, first.inner.number + 1);
        assert_eq!(second.inner.parent_hash, first.hash);
        let block: Value = ws
            .request(
                "eth_getBlockByNumber",
                rpc_params![format!("{:#x}", second.inner.number), false],
            )
            .await
            .unwrap();
        let mut expected = block.as_object().unwrap().clone();
        for k in ["transactions", "uncles", "withdrawals"] {
            expected.remove(k);
        }
        assert_eq!(second_json, Value::Object(expected));
        let _: RpcBlock = serde_json::from_value(block).unwrap();

        // Send the tx: pending hash first, then the log once it is mined.
        let signer = signer_from_hex(DEV_PRIVATE_KEYS[0]).unwrap();
        let tx = sign_tx_envelope(
            &signer,
            TxEip1559 {
                chain_id: DEV_CHAIN_ID,
                nonce: 0,
                gas_limit: 100_000,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: u128::from(GWEI),
                to: TxKind::Create,
                value: U256::ZERO,
                input: Bytes::from(init),
                access_list: Default::default(),
            },
        )
        .unwrap();
        let raw = hex::encode_prefixed(tx.encoded_2718());
        let hash: B256 = ws
            .request("eth_sendRawTransaction", rpc_params![raw])
            .await
            .unwrap();
        let announced: B256 = serde_json::from_value(next(&mut pending).await).unwrap();
        assert_eq!(announced, hash);

        let log_json = next(&mut logs).await;
        let log: RpcLog = serde_json::from_value(log_json.clone()).unwrap();
        assert_eq!(log.inner.address, created);
        assert_eq!(log.inner.data.topics(), &[topic]);
        assert_eq!(log.transaction_hash, Some(hash));
        assert!(!log.removed);
        // Identical to the eth_getLogs object for the same block.
        let number = log.block_number.unwrap();
        let got: Vec<Value> = ws
            .request(
                "eth_getLogs",
                rpc_params![serde_json::json!({
                    "fromBlock": format!("{number:#x}"),
                    "toBlock": format!("{number:#x}"),
                    "address": created,
                })],
            )
            .await
            .unwrap();
        assert_eq!(got, vec![log_json]);

        // The heads stream keeps going in order through the tx's block.
        let mut last = second.inner.number;
        while last < number {
            let h: RpcHeader = serde_json::from_value(next(&mut heads).await).unwrap();
            assert_eq!(h.inner.number, last + 1);
            last = h.inner.number;
        }

        drop((heads, pending, logs, ws));
        handle.shutdown().await;
    }
}
