//! Helpers for the end-to-end tests: start an in-process dev node on an ephemeral
//! port with a temporary data dir, talk to it over JSON-RPC, sign transactions with
//! the well-known dev keys, and a tiny hand-assembled contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use anyhow::{anyhow, bail, Context};
use jsonrpsee::{
    core::{client::ClientT, ClientError},
    http_client::{HttpClient, HttpClientBuilder},
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use xdevs_crypto::{sign_tx_envelope, signer_from_hex, PrivateKeySigner};
use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
use xdevs_node::{Node, NodeConfig, NodeHandle};
use xdevs_primitives::{hex, Address, Bytes, Encodable2718, TxEnvelope, B256, U256};

pub use alloy_rpc_types_eth::{Block as RpcBlock, Log as RpcLog, TransactionReceipt};

/// Block time used by the tests.
pub const TEST_BLOCK_TIME_MS: u64 = 200;

/// Topic emitted by [`counter_runtime`] when its value is set.
pub const VALUE_SET_TOPIC: B256 = B256::repeat_byte(0xaa);

/// Node configuration for tests: temp data dir, port 0, 200 ms blocks.
pub fn test_config(data_dir: &Path, fee_recipient: Address, produce: bool) -> NodeConfig {
    let mut cfg = NodeConfig::default();
    cfg.chain.data_dir = data_dir.to_path_buf();
    cfg.dev.enabled = produce;
    cfg.dev.block_time_ms = Some(TEST_BLOCK_TIME_MS);
    cfg.dev.fee_recipient = fee_recipient;
    cfg.rpc.http = "127.0.0.1:0".parse().expect("valid addr");
    cfg
}

/// A running in-process node plus an RPC client.
pub struct TestNode {
    /// Node handle (call [`TestNode::shutdown`] to stop).
    pub handle: NodeHandle,
    /// JSON-RPC client.
    pub client: HttpClient,
}

impl TestNode {
    /// Start a node with `config`.
    pub async fn start(config: NodeConfig) -> anyhow::Result<Self> {
        let handle = Node::new(config)?.start().await?;
        let client = HttpClientBuilder::default()
            .request_timeout(Duration::from_secs(30))
            .build(handle.http_url())?;
        Ok(Self { handle, client })
    }

    /// Stop the node and release the data dir.
    pub async fn shutdown(self) {
        drop(self.client);
        self.handle.shutdown().await;
    }

    /// Raw JSON-RPC call.
    pub async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value, ClientError> {
        self.client.request::<Value, _>(method, params).await
    }

    /// JSON-RPC call that must succeed, deserialized into `T`.
    pub async fn ok<T: DeserializeOwned>(&self, method: &str, params: Vec<Value>) -> T {
        let v = self
            .call(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method} failed: {e}"));
        serde_json::from_value(v.clone())
            .unwrap_or_else(|e| panic!("{method}: cannot deserialize {v}: {e}"))
    }

    /// JSON-RPC call that must fail; returns `(code, message)`.
    pub async fn err(&self, method: &str, params: Vec<Value>) -> (i32, String) {
        match self.call(method, params).await {
            Err(ClientError::Call(e)) => (e.code(), e.message().to_string()),
            other => panic!("{method}: expected a JSON-RPC error, got {other:?}"),
        }
    }

    /// `eth_blockNumber`.
    pub async fn block_number(&self) -> u64 {
        let n: alloy_rpc_types_eth::BlockNumberOrTag = self.ok("eth_blockNumber", vec![]).await;
        n.as_number().expect("number")
    }

    /// `eth_getBalance(addr, latest)`.
    pub async fn balance(&self, addr: Address) -> U256 {
        self.ok("eth_getBalance", vec![json!(addr), json!("latest")])
            .await
    }

    /// `eth_getTransactionCount(addr, pending)`.
    pub async fn pending_nonce(&self, addr: Address) -> u64 {
        let n: U256 = self
            .ok(
                "eth_getTransactionCount",
                vec![json!(addr), json!("pending")],
            )
            .await;
        n.to::<u64>()
    }

    /// `eth_getBlockByNumber(tag, full)`.
    pub async fn block(&self, tag: Value, full: bool) -> RpcBlock {
        self.ok("eth_getBlockByNumber", vec![tag, json!(full)])
            .await
    }

    /// `eth_sendRawTransaction` of a signed tx.
    pub async fn send(&self, tx: &TxEnvelope) -> Result<B256, ClientError> {
        let raw = Bytes::from(tx.encoded_2718());
        let v = self
            .call("eth_sendRawTransaction", vec![json!(raw)])
            .await?;
        Ok(serde_json::from_value(v).expect("tx hash"))
    }

    /// Poll `eth_getTransactionReceipt` until the tx is mined (10 s timeout).
    pub async fn wait_receipt(&self, hash: B256) -> anyhow::Result<TransactionReceipt> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let v = self
                .call("eth_getTransactionReceipt", vec![json!(hash)])
                .await
                .map_err(|e| anyhow!("eth_getTransactionReceipt: {e}"))?;
            if !v.is_null() {
                return serde_json::from_value(v.clone())
                    .with_context(|| format!("receipt does not deserialize: {v}"));
            }
            if Instant::now() > deadline {
                bail!("tx {hash} not mined within 10 s");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait until the head is at least `number` (10 s timeout).
    pub async fn wait_block(&self, number: u64) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.block_number().await < number {
            if Instant::now() > deadline {
                bail!("block {number} not produced within 10 s");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}

/// Signer and address of well-known dev account `i` (Anvil/Hardhat keys).
pub fn dev_signer(i: usize) -> (PrivateKeySigner, Address) {
    let signer = signer_from_hex(DEV_PRIVATE_KEYS[i]).expect("valid dev key");
    assert_eq!(signer.address(), DEV_ACCOUNTS[i]);
    (signer, DEV_ACCOUNTS[i])
}

/// Sign any transaction type into an envelope.
pub fn sign<T>(signer: &PrivateKeySigner, tx: T) -> TxEnvelope
where
    T: xdevs_primitives::reexports::alloy_consensus::SignableTransaction<
        xdevs_primitives::Signature,
    >,
    TxEnvelope: From<xdevs_primitives::Signed<T>>,
{
    sign_tx_envelope(signer, tx).expect("signing works")
}

/// Runtime of a tiny "counter" contract (hand-assembled, 65 bytes):
///
/// * calldata non-empty: `sstore(0, calldata[0..32])` and
///   `log1(value, topic = 0xaa..aa)`;
/// * always returns `sload(0)` as one 32-byte word.
pub fn counter_runtime() -> Bytes {
    let mut code = hex::decode(
        "36\
         15\
         6035\
         57\
         6000\
         35\
         80\
         6000\
         55\
         6000\
         52\
         7f",
    )
    .expect("hex");
    code.extend_from_slice(VALUE_SET_TOPIC.as_slice());
    code.extend(
        hex::decode(
            "6020\
             6000\
             a1\
             5b\
             6000\
             54\
             6000\
             52\
             6020\
             6000\
             f3",
        )
        .expect("hex"),
    );
    assert_eq!(code.len(), 0x41);
    assert_eq!(code[0x35], 0x5b, "JUMPDEST at 0x35");
    code.into()
}

/// Init code: `sstore(0, 42)`, then deploy [`counter_runtime`].
pub fn counter_init_code() -> Bytes {
    // PUSH1 42 PUSH1 0 SSTORE | PUSH1 0x41 DUP1 PUSH1 0x10 PUSH1 0 CODECOPY | PUSH1 0 RETURN
    let mut code = hex::decode("602a60005560418060106000396000f3").expect("hex");
    assert_eq!(code.len(), 0x10);
    code.extend_from_slice(&counter_runtime());
    code.into()
}

/// A 32-byte big-endian word.
pub fn word(v: u64) -> B256 {
    B256::from(U256::from(v).to_be_bytes::<32>())
}
