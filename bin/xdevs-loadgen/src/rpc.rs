//! JSON-RPC plumbing: one HTTP client per endpoint, round-robin selection, small typed
//! helpers and send-error classification.

use anyhow::{anyhow, bail, Context};
use jsonrpsee::{
    core::{client::ClientT, ClientError},
    http_client::{HttpClient, HttpClientBuilder},
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use xdevs_primitives::{Address, Bytes, B256, U256};

/// HTTP clients for all endpoints.
#[derive(Debug)]
pub struct RpcPool {
    urls: Vec<String>,
    clients: Vec<HttpClient>,
    next: AtomicUsize,
}

impl RpcPool {
    /// Connect to every URL (`request_timeout` per call).
    pub fn new(urls: &[String], request_timeout: Duration) -> anyhow::Result<Self> {
        if urls.is_empty() {
            bail!("no RPC URL given");
        }
        let clients = urls
            .iter()
            .map(|u| {
                HttpClientBuilder::default()
                    .request_timeout(request_timeout)
                    .max_request_size(8 * 1024 * 1024)
                    .max_response_size(64 * 1024 * 1024)
                    .build(u)
                    .with_context(|| format!("bad RPC URL {u}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            urls: urls.to_vec(),
            clients,
            next: AtomicUsize::new(0),
        })
    }

    /// Endpoint URLs.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// The first endpoint (used for setup and block polling).
    pub fn primary(&self) -> &HttpClient {
        &self.clients[0]
    }

    /// Next endpoint, round-robin.
    pub fn next_client(&self) -> &HttpClient {
        let i = self.next.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        &self.clients[i]
    }
}

/// Raw call.
pub async fn call(c: &HttpClient, method: &str, params: Vec<Value>) -> Result<Value, ClientError> {
    c.request::<Value, _>(method, params).await
}

/// Call that must succeed.
pub async fn call_ok(c: &HttpClient, method: &str, params: Vec<Value>) -> anyhow::Result<Value> {
    call(c, method, params)
        .await
        .map_err(|e| anyhow!("{method}: {e}"))
}

/// Parse a `0x`-hex quantity.
pub fn parse_u64(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    u64::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

/// Parse a `0x`-hex quantity up to 128 bits.
pub fn parse_u128(v: &Value) -> Option<u128> {
    let s = v.as_str()?;
    u128::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

/// `eth_chainId`.
pub async fn chain_id(c: &HttpClient) -> anyhow::Result<u64> {
    let v = call_ok(c, "eth_chainId", vec![]).await?;
    parse_u64(&v).ok_or_else(|| anyhow!("bad eth_chainId {v}"))
}

/// `eth_blockNumber`.
pub async fn block_number(c: &HttpClient) -> Result<u64, ClientError> {
    let v = call(c, "eth_blockNumber", vec![]).await?;
    parse_u64(&v).ok_or_else(|| ClientError::Custom(format!("bad eth_blockNumber {v}")))
}

/// `eth_getTransactionCount(addr, tag)`.
pub async fn nonce(c: &HttpClient, addr: Address, tag: &str) -> Result<u64, ClientError> {
    let v = call(c, "eth_getTransactionCount", vec![json!(addr), json!(tag)]).await?;
    parse_u64(&v).ok_or_else(|| ClientError::Custom(format!("bad nonce {v}")))
}

/// `eth_getBalance(addr, latest)`.
pub async fn balance(c: &HttpClient, addr: Address) -> anyhow::Result<U256> {
    let v = call_ok(c, "eth_getBalance", vec![json!(addr), json!("latest")]).await?;
    serde_json::from_value(v.clone()).map_err(|_| anyhow!("bad balance {v}"))
}

/// `eth_sendRawTransaction`.
pub async fn send_raw(c: &HttpClient, raw: &Bytes) -> Result<Value, ClientError> {
    call(c, "eth_sendRawTransaction", vec![json!(raw)]).await
}

/// `eth_call(to, data)` at `latest`, returning the output bytes.
pub async fn eth_call(c: &HttpClient, to: Address, data: &Bytes) -> anyhow::Result<Bytes> {
    let v = call_ok(
        c,
        "eth_call",
        vec![json!({ "to": to, "data": data }), json!("latest")],
    )
    .await?;
    serde_json::from_value(v.clone()).map_err(|_| anyhow!("bad eth_call output {v}"))
}

/// Suggested fees: `(base fee of latest block, priority fee)`.
pub async fn fees(c: &HttpClient) -> anyhow::Result<(u128, u128)> {
    let block = call_ok(
        c,
        "eth_getBlockByNumber",
        vec![json!("latest"), json!(false)],
    )
    .await?;
    let base = block
        .get("baseFeePerGas")
        .and_then(parse_u128)
        .unwrap_or(1_000_000_000);
    let tip = match call(c, "eth_maxPriorityFeePerGas", vec![]).await {
        Ok(v) => parse_u128(&v).unwrap_or(1_000_000_000),
        Err(_) => 1_000_000_000,
    };
    Ok((base, tip))
}

/// Poll `eth_getTransactionReceipt` for every hash until all are mined (setup only:
/// the measured phase never polls receipts per tx). Returns the receipts in order.
pub async fn wait_receipts(
    c: &HttpClient,
    hashes: &[B256],
    timeout: Duration,
) -> anyhow::Result<Vec<Value>> {
    let deadline = Instant::now() + timeout;
    let mut out = vec![Value::Null; hashes.len()];
    loop {
        for (i, h) in hashes.iter().enumerate() {
            if out[i].is_null() {
                out[i] = call(c, "eth_getTransactionReceipt", vec![json!(h)])
                    .await
                    .unwrap_or(Value::Null);
            }
        }
        if out.iter().all(|r| !r.is_null()) {
            return Ok(out);
        }
        if Instant::now() > deadline {
            let missing = out.iter().filter(|r| r.is_null()).count();
            bail!("{missing} setup tx(s) not mined within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Whether a receipt JSON has `status == 1`.
pub fn receipt_ok(r: &Value) -> bool {
    r.get("status").and_then(parse_u64) == Some(1)
}

/// How a failed `eth_sendRawTransaction` should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// The node already has this exact tx: count as accepted.
    AlreadyKnown,
    /// The sender's nonce view is wrong (too low / too high / replacement): resync.
    Nonce,
    /// Definitively rejected for another reason (fees, pool full, ...): reuse the nonce.
    Rejected,
    /// Outcome unknown (timeout, transport): resync the sender.
    Ambiguous,
}

/// Normalize an error message to a short key for counting: drops the variable details
/// after `:` (`"nonce too low: next nonce 5, tx nonce 3"` -> `"nonce too low"`) and
/// addresses (`"sender 0x.. has too many transactions in the pool"`).
pub fn normalize_error(msg: &str) -> String {
    let m = msg.trim();
    if m.starts_with("sender ") && m.contains("too many transactions") {
        return "sender has too many transactions in the pool".into();
    }
    let head = m.split(':').next().unwrap_or(m).trim();
    let mut key: String = head.chars().take(80).collect();
    if key.is_empty() {
        key = "unknown error".into();
    }
    key.to_ascii_lowercase()
}

/// Classify a send error: `(counting key, handling)`.
pub fn classify(err: &ClientError) -> (String, SendFailure) {
    match err {
        ClientError::Call(e) => classify_message(e.message()),
        ClientError::RequestTimeout => ("rpc timeout".into(), SendFailure::Ambiguous),
        ClientError::Transport(e) => (
            format!("transport: {}", normalize_error(&e.to_string())),
            SendFailure::Ambiguous,
        ),
        other => (normalize_error(&other.to_string()), SendFailure::Ambiguous),
    }
}

/// Classify a JSON-RPC error message from `eth_sendRawTransaction`.
pub fn classify_message(msg: &str) -> (String, SendFailure) {
    let key = normalize_error(msg);
    let kind = if key.contains("already known") || key.contains("already imported") {
        SendFailure::AlreadyKnown
    } else if key.contains("nonce too low")
        || key.contains("nonce too high")
        || key.contains("replacement transaction underpriced")
    {
        SendFailure::Nonce
    } else {
        SendFailure::Rejected
    };
    (key, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_errors() {
        assert_eq!(
            normalize_error("nonce too low: next nonce 5, tx nonce 3"),
            "nonce too low"
        );
        assert_eq!(
            normalize_error("sender 0xf39f...2266 has too many transactions in the pool"),
            "sender has too many transactions in the pool"
        );
        assert_eq!(
            normalize_error("insufficient funds for gas * price + value: balance 0, tx cost 1"),
            "insufficient funds for gas * price + value"
        );
        assert_eq!(normalize_error("txpool is full"), "txpool is full");
        assert_eq!(normalize_error(""), "unknown error");
    }

    #[test]
    fn classifies_send_failures() {
        use SendFailure::*;
        assert_eq!(classify_message("already known").1, AlreadyKnown);
        assert_eq!(
            classify_message("nonce too low: next nonce 5, tx nonce 3").1,
            Nonce
        );
        assert_eq!(
            classify_message("nonce too high: next nonce 5, tx nonce 90").1,
            Nonce
        );
        assert_eq!(
            classify_message("replacement transaction underpriced").1,
            Nonce
        );
        assert_eq!(
            classify_message("transaction underpriced: max fee 1 < required 2").1,
            Rejected
        );
        assert_eq!(classify_message("txpool is full").1, Rejected);
        assert_eq!(classify(&ClientError::RequestTimeout).1, Ambiguous);
        assert_eq!(classify(&ClientError::RequestTimeout).0, "rpc timeout");
    }

    #[test]
    fn parses_quantities() {
        assert_eq!(parse_u64(&json!("0x10")), Some(16));
        assert_eq!(parse_u64(&json!("16")), None);
        assert_eq!(parse_u64(&json!(16)), None);
        assert_eq!(parse_u128(&json!("0x3b9aca00")), Some(1_000_000_000));
        assert!(receipt_ok(&json!({ "status": "0x1" })));
        assert!(!receipt_ok(&json!({ "status": "0x0" })));
        assert!(!receipt_ok(&json!({})));
    }
}
