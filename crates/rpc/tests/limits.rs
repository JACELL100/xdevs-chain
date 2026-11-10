//! Request-side DoS limits (security review "RPC load can starve consensus"): the
//! per-connection in-flight cap and the per-IP token bucket, over a mock backend.

use jsonrpsee::{
    core::{client::ClientT, params::BatchRequestBuilder, ClientError},
    http_client::HttpClientBuilder,
    rpc_params,
    ws_client::WsClientBuilder,
};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use xdevs_primitives::{Address, Bytes, B256, U256};
use xdevs_rpc::{types::*, BackendResult, RpcBackend, RpcBackendError, RpcServerConfig};

const SLOW_CALL: Duration = Duration::from_millis(800);

struct Mock;

#[async_trait::async_trait]
impl RpcBackend for Mock {
    fn chain_id(&self) -> u64 {
        7117
    }
    async fn block_number(&self) -> BackendResult<u64> {
        Ok(7)
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
        // A slow call (think: a heavy eth_call) so calls overlap.
        tokio::time::sleep(SLOW_CALL).await;
        Ok(Bytes::from_static(b"ok"))
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
}

fn unsupported<T>() -> BackendResult<T> {
    Err(RpcBackendError::Unsupported("mock".into()))
}

async fn start(config: RpcServerConfig) -> xdevs_rpc::RpcServerHandle {
    let config = RpcServerConfig {
        http: SocketAddr::from(([127, 0, 0, 1], 0)),
        ..config
    };
    xdevs_rpc::start_server(config, Arc::new(Mock))
        .await
        .unwrap()
}

fn code(e: &ClientError) -> Option<i32> {
    match e {
        ClientError::Call(obj) => Some(obj.code()),
        _ => None,
    }
}

fn call_req() -> Value {
    json!({"to": "0x00000000000000000000000000000000000000aa"})
}

/// Regression: one WebSocket client firing many heavy calls at once gets at most
/// `max_in_flight_per_connection` of them running; the rest fail at once with -32005
/// (nothing piles up). Other connections are not affected and slots are reused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_flight_cap_per_connection() {
    let handle = start(RpcServerConfig {
        max_in_flight_per_connection: 4,
        ..Default::default()
    })
    .await;
    let url = format!("ws://{}", handle.local_addr);
    let ws = Arc::new(
        WsClientBuilder::default()
            .request_timeout(Duration::from_secs(10))
            .build(&url)
            .await
            .unwrap(),
    );
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let ws = ws.clone();
        tasks.spawn(async move {
            ws.request::<Value, _>("eth_call", rpc_params![call_req()])
                .await
        });
    }
    // A second connection still gets its own slots meanwhile.
    let other = WsClientBuilder::default().build(&url).await.unwrap();
    let started = std::time::Instant::now();
    let v: Value = other
        .request("eth_call", rpc_params![call_req()])
        .await
        .unwrap();
    assert_eq!(v, json!("0x6f6b"));
    assert!(started.elapsed() < SLOW_CALL * 3);

    let (mut ok, mut limited) = (0, 0);
    while let Some(r) = tasks.join_next().await {
        match r.unwrap() {
            Ok(_) => ok += 1,
            Err(e) => {
                assert_eq!(code(&e), Some(-32005), "{e}");
                assert!(
                    e.to_string().contains("too many concurrent requests"),
                    "{e}"
                );
                limited += 1;
            }
        }
    }
    assert_eq!(ok, 4, "exactly the cap ran");
    assert_eq!(limited, 8);
    // Slots are released when calls finish.
    let v: Value = ws
        .request("eth_call", rpc_params![call_req()])
        .await
        .unwrap();
    assert_eq!(v, json!("0x6f6b"));
    handle.stop();
}

/// Plan §19.3: the per-IP token bucket refuses calls (single and batch entries) over
/// the budget with -32005, and refills over time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_ip_rate_limit() {
    let handle = start(RpcServerConfig {
        rate_limit_per_ip: 5,
        rate_limit_burst: 5,
        ..Default::default()
    })
    .await;
    let http = HttpClientBuilder::default()
        .build(format!("http://{}", handle.local_addr))
        .unwrap();
    let mut results = Vec::new();
    for _ in 0..8 {
        results.push(
            http.request::<Value, _>("eth_blockNumber", rpc_params![])
                .await,
        );
    }
    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert!((5..=6).contains(&ok), "burst of 5 (+ refill): {results:?}");
    let err = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert_eq!(code(err), Some(-32005));
    assert!(err.to_string().contains("rate limit"), "{err}");

    // The bucket refills (5 tokens per second); a WS connection from the same IP
    // shares it.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let ws = WsClientBuilder::default()
        .build(format!("ws://{}", handle.local_addr))
        .await
        .unwrap();
    let mut batch = BatchRequestBuilder::new();
    for _ in 0..8 {
        batch.insert("eth_blockNumber", rpc_params![]).unwrap();
    }
    let out = ws.batch_request::<Value>(batch).await.unwrap();
    let ok = out.iter().filter(|r| r.is_ok()).count();
    assert!((5..=6).contains(&ok), "{out:?}");
    assert!(out
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| e.code() == -32005));
    handle.stop();
}
