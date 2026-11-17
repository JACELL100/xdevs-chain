//! Phase 4 safety RPCs end to end: `xdevs_checkRecipient`, `xdevs_getApprovals`,
//! `xdevs_getVaultQueue`, `xdevs_resolveName` / `xdevs_reverseName` and the
//! `xdevs_simulateTransaction` plumbing, over a mock backend that answers the view calls
//! (`eth_call`) and log scans (`eth_getLogs`) the methods are built from.

use jsonrpsee::{
    core::{client::ClientT, ClientError},
    http_client::{HttpClient, HttpClientBuilder},
};
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use xdevs_primitives::{
    safety::{
        abi::{self, address_word, encode_call, u256_word},
        xns_namehash, xns_skeleton_hash, SimulationReport, PERMIT2_EXPIRING, SMART_ACCOUNT_IMPL,
        TOPICS, XNS_REGISTRY,
    },
    set_compat_header_fields, Address, Block, Bytes, Header, Log, LogData, B256, U256,
};
use xdevs_rpc::{
    convert, types::*, BackendResult, RpcBackend, RpcBackendError, RpcServerConfig, RpcServerHandle,
};

const HEAD: u64 = 100;
const NOW: u64 = 1_700_000_000;
const ALICE: Address = Address::repeat_byte(0xa1);
const OWNER: Address = Address::repeat_byte(0x0e);
const SPENDER: Address = Address::repeat_byte(0x5e);
const TOKEN: Address = Address::repeat_byte(0x70);
const ACCOUNT: Address = Address::repeat_byte(0xac);

/// A past recipient of OWNER and a poisoned look-alike (same first / last 2 bytes).
fn past() -> Address {
    with_ends([0x11; 20])
}

fn lookalike() -> Address {
    with_ends([0x99; 20])
}

fn with_ends(mut b: [u8; 20]) -> Address {
    b[..2].copy_from_slice(&[0x12, 0x34]);
    b[18..].copy_from_slice(&[0x56, 0x78]);
    Address::from(b)
}

fn word(v: u64) -> Vec<u8> {
    u256_word(U256::from(v)).0.to_vec()
}

fn abi_string(s: &str) -> Bytes {
    let mut out = word(32);
    out.extend(word(s.len() as u64));
    let mut padded = s.as_bytes().to_vec();
    padded.resize(s.len().div_ceil(32) * 32, 0);
    out.extend(padded);
    out.into()
}

/// `getAction(id)` return data with one call `(ALICE, 5, 0xbeef)`.
fn action(execute_after: u64, status: u64, valid: bool) -> Bytes {
    let mut out = Vec::new();
    for v in [execute_after, status, u64::from(valid), 128, 1, 32] {
        out.extend(word(v));
    }
    out.extend(address_word(ALICE).0);
    for v in [5, 96, 2] {
        out.extend(word(v));
    }
    let mut tail = [0u8; 32];
    tail[..2].copy_from_slice(&[0xbe, 0xef]);
    out.extend(tail);
    out.into()
}

struct Mock {
    logs: Vec<RpcLog>,
    filters: Mutex<Vec<Filter>>,
}

fn rpc_log(address: Address, topics: Vec<B256>, data: Vec<u8>, block: u64) -> RpcLog {
    RpcLog {
        inner: Log {
            address,
            data: LogData::new_unchecked(topics, data.into()),
        },
        block_number: Some(block),
        transaction_hash: Some(B256::repeat_byte(block as u8)),
        log_index: Some(0),
        ..Default::default()
    }
}

impl Mock {
    fn new() -> Self {
        let t = &*TOPICS;
        let logs = vec![
            // OWNER paid `past()` in the past.
            rpc_log(
                TOKEN,
                vec![t.transfer, address_word(OWNER), address_word(past())],
                word(10),
                50,
            ),
            // The look-alike sent OWNER a zero-value transfer (address poisoning).
            rpc_log(
                TOKEN,
                vec![t.transfer, address_word(lookalike()), address_word(OWNER)],
                word(0),
                60,
            ),
            // OWNER approved SPENDER on TOKEN (unlimited, still live).
            rpc_log(
                TOKEN,
                vec![t.approval, address_word(OWNER), address_word(SPENDER)],
                u256_word(U256::MAX).0.to_vec(),
                70,
            ),
            // A Permit2 approval that has since been revoked (live allowance 0).
            rpc_log(
                PERMIT2_EXPIRING,
                vec![
                    t.permit2_approval,
                    address_word(OWNER),
                    address_word(TOKEN),
                    address_word(ALICE),
                ],
                [word(1), word(NOW + 10)].concat(),
                80,
            ),
        ];
        Self {
            logs,
            filters: Mutex::new(Vec::new()),
        }
    }

    fn block(&self, number: u64) -> RpcBlock {
        let mut header = Header {
            number,
            gas_limit: 30_000_000,
            timestamp: NOW,
            base_fee_per_gas: Some(1_000_000_000),
            ..Default::default()
        };
        set_compat_header_fields(&mut header);
        let block = Block::new(header, Vec::new());
        convert::rpc_block(&block, block.hash(), &[], false)
    }

    fn view(&self, to: Address, input: &[u8]) -> BackendResult<Bytes> {
        let call = |sig: &str, args: &[B256]| encode_call(sig, args);
        let reverted = || RpcBackendError::Reverted {
            data: Bytes::new(),
            reason: None,
        };
        let w = |a: Address| Bytes::from(address_word(a).0.to_vec());
        let alice_node = xns_namehash("alice.xdevs");
        if to == XNS_REGISTRY {
            if input == call("resolveAddr(bytes32)", &[alice_node]).as_ref() {
                return Ok(w(ALICE));
            }
            if input.starts_with(&abi::selector("resolveAddr(bytes32)")) {
                return Ok(Bytes::from(word(0)));
            }
            if input == call("primaryName(address)", &[address_word(ALICE)]).as_ref() {
                return Ok(abi_string("alice.xdevs"));
            }
            if input.starts_with(&abi::selector("primaryName(address)")) {
                return Ok(abi_string(""));
            }
            if input == call("skeletonHolder(bytes32)", &[xns_skeleton_hash("alice")]).as_ref() {
                return Ok(Bytes::from(alice_node.0.to_vec()));
            }
            return Ok(Bytes::from(word(0)));
        }
        if to == TOKEN {
            if input
                == call(
                    "allowance(address,address)",
                    &[address_word(OWNER), address_word(SPENDER)],
                )
                .as_ref()
            {
                return Ok(Bytes::from(u256_word(U256::MAX).0.to_vec()));
            }
            if input == call("symbol()", &[]).as_ref() {
                return Ok(abi_string("TKN"));
            }
            return Err(reverted());
        }
        if to == PERMIT2_EXPIRING {
            // (amount 0, expiration 0, nonce 0): revoked.
            return Ok([word(0), word(0), word(0)].concat().into());
        }
        if to == ACCOUNT {
            if input == call("lastActionId()", &[]).as_ref() {
                return Ok(Bytes::from(word(3)));
            }
            for (id, data) in [
                (1, action(NOW - 10, 3, true)),     // executed
                (2, action(NOW + 3_600, 1, true)),  // queued
                (3, action(NOW + 3_600, 1, false)), // invalidated by a recovery
            ] {
                if input == call("getAction(uint256)", &[u256_word(U256::from(id))]).as_ref() {
                    return Ok(data);
                }
            }
            if input
                == call(
                    "guardianVotes(uint8,uint256)",
                    &[u256_word(U256::from(2)), u256_word(U256::from(2))],
                )
                .as_ref()
            {
                return Ok([vec![0u8; 32], word(1)].concat().into());
            }
            return Err(reverted());
        }
        Ok(Bytes::new())
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
    async fn balance(&self, _: Address, _: BlockId) -> BackendResult<U256> {
        Ok(U256::ZERO)
    }
    async fn transaction_count(&self, _: Address, _: BlockId) -> BackendResult<u64> {
        Ok(0)
    }
    async fn code(&self, address: Address, _: BlockId) -> BackendResult<Bytes> {
        Ok(if address == ACCOUNT {
            let mut code = vec![0xef, 0x01, 0x00];
            code.extend_from_slice(SMART_ACCOUNT_IMPL.as_slice());
            code.into()
        } else if address == TOKEN {
            Bytes::from_static(&[0x60, 0x00])
        } else {
            Bytes::new()
        })
    }
    async fn storage_at(&self, _: Address, _: B256, _: BlockId) -> BackendResult<B256> {
        Ok(B256::ZERO)
    }
    async fn call(&self, request: TransactionRequest, block: BlockId) -> BackendResult<Bytes> {
        assert_eq!(
            block,
            BlockId::Number(BlockNumberOrTag::Number(HEAD)),
            "views are pinned to one block number"
        );
        assert!(
            request.gas.is_some_and(|g| g <= 300_000),
            "views are gas-bounded"
        );
        let to = request.to.and_then(|k| k.to().copied()).unwrap_or_default();
        let input = request.input.input().cloned().unwrap_or_default();
        self.view(to, &input)
    }
    async fn estimate_gas(&self, _: TransactionRequest, _: BlockId) -> BackendResult<u64> {
        Ok(21_000)
    }
    async fn send_raw_transaction(&self, _: Bytes) -> BackendResult<B256> {
        Err(RpcBackendError::Unsupported("send".into()))
    }
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        _: bool,
    ) -> BackendResult<Option<RpcBlock>> {
        Ok(match number {
            BlockNumberOrTag::Latest => Some(self.block(HEAD)),
            BlockNumberOrTag::Number(n) if n <= HEAD => Some(self.block(n)),
            _ => None,
        })
    }
    async fn block_by_hash(&self, _: B256, _: bool) -> BackendResult<Option<RpcBlock>> {
        Ok(None)
    }
    async fn block_transaction_count(&self, _: BlockId) -> BackendResult<Option<u64>> {
        Ok(None)
    }
    async fn transaction_by_hash(&self, _: B256) -> BackendResult<Option<RpcTransaction>> {
        Ok(None)
    }
    async fn transaction_receipt(&self, _: B256) -> BackendResult<Option<RpcReceipt>> {
        Ok(None)
    }
    async fn block_receipts(&self, _: BlockId) -> BackendResult<Option<Vec<RpcReceipt>>> {
        Ok(None)
    }
    async fn logs(&self, filter: Filter, limit: Option<usize>) -> BackendResult<Vec<RpcLog>> {
        let FilterBlockOption::Range {
            from_block: Some(BlockNumberOrTag::Number(from)),
            to_block: Some(BlockNumberOrTag::Number(to)),
        } = filter.block_option
        else {
            panic!("safety scans use numeric ranges: {filter:?}");
        };
        assert!(from <= to && to <= HEAD);
        assert!(limit.is_some(), "the result cap is passed down");
        let out = self
            .logs
            .iter()
            .filter(|l| {
                let n = l.block_number.unwrap();
                (from..=to).contains(&n) && filter.matches(&l.inner)
            })
            .cloned()
            .collect();
        self.filters.lock().unwrap().push(filter);
        Ok(out)
    }
    async fn gas_price(&self) -> BackendResult<U256> {
        Ok(U256::from(1))
    }
    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256> {
        Ok(U256::from(1))
    }
    async fn next_base_fee(&self) -> BackendResult<u64> {
        Ok(1)
    }
    async fn fee_history(
        &self,
        _: u64,
        _: BlockNumberOrTag,
        _: Option<Vec<f64>>,
    ) -> BackendResult<FeeHistory> {
        Ok(FeeHistory::default())
    }
}

/// Start a server over a fresh mock. Keep the returned handle alive: dropping it stops the server.
async fn start(config: RpcServerConfig) -> (Arc<Mock>, HttpClient, RpcServerHandle) {
    let mut config = config;
    config.http = SocketAddr::from(([127, 0, 0, 1], 0));
    let mock = Arc::new(Mock::new());
    let handle = xdevs_rpc::start_rpc_server(config, mock.clone())
        .await
        .unwrap();
    let client = HttpClientBuilder::default()
        .request_timeout(Duration::from_secs(10))
        .build(format!("http://{}", handle.local_addr))
        .unwrap();
    (mock, client, handle)
}

async fn ok(client: &HttpClient, method: &str, params: Vec<Value>) -> Value {
    client
        .request::<Value, _>(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

async fn err(client: &HttpClient, method: &str, params: Vec<Value>) -> (i32, String) {
    match client.request::<Value, _>(method, params).await {
        Err(ClientError::Call(e)) => (e.code(), e.message().to_string()),
        other => panic!("{method}: expected a JSON-RPC error, got {other:?}"),
    }
}

#[tokio::test]
async fn check_recipient_name_facts_and_advice() {
    let (_, client, _server) = start(RpcServerConfig::default()).await;
    let r = ok(&client, "xdevs_checkRecipient", vec![json!("Alice")]).await;
    assert_eq!(r["address"], json!(ALICE));
    assert_eq!(r["facts"]["resolvedFrom"], "alice.xdevs");
    assert_eq!(r["facts"]["primaryName"], "alice.xdevs");
    assert_eq!(r["name"], "alice.xdevs");
    assert_eq!(r["facts"]["isContract"], false);
    // The mock does not serve xdevs_getEoaLock: the fact is `null`, not an error.
    assert_eq!(r["facts"]["eoaLock"], Value::Null);
    assert_eq!(r["advice"]["firstTime"], Value::Null, "no `from`");
    assert!(r["advice"]["disclaimer"]
        .as_str()
        .unwrap()
        .starts_with("advice, not fact"));
    // Never used (nonce 0, no balance, no code).
    assert_eq!(r["advice"]["flags"][0]["code"], "NEW_ADDRESS");
    assert_eq!(r["warnings"], r["advice"]["flags"]);

    // Smart account recipient (EIP-7702 delegation to 0x…1005).
    let r = ok(&client, "xdevs_checkRecipient", vec![json!(ACCOUNT)]).await;
    assert_eq!(r["facts"]["isSmartAccount"], true);
    assert_eq!(r["facts"]["delegatedTo"], json!(SMART_ACCOUNT_IMPL));
    assert_eq!(r["blockNumber"], "0x64");
}

#[tokio::test]
async fn check_recipient_flags_lookalikes_and_poisoning() {
    let (_, client, _server) = start(RpcServerConfig::default()).await;
    let r = ok(
        &client,
        "xdevs_checkRecipient",
        vec![json!(OWNER), json!(lookalike())],
    )
    .await;
    assert_eq!(r["firstTime"], true);
    assert_eq!(r["lookAlikeOf"], json!([past()]));
    let codes: Vec<&str> = r["advice"]["flags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"LOOKALIKE_RECIPIENT"), "{codes:?}");
    assert!(codes.contains(&"POISONING_PATTERN"), "{codes:?}");
    assert!(codes.contains(&"FIRST_TIME_RECIPIENT"), "{codes:?}");
    assert_eq!(r["advice"]["flags"][0]["severity"], "high");
    assert_eq!(r["advice"]["history"]["fromBlock"], 0);
    assert_eq!(r["advice"]["history"]["toBlock"], HEAD);
    assert_eq!(r["advice"]["history"]["complete"], true);

    // Paying a known recipient: not first time, no look-alike.
    let r = ok(
        &client,
        "xdevs_checkRecipient",
        vec![json!(OWNER), json!(past()), json!("latest")],
    )
    .await;
    assert_eq!(r["firstTime"], false);
    assert_eq!(r["lookAlikeOf"], json!([]));
}

#[tokio::test]
async fn check_recipient_names_errors() {
    let (_, client, _server) = start(RpcServerConfig::default()).await;
    // `a1ice` has the skeleton of the registered `alice`: not registrable, not resolvable.
    let (code, msg) = err(&client, "xdevs_checkRecipient", vec![json!("a1ice.xdevs")]).await;
    assert_eq!(code, -32602);
    assert!(msg.contains("LOOKALIKE_NAME"), "{msg}");
    assert!(msg.contains(&ALICE.to_string()), "{msg}");
    let (code, msg) = err(&client, "xdevs_checkRecipient", vec![json!("bob")]).await;
    assert_eq!(code, -32602);
    assert!(
        msg.contains("not registered") && !msg.contains("LOOKALIKE"),
        "{msg}"
    );
    let (code, _) = err(&client, "xdevs_checkRecipient", vec![json!("x")]).await;
    assert_eq!(code, -32602);
    let (code, _) = err(&client, "xdevs_checkRecipient", vec![]).await;
    assert_eq!(code, -32602);

    assert_eq!(
        ok(&client, "xdevs_resolveName", vec![json!("alice")]).await,
        json!(ALICE)
    );
    assert_eq!(
        ok(&client, "xdevs_resolveName", vec![json!("bob.xdevs")]).await,
        Value::Null
    );
    assert_eq!(
        ok(&client, "xdevs_reverseName", vec![json!(ALICE)]).await,
        "alice.xdevs"
    );
    assert_eq!(
        ok(&client, "xdevs_reverseName", vec![json!(OWNER)]).await,
        Value::Null
    );
}

#[tokio::test]
async fn get_approvals_checks_live_allowances() {
    let (mock, client, _server) = start(RpcServerConfig::default()).await;
    let r = ok(
        &client,
        "xdevs_getApprovals",
        vec![json!(OWNER), json!("latest")],
    )
    .await;
    // The revoked Permit2 approval is gone; the ERC-20 one is live and unlimited.
    let items = r.as_array().unwrap();
    assert_eq!(items.len(), 1, "{r}");
    let a = &items[0];
    assert_eq!(a["kind"], "erc20");
    assert_eq!(a["token"], json!(TOKEN));
    assert_eq!(a["spender"], json!(SPENDER));
    assert_eq!(a["amount"], "unlimited");
    assert_eq!(a["unlimited"], true);
    assert_eq!(a["symbol"], "TKN");
    assert_eq!(a["expires"], Value::Null);
    assert_eq!(a["lastUpdatedBlock"], "0x46");

    // `tokens` narrows the scan (Permit2 is always included for those tokens).
    let r = ok(
        &client,
        "xdevs_getApprovals",
        vec![json!(OWNER), json!({"tokens": [ALICE]})],
    )
    .await;
    assert_eq!(r, json!([]));
    let last = mock.filters.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.address.len(), 2);

    // Older than the log range limit: rejected, the indexer is the full-history source.
    let config = RpcServerConfig {
        max_logs_range: 10,
        ..Default::default()
    };
    let (_, small, _small_server) = start(config).await;
    let (code, msg) = err(
        &small,
        "xdevs_getApprovals",
        vec![json!(OWNER), json!({"fromBlock": "0x0"})],
    )
    .await;
    assert_eq!(code, -32005);
    assert!(msg.contains("indexer"), "{msg}");
    // Default window: the last 10 blocks (no approval there).
    assert_eq!(
        ok(&small, "xdevs_getApprovals", vec![json!(OWNER)]).await,
        json!([])
    );
    let (code, _) = err(
        &client,
        "xdevs_getApprovals",
        vec![json!(OWNER), json!({"tokenz": []})],
    )
    .await;
    assert_eq!(code, -32602);
}

#[tokio::test]
async fn get_vault_queue_reads_pending_actions() {
    let (_, client, _server) = start(RpcServerConfig::default()).await;
    let r = ok(&client, "xdevs_getVaultQueue", vec![json!(ACCOUNT)]).await;
    let items = r.as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "executed and invalidated actions are left out: {r}"
    );
    let a = &items[0];
    assert_eq!(a["id"], "0x2");
    assert_eq!(a["account"], json!(ACCOUNT));
    assert_eq!(a["target"], json!(ALICE));
    assert_eq!(a["value"], "0x5");
    assert_eq!(a["data"], "0xbeef");
    assert_eq!(a["status"], "queued");
    assert_eq!(a["executeAfter"], format!("{:#x}", NOW + 3_600));
    assert_eq!(a["cancelVotes"], 1);
    assert_eq!(a["calls"].as_array().unwrap().len(), 1);

    // Not a smart account: empty queue.
    assert_eq!(
        ok(&client, "xdevs_getVaultQueue", vec![json!(OWNER)]).await,
        json!([])
    );
}

#[tokio::test]
async fn simulate_transaction_plumbing() {
    let (_, client, _server) = start(RpcServerConfig::default()).await;
    let tx = json!({"from": OWNER, "to": TOKEN, "data": "0x"});
    // The mock backend has no simulator.
    let (code, msg) = err(&client, "xdevs_simulateTransaction", vec![tx]).await;
    assert_eq!(code, -32601, "{msg}");
    let (code, msg) = err(
        &client,
        "xdevs_simulateTransaction",
        vec![json!({"to": TOKEN})],
    )
    .await;
    assert_eq!(code, -32602);
    assert!(msg.contains("from"), "{msg}");
    // The wire type round-trips.
    let report: SimulationReport = serde_json::from_value(json!({
        "success": true,
        "gasUsed": "0x5208",
        "assetChanges": [],
        "balanceChanges": [],
        "approvals": [],
        "contractsCreated": [],
        "warnings": [{"code": "UNLIMITED_APPROVAL", "severity": "high", "message": "m"}],
        "disclaimer": "advice, not fact",
        "blockNumber": "0x1"
    }))
    .unwrap();
    assert!(report.success && report.vault.is_none());
}
