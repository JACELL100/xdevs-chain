//! `eth_subscribe` / `eth_unsubscribe` over WebSocket against a mock backend that
//! publishes through [`ChainEvents`]: shapes, filter semantics, limits, slow
//! subscribers and the HTTP refusal.

use jsonrpsee::{
    core::{
        client::{ClientT, Subscription, SubscriptionClientT, SubscriptionKind},
        ClientError,
    },
    http_client::HttpClientBuilder,
    rpc_params,
    ws_client::{WsClient, WsClientBuilder},
};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use xdevs_primitives::{
    address, set_compat_header_fields, Address, Block, Bytes, Header, LogData, Receipt,
    ReceiptEnvelope, ReceiptWithBloom, Signature, TxEip1559, TxEnvelope, TxKind, B256, U256,
};
use xdevs_rpc::{
    convert, types::*, BackendResult, BlockEvent, ChainEvents, RpcBackend, RpcBackendError,
    RpcServerConfig, RpcServerHandle,
};

// ---------------------------------------------------------------------------
// Mock backend: only the subscription hooks are real.
// ---------------------------------------------------------------------------

struct Mock {
    events: Option<Arc<ChainEvents>>,
}

fn unsupported<T>() -> BackendResult<T> {
    Err(RpcBackendError::Unsupported("mock".into()))
}

#[async_trait::async_trait]
impl RpcBackend for Mock {
    fn chain_id(&self) -> u64 {
        7117
    }
    async fn block_number(&self) -> BackendResult<u64> {
        Ok(0)
    }
    async fn balance(&self, _: Address, _: BlockId) -> BackendResult<U256> {
        unsupported()
    }
    async fn transaction_count(&self, _: Address, _: BlockId) -> BackendResult<u64> {
        unsupported()
    }
    async fn code(&self, _: Address, _: BlockId) -> BackendResult<Bytes> {
        unsupported()
    }
    async fn storage_at(&self, _: Address, _: B256, _: BlockId) -> BackendResult<B256> {
        unsupported()
    }
    async fn call(&self, _: TransactionRequest, _: BlockId) -> BackendResult<Bytes> {
        unsupported()
    }
    async fn estimate_gas(&self, _: TransactionRequest, _: BlockId) -> BackendResult<u64> {
        unsupported()
    }
    async fn send_raw_transaction(&self, _: Bytes) -> BackendResult<B256> {
        unsupported()
    }
    async fn block_by_number(
        &self,
        _: BlockNumberOrTag,
        _: bool,
    ) -> BackendResult<Option<RpcBlock>> {
        unsupported()
    }
    async fn block_by_hash(&self, _: B256, _: bool) -> BackendResult<Option<RpcBlock>> {
        unsupported()
    }
    async fn block_transaction_count(&self, _: BlockId) -> BackendResult<Option<u64>> {
        unsupported()
    }
    async fn transaction_by_hash(&self, _: B256) -> BackendResult<Option<RpcTransaction>> {
        unsupported()
    }
    async fn transaction_receipt(&self, _: B256) -> BackendResult<Option<RpcReceipt>> {
        unsupported()
    }
    async fn block_receipts(&self, _: BlockId) -> BackendResult<Option<Vec<RpcReceipt>>> {
        unsupported()
    }
    async fn logs(&self, _: Filter, _: Option<usize>) -> BackendResult<Vec<RpcLog>> {
        unsupported()
    }
    async fn gas_price(&self) -> BackendResult<U256> {
        unsupported()
    }
    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256> {
        unsupported()
    }
    async fn next_base_fee(&self) -> BackendResult<u64> {
        unsupported()
    }
    async fn fee_history(
        &self,
        _: u64,
        _: BlockNumberOrTag,
        _: Option<Vec<f64>>,
    ) -> BackendResult<FeeHistory> {
        unsupported()
    }
    fn subscribe_blocks(&self) -> Option<broadcast::Receiver<Arc<BlockEvent>>> {
        self.events.as_ref().map(|e| e.subscribe_blocks())
    }
    fn subscribe_pending_txs(&self) -> Option<broadcast::Receiver<B256>> {
        self.events.as_ref().map(|e| e.subscribe_pending_txs())
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const TOKEN: Address = address!("0x00000000000000000000000000000000000000cc");
const OTHER: Address = address!("0x00000000000000000000000000000000000000dd");
const TIMEOUT: Duration = Duration::from_secs(10);

fn topic(i: u8) -> B256 {
    B256::repeat_byte(i)
}

struct Server {
    events: Arc<ChainEvents>,
    handle: RpcServerHandle,
}

impl Server {
    async fn start(config: RpcServerConfig, events: ChainEvents) -> Self {
        Self::start_with(config, Some(Arc::new(events))).await
    }

    async fn start_with(mut config: RpcServerConfig, events: Option<Arc<ChainEvents>>) -> Self {
        config.http = SocketAddr::from(([127, 0, 0, 1], 0));
        let mock = Arc::new(Mock {
            events: events.clone(),
        });
        let handle = xdevs_rpc::start_server(config, mock).await.unwrap();
        Self {
            events: events.unwrap_or_default(),
            handle,
        }
    }

    async fn ws(&self) -> WsClient {
        WsClientBuilder::default()
            .request_timeout(TIMEOUT)
            .build(format!("ws://{}", self.handle.local_addr))
            .await
            .unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.stop();
    }
}

async fn sub(client: &WsClient, params: Vec<Value>) -> Result<Subscription<Value>, ClientError> {
    let mut p = jsonrpsee::core::params::ArrayParams::new();
    for v in params {
        p.insert(v).unwrap();
    }
    client
        .subscribe("eth_subscribe", p, "eth_unsubscribe")
        .await
}

async fn next(s: &mut Subscription<Value>) -> Value {
    tokio::time::timeout(TIMEOUT, s.next())
        .await
        .expect("notification in time")
        .expect("subscription open")
        .expect("valid JSON")
}

/// Assert nothing arrives for a short while.
async fn quiet(s: &mut Subscription<Value>) {
    if let Ok(v) = tokio::time::timeout(Duration::from_millis(300), s.next()).await {
        panic!("unexpected notification: {v:?}");
    }
}

fn call_error(r: Result<impl std::fmt::Debug, ClientError>) -> (i32, String) {
    match r {
        Err(ClientError::Call(e)) => (e.code(), e.message().to_string()),
        other => panic!("expected a call error, got {other:?}"),
    }
}

fn tx(nonce: u64) -> TxEnvelope {
    use xdevs_primitives::reexports::alloy_consensus::SignableTransaction;
    TxEip1559 {
        chain_id: 7117,
        nonce,
        gas_limit: 100_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(TOKEN),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    }
    .into_signed(Signature::test_signature())
    .into()
}

fn log(address: Address, topics: Vec<B256>, data: u8) -> xdevs_primitives::Log {
    xdevs_primitives::Log {
        address,
        data: LogData::new_unchecked(topics, Bytes::from(vec![data])),
    }
}

/// Block `number` with two txs: tx 0 emits (TOKEN,[1,2]) and (OTHER,[1]); tx 1 emits
/// (TOKEN,[3]).
fn block_event(number: u64) -> BlockEvent {
    let receipts = vec![
        ReceiptEnvelope::Eip1559(ReceiptWithBloom::from(Receipt {
            status: true.into(),
            cumulative_gas_used: 30_000,
            logs: vec![
                log(TOKEN, vec![topic(1), topic(2)], 0),
                log(OTHER, vec![topic(1)], 1),
            ],
        })),
        ReceiptEnvelope::Eip1559(ReceiptWithBloom::from(Receipt {
            status: true.into(),
            cumulative_gas_used: 60_000,
            logs: vec![log(TOKEN, vec![topic(3)], 2)],
        })),
    ];
    let mut header = Header {
        number,
        gas_limit: 30_000_000,
        gas_used: 60_000,
        timestamp: 1_700_000_000 + number,
        base_fee_per_gas: Some(1_000_000_000),
        ..Default::default()
    };
    set_compat_header_fields(&mut header);
    let mut block = Block::new(header, vec![tx(2 * number), tx(2 * number + 1)]);
    block.header.transactions_root = block.compute_transactions_root();
    let hash = block.hash();
    convert::block_event(&block, hash, &receipts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn new_heads_and_unsubscribe() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::default()).await;
    let client = s.ws().await;
    let mut heads = sub(&client, vec![json!("newHeads")]).await.unwrap();
    let SubscriptionKind::Subscription(id) = heads.kind().clone() else {
        panic!("subscription kind")
    };
    let id = serde_json::to_value(&id).unwrap();
    let id = id.as_str().unwrap();
    assert!(id.starts_with("0x") && id.len() == 34, "{id}");

    for n in 1..=3 {
        s.events.publish_block(block_event(n));
    }
    for n in 1..=3u64 {
        let ev = block_event(n);
        let json = next(&mut heads).await;
        // Same JSON as eth_getBlockByNumber(n, false) minus transactions/uncles/withdrawals.
        assert_eq!(json, serde_json::to_value(&ev.header).unwrap());
        assert_eq!(json["number"], format!("{n:#x}"));
        assert!(json["size"].is_string());
        assert!(json.get("transactions").is_none());
        let header: RpcHeader = serde_json::from_value(json).unwrap();
        assert_eq!(header.hash, ev.header.hash);
        assert_eq!(header.inner.number, n);
    }

    heads.unsubscribe().await.unwrap();
    // The server-side task ends: nothing listens any more.
    tokio::time::timeout(TIMEOUT, async {
        while s.events.has_block_subscribers() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("receiver dropped after eth_unsubscribe");

    // Unsubscribing an unknown id answers false.
    let gone: bool = client
        .request("eth_unsubscribe", rpc_params![id])
        .await
        .unwrap();
    assert!(!gone);
}

#[tokio::test]
async fn logs_follow_get_logs_filter_semantics() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::default()).await;
    let client = s.ws().await;
    let mut all = sub(&client, vec![json!("logs")]).await.unwrap();
    let mut empty_obj = sub(&client, vec![json!("logs"), json!({})]).await.unwrap();
    let mut token = sub(&client, vec![json!("logs"), json!({"address": TOKEN})])
        .await
        .unwrap();
    // OR over addresses, position 0 = topic 1, any topic 1.
    let mut either_t1 = sub(
        &client,
        vec![
            json!("logs"),
            json!({"address": [TOKEN, OTHER], "topics": [topic(1)]}),
        ],
    )
    .await
    .unwrap();
    // Wildcard at position 0, position 1 = topic 2.
    let mut second = sub(
        &client,
        vec![json!("logs"), json!({"topics": [null, topic(2)]})],
    )
    .await
    .unwrap();
    // OR within a position.
    let mut t1_or_t3 = sub(
        &client,
        vec![
            json!("logs"),
            json!({"address": TOKEN, "topics": [[topic(1), topic(3)]]}),
        ],
    )
    .await
    .unwrap();
    let mut none = sub(&client, vec![json!("logs"), json!({"topics": [topic(9)]})])
        .await
        .unwrap();

    let ev = block_event(5);
    s.events.publish_block(ev.clone());
    s.events.publish_block(block_event(6));

    // Delivered once per log, in block order, identical to eth_getLogs objects.
    for i in 0..3 {
        let json = next(&mut all).await;
        assert_eq!(json, serde_json::to_value(&ev.logs[i]).unwrap());
        let log: RpcLog = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(log.log_index, Some(i as u64));
        assert_eq!(log.block_hash, Some(ev.header.hash));
        assert_eq!(log.block_number, Some(5));
        assert!(!log.removed);
        assert_eq!(json["removed"], false);
        assert_eq!(json["transactionIndex"], if i < 2 { "0x0" } else { "0x1" });
        assert_eq!(next(&mut empty_obj).await, json);
    }
    // Block 6 follows block 5.
    assert_eq!(next(&mut all).await["blockNumber"], "0x6");

    let idx = |v: Value| v["logIndex"].as_str().unwrap().to_string();
    assert_eq!(idx(next(&mut token).await), "0x0");
    assert_eq!(idx(next(&mut token).await), "0x2");
    assert_eq!(idx(next(&mut either_t1).await), "0x0");
    assert_eq!(idx(next(&mut either_t1).await), "0x1");
    assert_eq!(idx(next(&mut second).await), "0x0");
    assert_eq!(idx(next(&mut t1_or_t3).await), "0x0");
    assert_eq!(idx(next(&mut t1_or_t3).await), "0x2");
    for s in [&mut token, &mut either_t1, &mut second, &mut t1_or_t3] {
        // The next notification is already block 6.
        assert_eq!(next(s).await["blockNumber"], "0x6");
    }
    quiet(&mut none).await;
}

#[tokio::test]
async fn pending_transactions() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::default()).await;
    let client = s.ws().await;
    let mut hashes = sub(&client, vec![json!("newPendingTransactions")])
        .await
        .unwrap();
    let mut hashes_false = sub(&client, vec![json!("newPendingTransactions"), json!(false)])
        .await
        .unwrap();
    let (code, msg) =
        call_error(sub(&client, vec![json!("newPendingTransactions"), json!(true)]).await);
    assert_eq!(code, -32602, "{msg}");

    s.events.publish_pending_tx(topic(0xaa));
    s.events.publish_pending_tx(topic(0xbb));
    for h in [topic(0xaa), topic(0xbb)] {
        assert_eq!(next(&mut hashes).await, json!(h));
        let got: B256 = serde_json::from_value(next(&mut hashes_false).await).unwrap();
        assert_eq!(got, h);
    }
}

#[tokio::test]
async fn invalid_params_and_unavailable_sources() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::default()).await;
    let client = s.ws().await;
    let (code, msg) = call_error(sub(&client, vec![json!("syncing")]).await);
    assert_eq!(code, -32602, "{msg}");
    let (code, _) =
        call_error(sub(&client, vec![json!("logs"), json!({"fromBlock": "0x0"})]).await);
    assert_eq!(code, -32602);
    let many: Vec<Address> = (0..300u64)
        .map(|i| Address::left_padding_from(&i.to_be_bytes()))
        .collect();
    let (code, msg) = call_error(sub(&client, vec![json!("logs"), json!({"address": many})]).await);
    assert_eq!(code, -32005);
    assert!(msg.contains("too many addresses"), "{msg}");

    // A backend without event sources (default trait methods).
    let bare = Server::start_with(RpcServerConfig::default(), None).await;
    let client = bare.ws().await;
    let (code, msg) = call_error(sub(&client, vec![json!("newHeads")]).await);
    assert_eq!(code, -32601);
    assert!(msg.contains("newHeads is not available"), "{msg}");
}

#[tokio::test]
async fn http_gets_a_clear_error() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::default()).await;
    let http = HttpClientBuilder::default()
        .build(format!("http://{}", s.handle.local_addr))
        .unwrap();
    for (method, params) in [
        ("eth_subscribe", rpc_params!["newHeads"]),
        ("eth_unsubscribe", rpc_params!["0x01"]),
    ] {
        let (code, msg) = call_error(http.request::<Value, _>(method, params).await);
        assert_eq!(code, -32601, "{method}");
        assert_eq!(msg, "notifications not supported", "{method}");
    }
    // Ordinary methods still work over HTTP.
    let id: String = http.request("eth_chainId", rpc_params![]).await.unwrap();
    assert_eq!(id, "0x1bcd");
    assert!(!s.events.has_block_subscribers());
}

#[tokio::test]
async fn per_connection_and_global_limits() {
    let config = RpcServerConfig {
        max_subscriptions_per_connection: 2,
        max_subscriptions: 3,
        ..Default::default()
    };
    let s = Server::start(config, ChainEvents::default()).await;
    let a = s.ws().await;
    let a1 = sub(&a, vec![json!("newHeads")]).await.unwrap();
    let _a2 = sub(&a, vec![json!("newPendingTransactions")])
        .await
        .unwrap();
    let (code, _) = call_error(sub(&a, vec![json!("newHeads")]).await);
    assert_eq!(code, -32006, "per-connection limit");

    let b = s.ws().await;
    let _b1 = sub(&b, vec![json!("logs")]).await.unwrap();
    let (code, msg) = call_error(sub(&b, vec![json!("newHeads")]).await);
    assert_eq!(code, -32005, "global limit: {msg}");
    assert!(msg.contains("too many subscriptions"), "{msg}");

    // Unsubscribing frees a global slot (the subscription task ends asynchronously).
    a1.unsubscribe().await.unwrap();
    let mut b2 = tokio::time::timeout(TIMEOUT, async {
        loop {
            match sub(&b, vec![json!("newHeads")]).await {
                Ok(s) => break s,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("slot freed");
    s.events.publish_block(block_event(1));
    assert_eq!(next(&mut b2).await["number"], "0x1");

    // Global cap reached again (a: 1, b: 2): a third connection is refused until the
    // connection holding a slot goes away.
    let c = s.ws().await;
    let (code, _) = call_error(sub(&c, vec![json!("newHeads")]).await);
    assert_eq!(code, -32005);
    drop(a);
    tokio::time::timeout(TIMEOUT, async {
        loop {
            match sub(&c, vec![json!("newPendingTransactions")]).await {
                Ok(_) => break,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("slot freed by disconnect");
}

/// A subscriber that falls more than the channel capacity behind is closed with an
/// error notification; others keep working. Single-threaded runtime: the server task
/// cannot run while the test publishes synchronously, so the lag is deterministic.
#[tokio::test(flavor = "current_thread")]
async fn lagging_subscriber_is_dropped() {
    let s = Server::start(RpcServerConfig::default(), ChainEvents::new(4, 4)).await;
    let client = s.ws().await;
    let mut slow = sub(&client, vec![json!("newHeads")]).await.unwrap();
    for n in 1..=10 {
        s.events.publish_block(block_event(n));
    }
    // The subscription is closed without delivering anything: the server saw
    // `Lagged` before the first event.
    let end = tokio::time::timeout(TIMEOUT, slow.next())
        .await
        .expect("closed in time");
    assert!(
        end.is_none(),
        "expected the subscription to end, got {end:?}"
    );
    // Its receiver is gone; a fresh subscription works.
    let mut fresh = sub(&client, vec![json!("newHeads")]).await.unwrap();
    s.events.publish_block(block_event(11));
    assert_eq!(next(&mut fresh).await["number"], "0xb");
}

/// Raw WebSocket frames: the notification envelope and the lag error notification as
/// a non-jsonrpsee client (viem, ethers) sees them.
#[tokio::test(flavor = "current_thread")]
async fn raw_notification_envelopes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let s = Server::start(RpcServerConfig::default(), ChainEvents::new(2, 2)).await;
    let mut stream = tokio::net::TcpStream::connect(s.handle.local_addr)
        .await
        .unwrap();
    let upgrade = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        s.handle.local_addr
    );
    stream.write_all(upgrade.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    while !buf.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        stream.read_exact(&mut b).await.unwrap();
        buf.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 101"));

    async fn send(stream: &mut tokio::net::TcpStream, text: &str) {
        // Masked text frame (client -> server), payload < 126 bytes.
        let payload = text.as_bytes();
        assert!(payload.len() < 126);
        let mut frame = vec![0x81, 0x80 | payload.len() as u8, 1, 2, 3, 4];
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ [1, 2, 3, 4][i % 4]),
        );
        stream.write_all(&frame).await.unwrap();
    }
    async fn recv(stream: &mut tokio::net::TcpStream) -> Value {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).await.unwrap();
        let len = match head[1] & 0x7f {
            126 => {
                let mut l = [0u8; 2];
                stream.read_exact(&mut l).await.unwrap();
                u16::from_be_bytes(l) as usize
            }
            127 => {
                let mut l = [0u8; 8];
                stream.read_exact(&mut l).await.unwrap();
                u64::from_be_bytes(l) as usize
            }
            n => n as usize,
        };
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await.unwrap();
        serde_json::from_slice(&payload).unwrap()
    }

    send(
        &mut stream,
        r#"{"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newPendingTransactions"]}"#,
    )
    .await;
    let resp = tokio::time::timeout(TIMEOUT, recv(&mut stream))
        .await
        .unwrap();
    assert_eq!(resp["id"], 1);
    let id = resp["result"].as_str().unwrap().to_string();
    assert!(id.starts_with("0x"));

    s.events.publish_pending_tx(topic(7));
    let note = tokio::time::timeout(TIMEOUT, recv(&mut stream))
        .await
        .unwrap();
    assert_eq!(
        note,
        json!({
            "jsonrpc": "2.0",
            "method": "eth_subscription",
            "params": {"subscription": id, "result": topic(7)}
        })
    );

    // Overflow the 2-slot channel: the subscription closes with an error notification.
    for i in 0..5 {
        s.events.publish_pending_tx(topic(i));
    }
    let note = tokio::time::timeout(TIMEOUT, recv(&mut stream))
        .await
        .unwrap();
    assert_eq!(note["method"], "eth_subscription");
    assert_eq!(note["params"]["subscription"], id);
    let err = note["params"]["error"].as_str().unwrap();
    assert!(err.contains("subscription too slow"), "{err}");
}
