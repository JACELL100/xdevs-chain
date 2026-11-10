use std::sync::{Arc, Mutex};

use clap::CommandFactory;
use jsonrpsee::{
    server::{RpcModule, Server, ServerHandle},
    types::ErrorObjectOwned,
};
use serde_json::{json, Value};
use xdevs_crypto::decode_and_recover;
use xdevs_primitives::{keccak256, Bytes, Transaction};

use super::*;

const ANVIL0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const CHAIN: u64 = 7117;
const GWEI: u128 = 1_000_000_000;

fn pw(p: &str) -> PasswordArgs {
    PasswordArgs {
        password: Some(p.into()),
        password_file: None,
    }
}

#[test]
fn cli_definition_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn send_needs_a_key_source() {
    let base = [
        "xdevs-cli",
        "send",
        "--to",
        "0x00000000000000000000000000000000000000aa",
        "--value",
        "1",
    ];
    assert!(Cli::try_parse_from(base).is_err());
    let mut with_pk = base.to_vec();
    with_pk.extend(["--private-key", ANVIL0]);
    assert!(Cli::try_parse_from(&with_pk).is_ok());
    let mut both = with_pk.clone();
    both.extend(["--from", "k.json"]);
    assert!(Cli::try_parse_from(&both).is_err());
    let mut bad_to = vec!["xdevs-cli", "send", "--to", "0xnope", "--value", "1"];
    bad_to.extend(["--private-key", ANVIL0]);
    assert!(Cli::try_parse_from(&bad_to).is_err());
}

#[test]
fn keygen_address_roundtrip_and_no_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys").join("alice.json");
    let addr = cmd_keygen(Some(&path), &pw("secret"), true, false).unwrap();
    assert_eq!(
        cmd_address(&path, &PasswordArgs::default())
            .unwrap()
            .to_string(),
        addr
    );

    // Without the `address` field, the password is needed.
    let mut ks = read_keystore(&path).unwrap();
    ks.address = None;
    let anon = dir.path().join("anon.json");
    write_new_file(&anon, &ks.to_json_string(), false).unwrap();
    assert_eq!(cmd_address(&anon, &pw("secret")).unwrap().to_string(), addr);
    assert!(cmd_address(&anon, &pw("wrong")).is_err());

    // Loading it as a signer.
    let signer = load_signer(Some(&path), None, &pw("secret")).unwrap();
    assert_eq!(signer.address().to_string(), addr);

    // Refuses to overwrite unless forced.
    let err = cmd_keygen(Some(&path), &pw("x"), true, false).unwrap_err();
    assert!(err.to_string().contains("already exists"));
    let addr2 = cmd_keygen(Some(&path), &pw("x"), true, true).unwrap();
    assert_ne!(addr, addr2);
    assert_eq!(cmd_address(&path, &pw("x")).unwrap().to_string(), addr2);
}

#[test]
fn keygen_without_out_prints_key() {
    let out = cmd_keygen(None, &PasswordArgs::default(), true, false).unwrap();
    let key = out
        .lines()
        .nth(1)
        .unwrap()
        .trim_start_matches("private key: ")
        .trim();
    let addr = out
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("address:")
        .trim();
    assert_eq!(signer_from_hex(key).unwrap().address().to_string(), addr);
}

#[test]
fn password_file_first_line() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("pw.txt");
    std::fs::write(&f, "hunter2\r\nignored\n").unwrap();
    let args = PasswordArgs {
        password: None,
        password_file: Some(f),
    };
    assert_eq!(get_password(&args, false).unwrap().as_str(), "hunter2");
}

// ---------------------------------------------------------------------------
// Mock JSON-RPC node
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockState {
    sent: Mutex<Vec<Bytes>>,
    estimate_fails: bool,
}

async fn mock_node(estimate_fails: bool) -> (String, Arc<MockState>, ServerHandle) {
    let state = Arc::new(MockState {
        estimate_fails,
        ..MockState::default()
    });
    let mut m = RpcModule::new(state.clone());
    m.register_method("eth_chainId", |_, _, _| format!("{CHAIN:#x}"))
        .unwrap();
    m.register_method("eth_getTransactionCount", |p, _, _| {
        let (_addr, tag): (Address, String) = p.parse().unwrap();
        assert_eq!(tag, "pending");
        "0x5"
    })
    .unwrap();
    m.register_method("eth_getBlockByNumber", |_, _, _| {
        json!({ "number": "0x10", "baseFeePerGas": "0x3b9aca00" }) // 1 gwei
    })
    .unwrap();
    m.register_method("eth_maxPriorityFeePerGas", |_, _, _| "0x77359400") // 2 gwei
        .unwrap();
    m.register_method("eth_estimateGas", |p, s: &Arc<MockState>, _| {
        let (req,): (Value,) = p.parse().unwrap();
        assert!(req.get("from").is_some() && req.get("to").is_some());
        if s.estimate_fails {
            Err(ErrorObjectOwned::owned(
                -32000,
                "not implemented",
                None::<()>,
            ))
        } else {
            Ok("0x5300")
        }
    })
    .unwrap();
    m.register_method("eth_sendRawTransaction", |p, s: &Arc<MockState>, _| {
        let raw: Bytes = p.one()?;
        let hash = keccak256(&raw);
        s.sent.lock().unwrap().push(raw);
        Ok::<_, ErrorObjectOwned>(hash)
    })
    .unwrap();
    m.register_method(
        "eth_getTransactionReceipt",
        |_, _, _| json!({ "status": "0x1", "blockNumber": "0x11" }),
    )
    .unwrap();
    m.register_method("eth_getBalance", |p, _, _| {
        let (_addr, tag): (Address, String) = p.parse().unwrap();
        assert_eq!(tag, "latest");
        "0x14d1120d7b160000" // 1.5 XDS
    })
    .unwrap();

    let server = Server::builder().build("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", server.local_addr().unwrap());
    (url, state, server.start(m))
}

#[tokio::test]
async fn send_builds_signs_and_submits_eip1559() {
    let (url, state, handle) = mock_node(false).await;
    let rpc = Rpc::new(&url).unwrap();
    let signer = signer_from_hex(ANVIL0).unwrap();
    let to: Address = "0x00000000000000000000000000000000000000aa"
        .parse()
        .unwrap();
    let value = parse_amount("1.5", 18).unwrap();
    let opts = SendOptions {
        wait: true,
        ..SendOptions::default()
    };
    let hash = cmd_send(&rpc, &signer, to, value, &opts).await.unwrap();

    let raw = state.sent.lock().unwrap().pop().unwrap();
    assert_eq!(hash, keccak256(&raw));
    assert_eq!(raw[0], 2, "EIP-1559 envelope");
    let tx = decode_and_recover(&raw, CHAIN).unwrap();
    assert_eq!(tx.sender, signer.address());
    assert_eq!(tx.nonce(), 5);
    assert_eq!(tx.to(), Some(to));
    assert_eq!(tx.value(), value);
    assert_eq!(tx.gas_limit(), 0x5300);
    assert_eq!(tx.max_priority_fee_per_gas(), Some(2 * GWEI));
    assert_eq!(tx.max_fee_per_gas(), 2 * GWEI + 2 * GWEI); // 2 × base + tip
    handle.stop().unwrap();
}

#[tokio::test]
async fn send_overrides_and_estimate_fallback() {
    let (url, state, handle) = mock_node(true).await;
    let rpc = Rpc::new(&url).unwrap();
    let signer = signer_from_hex(ANVIL0).unwrap();
    let opts = SendOptions {
        max_fee: Some("3".into()),
        priority_fee: Some("5".into()), // capped at max fee
        nonce: Some(42),
        ..SendOptions::default()
    };
    cmd_send(&rpc, &signer, Address::ZERO, U256::from(7), &opts)
        .await
        .unwrap();
    let raw = state.sent.lock().unwrap().pop().unwrap();
    let tx = decode_and_recover(&raw, CHAIN).unwrap();
    assert_eq!(tx.nonce(), 42);
    assert_eq!(tx.gas_limit(), TRANSFER_GAS);
    assert_eq!(tx.max_fee_per_gas(), 3 * GWEI);
    assert_eq!(tx.max_priority_fee_per_gas(), Some(3 * GWEI));
    handle.stop().unwrap();
}

#[tokio::test]
async fn balance_and_full_run() {
    let (url, _state, handle) = mock_node(false).await;
    let cli = Cli::try_parse_from([
        "xdevs-cli",
        "balance",
        "0x00000000000000000000000000000000000000aa",
        "--rpc",
        &url,
    ])
    .unwrap();
    assert_eq!(run(cli).await.unwrap(), "1.5 tXDS");
    let cli = Cli::try_parse_from([
        "xdevs-cli",
        "balance",
        "0x00000000000000000000000000000000000000aa",
        "--rpc",
        &url,
        "--wei",
    ])
    .unwrap();
    assert_eq!(run(cli).await.unwrap(), "1500000000000000000");

    let cli = Cli::try_parse_from([
        "xdevs-cli",
        "send",
        "--rpc",
        &url,
        "--private-key",
        ANVIL0,
        "--to",
        "0x00000000000000000000000000000000000000bb",
        "--value",
        "10",
    ])
    .unwrap();
    let out = run(cli).await.unwrap();
    assert!(out.starts_with("0x") && out.len() == 66, "{out}");
    handle.stop().unwrap();
}

#[tokio::test]
async fn rpc_errors_are_reported() {
    let rpc = Rpc::new("http://127.0.0.1:1").unwrap();
    let err = rpc.chain_id().await.unwrap_err().to_string();
    assert!(err.contains("eth_chainId"), "{err}");
}
