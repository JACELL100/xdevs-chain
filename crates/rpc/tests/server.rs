//! End-to-end tests: start the JSON-RPC server over a mock [`RpcBackend`] and talk to
//! it with the jsonrpsee HTTP client (and raw HTTP for transport-level checks).

use alloy_rpc_types_eth::{BlockTransactions, FilterBlockOption};
use jsonrpsee::{
    core::{client::ClientT, ClientError},
    http_client::{HttpClient, HttpClientBuilder},
};
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use xdevs_primitives::{
    address, hex, keccak256, set_compat_header_fields, Address, Block, Bytes, Decodable2718,
    Header, LogData, Receipt, ReceiptEnvelope, ReceiptWithBloom, RecoveredTx, TxEnvelope, B256,
    U256,
};
use xdevs_rpc::{
    convert::{self, ReceiptMeta, TxMeta},
    types::*,
    BackendResult, RpcBackend, RpcBackendError, RpcServerConfig, RpcServerHandle,
};

const HEAD: u64 = 100;
const EIP155_RAW: &str = "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

/// Deterministic fake chain with a single 1-tx block shape at every height `<= HEAD`.
struct Mock {
    tx: RecoveredTx,
    last_block: Mutex<Option<BlockId>>,
    last_request: Mutex<Option<TransactionRequest>>,
    last_filter: Mutex<Option<Filter>>,
    logs_calls: AtomicUsize,
    logs_to_return: AtomicUsize,
    /// `limit` argument of the last `logs` call.
    last_logs_limit: Mutex<Option<Option<usize>>>,
}

impl Mock {
    fn new() -> Self {
        let raw = hex::decode(EIP155_RAW).unwrap();
        let tx =
            RecoveredTx::recover(TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap()).unwrap();
        Self {
            tx,
            last_block: Mutex::new(None),
            last_request: Mutex::new(None),
            last_filter: Mutex::new(None),
            logs_calls: AtomicUsize::new(0),
            logs_to_return: AtomicUsize::new(1),
            last_logs_limit: Mutex::new(None),
        }
    }

    fn block(&self, number: u64) -> Block {
        let mut header = Header {
            number,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000 + number,
            base_fee_per_gas: Some(1_000_000_000),
            ..Default::default()
        };
        set_compat_header_fields(&mut header);
        let mut block = Block::new(header, vec![self.tx.tx.clone()]);
        block.header.transactions_root = block.compute_transactions_root();
        block
    }

    fn meta(&self, number: u64) -> TxMeta {
        let block = self.block(number);
        TxMeta {
            block_hash: block.hash(),
            block_number: number,
            index: 0,
            base_fee: block.header.base_fee_per_gas,
            timestamp: block.header.timestamp,
        }
    }

    fn receipt(&self) -> ReceiptEnvelope {
        ReceiptEnvelope::Legacy(ReceiptWithBloom::from(Receipt {
            status: true.into(),
            cumulative_gas_used: 21_000,
            logs: vec![xdevs_primitives::Log {
                address: address!("0x00000000000000000000000000000000000000cc"),
                data: LogData::new_unchecked(vec![B256::repeat_byte(1)], Bytes::from_static(&[7])),
            }],
        }))
    }

    fn rpc_receipt(&self, number: u64) -> RpcReceipt {
        convert::rpc_receipt(
            &self.tx,
            &self.receipt(),
            self.meta(number),
            ReceiptMeta {
                gas_used: 21_000,
                first_log_index: 0,
                contract_address: None,
            },
        )
    }

    fn resolve(&self, tag: BlockNumberOrTag) -> Option<u64> {
        match tag {
            BlockNumberOrTag::Number(n) if n <= HEAD => Some(n),
            BlockNumberOrTag::Number(_) => None,
            BlockNumberOrTag::Latest => Some(HEAD),
            // The server normalises every other tag before calling the backend.
            other => panic!("backend received unnormalised tag {other:?}"),
        }
    }

    fn record(&self, block: BlockId) -> BackendResult<()> {
        *self.last_block.lock().unwrap() = Some(block);
        match block {
            BlockId::Number(BlockNumberOrTag::Number(n)) if n > HEAD => {
                Err(RpcBackendError::UnknownBlock)
            }
            _ => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl RpcBackend for Mock {
    fn chain_id(&self) -> u64 {
        7117
    }
    async fn block_number(&self) -> BackendResult<u64> {
        Ok(HEAD)
    }
    async fn balance(&self, _: Address, block: BlockId) -> BackendResult<U256> {
        self.record(block)?;
        Ok(U256::from(1_000_000_000_000_000_000u128))
    }
    async fn transaction_count(&self, _: Address, block: BlockId) -> BackendResult<u64> {
        self.record(block)?;
        Ok(if block == BlockId::Number(BlockNumberOrTag::Pending) {
            6
        } else {
            5
        })
    }
    async fn code(&self, _: Address, block: BlockId) -> BackendResult<Bytes> {
        self.record(block)?;
        Ok(Bytes::from_static(&[0x60, 0x01]))
    }
    async fn storage_at(&self, _: Address, slot: B256, block: BlockId) -> BackendResult<B256> {
        self.record(block)?;
        Ok(slot) // echo: lets tests check slot parsing
    }
    async fn call(&self, request: TransactionRequest, block: BlockId) -> BackendResult<Bytes> {
        self.record(block)?;
        let input = request.input.input().cloned().unwrap_or_default();
        *self.last_request.lock().unwrap() = Some(request);
        if input.as_ref() == [0xde, 0xad] {
            // Error(string) "nope"
            let mut data = hex::decode("08c379a0").unwrap();
            data.extend_from_slice(&U256::from(32).to_be_bytes::<32>());
            data.extend_from_slice(&U256::from(4).to_be_bytes::<32>());
            let mut reason = [0u8; 32];
            reason[..4].copy_from_slice(b"nope");
            data.extend_from_slice(&reason);
            return Err(RpcBackendError::Reverted {
                data: data.into(),
                reason: Some("nope".into()),
            });
        }
        Ok(Bytes::from_static(&[0x01]))
    }
    async fn estimate_gas(
        &self,
        request: TransactionRequest,
        block: BlockId,
    ) -> BackendResult<u64> {
        self.record(block)?;
        *self.last_request.lock().unwrap() = Some(request);
        Ok(21_000)
    }
    async fn send_raw_transaction(&self, raw: Bytes) -> BackendResult<B256> {
        if raw[0] == 0xff {
            return Err(RpcBackendError::Rejected("nonce too low".into()));
        }
        Ok(keccak256(&raw))
    }
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> BackendResult<Option<RpcBlock>> {
        Ok(self.resolve(number).map(|n| {
            let block = self.block(n);
            convert::rpc_block(&block, block.hash(), &[self.tx.sender], full)
        }))
    }
    async fn block_by_hash(&self, hash: B256, full: bool) -> BackendResult<Option<RpcBlock>> {
        let block = self.block(HEAD);
        Ok((hash == block.hash())
            .then(|| convert::rpc_block(&block, hash, &[self.tx.sender], full)))
    }
    async fn block_transaction_count(&self, block: BlockId) -> BackendResult<Option<u64>> {
        Ok(match block {
            BlockId::Number(tag) => self.resolve(tag).map(|_| 1),
            BlockId::Hash(h) => (h.block_hash == self.block(HEAD).hash()).then_some(1),
        })
    }
    async fn transaction_by_hash(&self, hash: B256) -> BackendResult<Option<RpcTransaction>> {
        Ok((hash == self.tx.hash())
            .then(|| convert::rpc_transaction(&self.tx, Some(self.meta(HEAD)))))
    }
    async fn transaction_receipt(&self, hash: B256) -> BackendResult<Option<RpcReceipt>> {
        Ok((hash == self.tx.hash()).then(|| self.rpc_receipt(HEAD)))
    }
    async fn block_receipts(&self, block: BlockId) -> BackendResult<Option<Vec<RpcReceipt>>> {
        Ok(match block {
            BlockId::Number(tag) => self.resolve(tag).map(|n| vec![self.rpc_receipt(n)]),
            BlockId::Hash(_) => None,
        })
    }
    async fn logs(&self, filter: Filter, limit: Option<usize>) -> BackendResult<Vec<RpcLog>> {
        self.logs_calls.fetch_add(1, Ordering::SeqCst);
        *self.last_filter.lock().unwrap() = Some(filter);
        *self.last_logs_limit.lock().unwrap() = Some(limit);
        let n = self.logs_to_return.load(Ordering::SeqCst);
        let one = convert::rpc_logs(&self.receipt(), self.tx.hash(), self.meta(HEAD), 0);
        Ok(std::iter::repeat_n(one[0].clone(), n).collect())
    }
    async fn gas_price(&self) -> BackendResult<U256> {
        Ok(U256::from(2_000_000_000u64))
    }
    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256> {
        Ok(U256::from(1_000_000_000u64))
    }
    async fn next_base_fee(&self) -> BackendResult<u64> {
        Ok(1_000_000_000)
    }
    async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> BackendResult<FeeHistory> {
        let newest = self
            .resolve(newest_block)
            .ok_or(RpcBackendError::UnknownBlock)?;
        let count = block_count.min(newest + 1);
        Ok(FeeHistory {
            base_fee_per_gas: vec![1_000_000_000; count as usize + 1],
            gas_used_ratio: vec![0.5; count as usize],
            base_fee_per_blob_gas: vec![],
            blob_gas_used_ratio: vec![],
            oldest_block: newest + 1 - count,
            reward: reward_percentiles
                .map(|p| vec![vec![1_000_000_000u128; p.len()]; count as usize]),
        })
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct TestServer {
    mock: Arc<Mock>,
    handle: RpcServerHandle,
    client: HttpClient,
}

impl TestServer {
    async fn start() -> Self {
        Self::with_config(RpcServerConfig::default()).await
    }

    async fn with_config(mut config: RpcServerConfig) -> Self {
        config.http = SocketAddr::from(([127, 0, 0, 1], 0));
        let mock = Arc::new(Mock::new());
        let handle = xdevs_rpc::start_rpc_server(config, mock.clone())
            .await
            .unwrap();
        let client = HttpClientBuilder::default()
            .request_timeout(Duration::from_secs(10))
            .build(format!("http://{}", handle.local_addr))
            .unwrap();
        Self {
            mock,
            handle,
            client,
        }
    }

    async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value, ClientError> {
        self.client.request::<Value, _>(method, params).await
    }

    async fn ok(&self, method: &str, params: Vec<Value>) -> Value {
        self.call(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method} failed: {e}"))
    }

    /// Expect a JSON-RPC error; returns (code, message, data).
    async fn err(&self, method: &str, params: Vec<Value>) -> (i32, String, Option<Value>) {
        match self.call(method, params).await {
            Err(ClientError::Call(e)) => (
                e.code(),
                e.message().to_string(),
                e.data().map(|d| serde_json::from_str(d.get()).unwrap()),
            ),
            other => panic!("{method}: expected call error, got {other:?}"),
        }
    }

    fn last_block(&self) -> BlockId {
        self.mock
            .last_block
            .lock()
            .unwrap()
            .take()
            .expect("backend not called")
    }

    fn addr(&self) -> SocketAddr {
        self.handle.local_addr
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.stop();
    }
}

const ALICE: &str = "0x9d8a62f656a8d1615c1294fd71e9cfb3e4855a4f";

/// Minimal HTTP/1.1 exchange: returns (status, lower-cased header block, body).
async fn raw_http(addr: SocketAddr, request: String) -> (u16, String, String) {
    let fut = async {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let header_end = loop {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0, "connection closed before headers");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let status: u16 = head[9..12].parse().unwrap();
        let content_length = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .map(|v| v.trim().parse::<usize>().unwrap());
        match content_length {
            Some(len) => {
                while buf.len() < header_end + len {
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0, "connection closed mid-body");
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            None if status == 101 || status == 204 => {}
            None => {
                stream.read_to_end(&mut buf).await.unwrap();
            }
        }
        let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
        (status, head, body)
    };
    tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .expect("raw http timed out")
}

fn post(addr: SocketAddr, body: &str, extra_headers: &str) -> String {
    format!(
        "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
        body.len()
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn simple_methods_use_hex_quantities() {
    let s = TestServer::start().await;
    assert_eq!(s.ok("eth_chainId", vec![]).await, json!("0x1bcd"));
    assert_eq!(s.ok("net_version", vec![]).await, json!("7117"));
    assert_eq!(s.ok("eth_blockNumber", vec![]).await, json!("0x64"));
    assert_eq!(s.ok("net_peerCount", vec![]).await, json!("0x0"));
    assert_eq!(s.ok("net_listening", vec![]).await, json!(true));
    assert_eq!(
        s.ok("web3_clientVersion", vec![]).await,
        json!("xdevs/v0.1.0")
    );
    assert_eq!(s.ok("eth_accounts", vec![]).await, json!([]));
    assert_eq!(s.ok("eth_syncing", vec![]).await, json!(false));
    assert_eq!(s.ok("eth_gasPrice", vec![]).await, json!("0x77359400"));
    assert_eq!(
        s.ok("eth_maxPriorityFeePerGas", vec![]).await,
        json!("0x3b9aca00")
    );
    assert_eq!(
        s.ok("web3_sha3", vec![json!("0x68656c6c6f20776f726c64")])
            .await,
        json!("0x47173285a8d7341e5e972fc677286384f802f8ef42a5ec5f03bbfa254cb01fad")
    );
    assert_eq!(
        s.ok("eth_getBalance", vec![json!(ALICE), json!("latest")])
            .await,
        json!("0xde0b6b3a7640000")
    );
    assert_eq!(
        s.ok("eth_getCode", vec![json!(ALICE)]).await,
        json!("0x6001")
    );
    assert_eq!(
        s.ok(
            "eth_getTransactionCount",
            vec![json!(ALICE), json!("latest")]
        )
        .await,
        json!("0x5")
    );
    assert_eq!(
        s.ok("eth_sendRawTransaction", vec![json!("0x01")]).await,
        serde_json::to_value(keccak256([1u8])).unwrap()
    );
}

#[tokio::test]
async fn block_tags_are_normalised() {
    use BlockNumberOrTag::*;
    let s = TestServer::start().await;
    let cases = [
        (json!("latest"), BlockId::Number(Latest)),
        (json!("safe"), BlockId::Number(Latest)),
        (json!("finalized"), BlockId::Number(Latest)),
        (json!("pending"), BlockId::Number(Latest)),
        (json!("earliest"), BlockId::Number(Number(0))),
        (json!("0x5"), BlockId::Number(Number(5))),
        (json!({ "blockNumber": "0x1" }), BlockId::Number(Number(1))),
        (
            json!({ "blockNumber": "finalized" }),
            BlockId::Number(Latest),
        ),
    ];
    for (param, expected) in cases {
        s.ok("eth_getBalance", vec![json!(ALICE), param.clone()])
            .await;
        assert_eq!(s.last_block(), expected, "param {param}");
    }

    // Missing block param = latest.
    s.ok("eth_getBalance", vec![json!(ALICE)]).await;
    assert_eq!(s.last_block(), BlockId::Number(Latest));

    // EIP-1898 hash objects (and bare 32-byte hashes) reach the backend as hashes.
    let h = B256::repeat_byte(0xab);
    s.ok(
        "eth_getCode",
        vec![
            json!(ALICE),
            json!({ "blockHash": h, "requireCanonical": true }),
        ],
    )
    .await;
    assert_eq!(s.last_block(), BlockId::from((h, Some(true))));
    s.ok("eth_getCode", vec![json!(ALICE), json!(h)]).await;
    assert_eq!(s.last_block(), BlockId::from(h));

    // `pending` survives for eth_getTransactionCount (pool nonce).
    assert_eq!(
        s.ok(
            "eth_getTransactionCount",
            vec![json!(ALICE), json!("pending")]
        )
        .await,
        json!("0x6")
    );
    assert_eq!(s.last_block(), BlockId::Number(Pending));

    // Tags work for block queries too.
    let latest = s
        .ok(
            "eth_getBlockByNumber",
            vec![json!("finalized"), json!(false)],
        )
        .await;
    assert_eq!(latest["number"], json!("0x64"));
    let genesis = s
        .ok(
            "eth_getBlockByNumber",
            vec![json!("earliest"), json!(false)],
        )
        .await;
    assert_eq!(genesis["number"], json!("0x0"));
    assert_eq!(
        s.ok("eth_getBlockTransactionCountByNumber", vec![json!("safe")])
            .await,
        json!("0x1")
    );
}

#[tokio::test]
async fn storage_slot_accepts_quantity_and_padded_forms() {
    let s = TestServer::start().await;
    let zero = s
        .ok(
            "eth_getStorageAt",
            vec![json!(ALICE), json!("0x0"), json!("latest")],
        )
        .await;
    assert_eq!(zero, json!(format!("0x{}", "0".repeat(64))));
    let padded = format!("0x{}{}", "0".repeat(63), "5");
    let five = s
        .ok(
            "eth_getStorageAt",
            vec![json!(ALICE), json!(padded.clone())],
        )
        .await;
    assert_eq!(five, json!(padded));
}

#[tokio::test]
async fn unknown_things_are_null() {
    let s = TestServer::start().await;
    let unknown = json!(B256::repeat_byte(0x99));
    assert_eq!(
        s.ok("eth_getTransactionByHash", vec![unknown.clone()])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getTransactionReceipt", vec![unknown.clone()])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getBlockByHash", vec![unknown.clone(), json!(true)])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getBlockByNumber", vec![json!("0x3e8"), json!(true)])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getBlockTransactionCountByHash", vec![unknown.clone()])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getBlockTransactionCountByNumber", vec![json!("0x3e8")])
            .await,
        Value::Null
    );
    assert_eq!(
        s.ok("eth_getBlockReceipts", vec![json!("0x3e8")]).await,
        Value::Null
    );
}

#[tokio::test]
async fn responses_deserialize_into_alloy_types() {
    let s = TestServer::start().await;
    let tx_hash = s.mock.tx.hash();

    let full: RpcBlock = serde_json::from_value(
        s.ok("eth_getBlockByNumber", vec![json!("latest"), json!(true)])
            .await,
    )
    .unwrap();
    assert_eq!(full.header.number, HEAD);
    assert!(matches!(&full.transactions, BlockTransactions::Full(txs) if txs.len() == 1));
    assert_eq!(full.header.hash, s.mock.block(HEAD).hash());

    let by_hash = s
        .ok(
            "eth_getBlockByHash",
            vec![json!(full.header.hash), json!(false)],
        )
        .await;
    assert_eq!(by_hash["transactions"], json!([tx_hash]));
    let light: RpcBlock = serde_json::from_value(by_hash).unwrap();
    assert_eq!(light.header.hash, full.header.hash);

    let tx: RpcTransaction =
        serde_json::from_value(s.ok("eth_getTransactionByHash", vec![json!(tx_hash)]).await)
            .unwrap();
    assert_eq!(tx.block_number, Some(HEAD));
    assert_eq!(tx.inner.signer(), s.mock.tx.sender);

    let receipt: RpcReceipt = serde_json::from_value(
        s.ok("eth_getTransactionReceipt", vec![json!(tx_hash)])
            .await,
    )
    .unwrap();
    assert_eq!(receipt.transaction_hash, tx_hash);
    assert_eq!(receipt.gas_used, 21_000);
    assert!(receipt.inner.status());

    let receipts: Vec<RpcReceipt> =
        serde_json::from_value(s.ok("eth_getBlockReceipts", vec![json!("latest")]).await).unwrap();
    assert_eq!(receipts.len(), 1);

    let logs: Vec<RpcLog> = serde_json::from_value(
        s.ok(
            "eth_getLogs",
            vec![json!({ "fromBlock": "0x1", "toBlock": "latest" })],
        )
        .await,
    )
    .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].transaction_hash, Some(tx_hash));

    // MetaMask-style fee history (hex count, percentiles).
    let fh = s
        .ok(
            "eth_feeHistory",
            vec![json!("0x5"), json!("latest"), json!([10, 20, 30])],
        )
        .await;
    let fh: FeeHistory = serde_json::from_value(fh).unwrap();
    assert_eq!(fh.oldest_block, HEAD - 4);
    assert_eq!(fh.base_fee_per_gas.len(), 6);
    assert_eq!(fh.reward.as_ref().map(|r| r[0].len()), Some(3));
    // Decimal count, no percentiles -> no `reward` field.
    let fh = s
        .ok("eth_feeHistory", vec![json!(3), json!("pending")])
        .await;
    assert!(fh.get("reward").is_none());
    assert_eq!(fh["oldestBlock"], json!("0x62"));
}

#[tokio::test]
async fn call_and_estimate_gas() {
    let config = RpcServerConfig {
        gas_cap: 1_000_000,
        ..Default::default()
    };
    let s = TestServer::with_config(config).await;

    // MetaMask transfer estimate: only from/to/value.
    let est = s
        .ok(
            "eth_estimateGas",
            vec![json!({ "from": ALICE, "to": "0x3535353535353535353535353535353535353535", "value": "0x1" })],
        )
        .await;
    assert_eq!(est, json!("0x5208"));

    // eth_call: gas above the cap is clamped, below it is kept.
    let out = s
        .ok(
            "eth_call",
            vec![
                json!({ "to": ALICE, "data": "0x01", "gas": "0x10000000000" }),
                json!("latest"),
            ],
        )
        .await;
    assert_eq!(out, json!("0x01"));
    let req = s.mock.last_request.lock().unwrap().take().unwrap();
    assert_eq!(req.gas, Some(1_000_000));
    s.ok(
        "eth_call",
        vec![json!({ "to": ALICE, "input": "0x01", "gas": "0x5208" })],
    )
    .await;
    let req = s.mock.last_request.lock().unwrap().take().unwrap();
    assert_eq!(req.gas, Some(21_000));

    // Revert: code 3, message with reason, data = raw revert bytes.
    let (code, msg, data) = s
        .err("eth_call", vec![json!({ "to": ALICE, "data": "0xdead" })])
        .await;
    assert_eq!(code, 3);
    assert_eq!(msg, "execution reverted: nope");
    let data = data.unwrap();
    assert!(data.as_str().unwrap().starts_with("0x08c379a0"));

    // State overrides are refused explicitly rather than ignored.
    let (code, _, _) = s
        .err(
            "eth_call",
            vec![
                json!({ "to": ALICE }),
                json!("latest"),
                json!({ ALICE: { "balance": "0x1" } }),
            ],
        )
        .await;
    assert_eq!(code, -32602);
}

#[tokio::test]
async fn error_codes() {
    let s = TestServer::start().await;
    // Malformed / missing params.
    assert_eq!(
        s.err("eth_getBalance", vec![json!("0x1234")]).await.0,
        -32602
    );
    assert_eq!(s.err("eth_getBalance", vec![]).await.0, -32602);
    assert_eq!(
        s.err("eth_getBlockByNumber", vec![json!("newest")]).await.0,
        -32602
    );
    assert_eq!(
        s.err("eth_sendRawTransaction", vec![json!("0x")]).await.0,
        -32602
    );
    assert_eq!(
        s.err(
            "eth_feeHistory",
            vec![json!("0x5"), json!("latest"), json!([30, 10])]
        )
        .await
        .0,
        -32602
    );
    // Unknown block for state queries: -32000 "header not found".
    let (code, msg, _) = s
        .err("eth_getBalance", vec![json!(ALICE), json!("0x3e8")])
        .await;
    assert_eq!((code, msg.as_str()), (-32000, "header not found"));
    // Pool rejection: -32000 with geth wording.
    let (code, msg, _) = s.err("eth_sendRawTransaction", vec![json!("0xff")]).await;
    assert_eq!((code, msg.as_str()), (-32000, "nonce too low"));
    // Unknown method.
    assert_eq!(s.err("eth_mining", vec![]).await.0, -32601);
}

#[tokio::test]
async fn get_logs_limits() {
    let config = RpcServerConfig {
        max_logs_range: 10,
        max_logs_results: 3,
        ..Default::default()
    };
    let s = TestServer::with_config(config).await;

    // Tags are resolved to numbers before reaching the backend.
    s.ok("eth_getLogs", vec![json!({ "fromBlock": "0x5b" })])
        .await;
    let f = s.mock.last_filter.lock().unwrap().take().unwrap();
    assert_eq!(
        f.block_option,
        FilterBlockOption::Range {
            from_block: Some(BlockNumberOrTag::Number(91)),
            to_block: Some(BlockNumberOrTag::Number(HEAD)),
        }
    );

    // 11 blocks > max range 10.
    let (code, msg, _) = s
        .err(
            "eth_getLogs",
            vec![json!({ "fromBlock": "0x5a", "toBlock": "latest" })],
        )
        .await;
    assert_eq!(code, -32005);
    assert!(msg.contains("10"), "{msg}");

    // from > to.
    assert_eq!(
        s.err(
            "eth_getLogs",
            vec![json!({ "fromBlock": "0x60", "toBlock": "0x50" })]
        )
        .await
        .0,
        -32602
    );

    // Starting after the head: empty without touching the backend.
    let before = s.mock.logs_calls.load(Ordering::SeqCst);
    assert_eq!(
        s.ok("eth_getLogs", vec![json!({ "fromBlock": "0x65" })])
            .await,
        json!([])
    );
    assert_eq!(s.mock.logs_calls.load(Ordering::SeqCst), before);

    // Block-hash filters skip the range check.
    s.ok(
        "eth_getLogs",
        vec![json!({ "blockHash": B256::repeat_byte(3) })],
    )
    .await;

    // The result cap is handed to the backend so it can stop collecting early.
    assert_eq!(s.mock.last_logs_limit.lock().unwrap().take(), Some(Some(3)));

    // Too many results (server-side backstop for backends that ignore the limit).
    s.mock.logs_to_return.store(4, Ordering::SeqCst);
    let (code, msg, _) = s.err("eth_getLogs", vec![json!({})]).await;
    assert_eq!(code, -32005);
    assert!(msg.contains("more than 3 results"), "{msg}");
    s.mock.logs_to_return.store(3, Ordering::SeqCst);
    assert_eq!(
        s.ok("eth_getLogs", vec![json!({})])
            .await
            .as_array()
            .unwrap()
            .len(),
        3
    );

    // `max_logs_results = 0` means unlimited.
    let unlimited = TestServer::with_config(RpcServerConfig {
        max_logs_results: 0,
        ..Default::default()
    })
    .await;
    unlimited.mock.logs_to_return.store(20, Ordering::SeqCst);
    unlimited.ok("eth_getLogs", vec![json!({})]).await;
    assert_eq!(
        unlimited.mock.last_logs_limit.lock().unwrap().take(),
        Some(None)
    );
}

#[tokio::test]
async fn batch_limit() {
    let s = TestServer::start().await;
    let batch = |n: usize| {
        let calls: Vec<Value> = (0..n)
            .map(|i| json!({ "jsonrpc": "2.0", "id": i, "method": "eth_chainId", "params": [] }))
            .collect();
        serde_json::to_string(&calls).unwrap()
    };

    let (status, _, body) = raw_http(s.addr(), post(s.addr(), &batch(50), "")).await;
    assert_eq!(status, 200);
    let responses: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(responses.len(), 50);
    assert!(responses.iter().all(|r| r["result"] == json!("0x1bcd")));

    let (_, _, body) = raw_http(s.addr(), post(s.addr(), &batch(51), "")).await;
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], json!(-32010), "{response}");
}

#[tokio::test]
async fn request_body_limit() {
    let config = RpcServerConfig {
        max_request_body_size: 1024,
        ..Default::default()
    };
    let s = TestServer::with_config(config).await;
    let big = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"eth_sendRawTransaction","params":["0x{}"]}}"#,
        "ab".repeat(1024)
    );
    let (status, _, _) = raw_http(s.addr(), post(s.addr(), &big, "")).await;
    assert_eq!(status, 413);

    let small = r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#;
    let (status, _, body) = raw_http(s.addr(), post(s.addr(), small, "")).await;
    assert_eq!(status, 200);
    assert!(body.contains("0x1bcd"));
}

#[tokio::test]
async fn cors_allow_all() {
    let s = TestServer::start().await;
    let preflight = format!(
        "OPTIONS / HTTP/1.1\r\nHost: {}\r\nOrigin: chrome-extension://metamask\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: content-type\r\nConnection: close\r\n\r\n",
        s.addr()
    );
    let (status, head, _) = raw_http(s.addr(), preflight).await;
    assert_eq!(status, 204);
    assert!(head.contains("access-control-allow-origin: *"), "{head}");
    assert!(
        head.contains("access-control-allow-methods: post, get, options"),
        "{head}"
    );
    assert!(
        head.contains("access-control-allow-headers: content-type"),
        "{head}"
    );

    let body = r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#;
    let (status, head, _) = raw_http(
        s.addr(),
        post(s.addr(), body, "Origin: https://dapp.example\r\n"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(head.contains("access-control-allow-origin: *"), "{head}");
}

#[tokio::test]
async fn cors_allow_list() {
    let config = RpcServerConfig {
        cors: vec!["https://app.xdevs.io".into()],
        ..Default::default()
    };
    let s = TestServer::with_config(config).await;
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#;

    let (_, head, _) = raw_http(
        s.addr(),
        post(s.addr(), body, "Origin: https://app.xdevs.io\r\n"),
    )
    .await;
    assert!(
        head.contains("access-control-allow-origin: https://app.xdevs.io"),
        "{head}"
    );

    let (status, head, _) = raw_http(
        s.addr(),
        post(s.addr(), body, "Origin: https://evil.example\r\n"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!head.contains("access-control-allow-origin"), "{head}");
}

#[tokio::test]
async fn websocket_upgrade_is_served() {
    let s = TestServer::start().await;
    let upgrade = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        s.addr()
    );
    let (status, head, _) = raw_http(s.addr(), upgrade).await;
    assert_eq!(status, 101, "{head}");
    assert!(
        head.contains("sec-websocket-accept: s3pplmbitxaq9kygzzhzrbk+xoo="),
        "{head}"
    );
}
