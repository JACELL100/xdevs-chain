//! Minimal Ethereum JSON-RPC client (only what the CLI needs).

use std::time::Duration;

use alloy_rpc_types_eth::TransactionRequest;
use anyhow::{anyhow, Context};
use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use serde_json::Value;
use xdevs_primitives::{reexports::alloy_primitives::U64, Address, Bytes, B256, U256};

/// JSON-RPC client over HTTP.
pub struct Rpc {
    client: HttpClient,
    url: String,
}

fn u256_field(v: &Value, field: &str) -> anyhow::Result<Option<U256>> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => serde_json::from_value(x.clone())
            .map(Some)
            .with_context(|| format!("invalid {field} in response")),
    }
}

impl Rpc {
    /// Connect lazily to `url`.
    pub fn new(url: &str) -> anyhow::Result<Self> {
        let client = HttpClientBuilder::default()
            .request_timeout(Duration::from_secs(30))
            .build(url)
            .with_context(|| format!("invalid RPC url {url}"))?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: jsonrpsee::core::params::ArrayParams,
    ) -> anyhow::Result<R> {
        self.client
            .request(method, params)
            .await
            .map_err(|e| anyhow!("{method} via {}: {e}", self.url))
    }

    /// `eth_chainId`.
    pub async fn chain_id(&self) -> anyhow::Result<u64> {
        let id: U64 = self.call("eth_chainId", rpc_params![]).await?;
        Ok(id.to())
    }

    /// `eth_getTransactionCount(addr, "pending")`.
    pub async fn pending_nonce(&self, addr: Address) -> anyhow::Result<u64> {
        let n: U64 = self
            .call("eth_getTransactionCount", rpc_params![addr, "pending"])
            .await?;
        Ok(n.to())
    }

    /// `eth_getBalance(addr, block)`.
    pub async fn balance(&self, addr: Address, block: &str) -> anyhow::Result<U256> {
        self.call("eth_getBalance", rpc_params![addr, block]).await
    }

    /// `baseFeePerGas` of the latest block.
    pub async fn latest_base_fee(&self) -> anyhow::Result<u128> {
        let block: Value = self
            .call("eth_getBlockByNumber", rpc_params!["latest", false])
            .await?;
        let fee = u256_field(&block, "baseFeePerGas")?
            .ok_or_else(|| anyhow!("latest block has no baseFeePerGas (pre-London chain?)"))?;
        u128::try_from(fee).context("base fee too large")
    }

    /// `eth_maxPriorityFeePerGas`.
    pub async fn max_priority_fee(&self) -> anyhow::Result<u128> {
        let tip: U256 = self.call("eth_maxPriorityFeePerGas", rpc_params![]).await?;
        u128::try_from(tip).context("priority fee too large")
    }

    /// `eth_estimateGas(request)`.
    pub async fn estimate_gas(&self, request: &TransactionRequest) -> anyhow::Result<u64> {
        let gas: U64 = self.call("eth_estimateGas", rpc_params![request]).await?;
        Ok(gas.to())
    }

    /// `eth_sendRawTransaction(raw)`.
    pub async fn send_raw(&self, raw: Bytes) -> anyhow::Result<B256> {
        self.call("eth_sendRawTransaction", rpc_params![raw]).await
    }

    /// `eth_getTransactionReceipt(hash)` as raw JSON (`None` while pending).
    pub async fn receipt(&self, hash: B256) -> anyhow::Result<Option<Value>> {
        let r: Value = self
            .call("eth_getTransactionReceipt", rpc_params![hash])
            .await?;
        Ok((!r.is_null()).then_some(r))
    }
}

/// `(status, block number)` of a receipt JSON.
pub fn receipt_summary(receipt: &Value) -> anyhow::Result<(bool, Option<u64>)> {
    let status = u256_field(receipt, "status")?
        .map(|s| !s.is_zero())
        .unwrap_or(true);
    let block = u256_field(receipt, "blockNumber")?.and_then(|b| u64::try_from(b).ok());
    Ok((status, block))
}
