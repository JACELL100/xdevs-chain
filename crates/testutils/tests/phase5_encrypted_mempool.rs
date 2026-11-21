//! Phase 5 exit criterion (plan §27): "the encrypted mempool prototype demonstrably
//! prevents a sandwich attack in a test".
//!
//! One in-process [`LocalNet`] (4 validators, 1 s blocks) with the encrypted mempool
//! active from block 1 (a dealer-generated 3-of-4 committee key) and a constant-product
//! AMM (`fixtures/SimpleAmm.sol`). A bot watches the mempool through node 2's RPC (it is
//! handed each new pending tx hash, as `newPendingTransactions` would) and sandwiches
//! every AMM buy it can read: a front-run buy with a higher tip, then a back-run sell
//! with a lower tip.
//!
//! * **Plain transaction**: the bot reads the victim's swap and sandwiches it; the
//!   attacker ends with more tXDS and the victim gets fewer tokens than quoted.
//! * **Encrypted transaction** (`xdevs_sendEncryptedTransaction`): the bot sees only an
//!   envelope to the inbox with opaque calldata; the swap is ordered blind in block N,
//!   decrypted and executed first in block N+1, the victim gets exactly the quote and the
//!   attacker gains nothing. A griefing envelope (garbage payload) pays its gas and is
//!   skipped.
//!
//! Ends with the no-fork check (same block hashes and state roots on every node).

use anyhow::Context;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use xdevs_crypto::{
    encrypted_tx::{self, MempoolKey},
    sign_tx_envelope, signer_from_hex,
    threshold::PublicPoint,
};
use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
use xdevs_primitives::{
    constants::{ENCRYPTED_INBOX_ADDRESS, GWEI},
    hex, keccak256, Address, Bytes, Encodable2718, TxEip1559, TxKind, B256, U256,
};
use xdevs_testutils::{EncryptedMempoolNet, LocalNet, LocalNetOptions};

const BLOCK_MS: u64 = 1_000;
/// Creation bytecode of `fixtures/SimpleAmm.sol` (solc 0.8.38, osaka, optimizer 200).
const AMM_CODE: &str = include_str!("fixtures/SimpleAmm.bin");
const BUY: [u8; 4] = [0xd9, 0x6a, 0x09, 0x4a];
const SELL_ALL: [u8; 4] = [0xa9, 0x16, 0xad, 0x44];
const RESERVE_NATIVE: [u8; 4] = [0xbf, 0x36, 0xb5, 0x36];
const RESERVE_TOKEN: [u8; 4] = [0xf4, 0x32, 0x5d, 0x67];
const TOKEN_BALANCE: [u8; 4] = [0xee, 0xdc, 0x96, 0x6a];

const DEPLOYER: usize = 0;
const VICTIM: usize = 1;
const ATTACKER: usize = 2;
/// The victim talks to node 0, the attacker's bot to node 1.
const VICTIM_NODE: usize = 0;
const BOT_NODE: usize = 1;

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ether(n: u64) -> U256 {
    U256::from(n) * U256::from(10u64).pow(U256::from(18u64))
}

fn quantity(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().expect("quantity").trim_start_matches("0x"), 16)
        .expect("hex quantity")
}

fn calldata(selector: [u8; 4], words: &[U256]) -> Bytes {
    let mut out = selector.to_vec();
    for w in words {
        out.extend_from_slice(&w.to_be_bytes::<32>());
    }
    out.into()
}

#[allow(clippy::too_many_arguments)]
fn sign(
    chain_id: u64,
    account: usize,
    nonce: u64,
    to: TxKind,
    value: U256,
    input: Bytes,
    gas_limit: u64,
    tip_gwei: u64,
) -> (Vec<u8>, B256) {
    let signer = signer_from_hex(DEV_PRIVATE_KEYS[account]).expect("dev key");
    let tx = sign_tx_envelope(
        &signer,
        TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: u128::from((tip_gwei + 50) * GWEI),
            max_priority_fee_per_gas: u128::from(tip_gwei * GWEI),
            to,
            value,
            input,
            access_list: Default::default(),
        },
    )
    .expect("sign");
    (tx.encoded_2718(), *tx.tx_hash())
}

async fn nonce(net: &LocalNet, account: usize) -> anyhow::Result<u64> {
    let v = net
        .call(
            VICTIM_NODE,
            "eth_getTransactionCount",
            vec![json!(DEV_ACCOUNTS[account]), json!("latest")],
        )
        .await?;
    Ok(quantity(&v))
}

async fn read(net: &LocalNet, amm: Address, data: Bytes) -> anyhow::Result<U256> {
    let out = net.eth_call(VICTIM_NODE, amm, data).await?;
    anyhow::ensure!(out.len() >= 32, "short eth_call output");
    Ok(U256::from_be_slice(&out[..32]))
}

async fn tokens(net: &LocalNet, amm: Address, who: Address) -> anyhow::Result<U256> {
    let mut data = TOKEN_BALANCE.to_vec();
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(who.as_slice());
    read(net, amm, data.into()).await
}

/// Tokens a buy of `value` gets at the current reserves, with no trade in between.
async fn quote(net: &LocalNet, amm: Address, value: U256) -> anyhow::Result<U256> {
    let rn = read(net, amm, calldata(RESERVE_NATIVE, &[])).await?;
    let rt = read(net, amm, calldata(RESERVE_TOKEN, &[])).await?;
    Ok(rt * value / (rn + value))
}

/// Wait for the next block, so a tx sent now has most of a block time before the
/// proposer of the block after next builds (the bot needs a few ms to react).
async fn next_block(net: &LocalNet) -> anyhow::Result<u64> {
    let start = net.head(VICTIM_NODE).await?;
    let deadline = Instant::now() + secs(20);
    loop {
        let h = net.head(VICTIM_NODE).await?;
        if h > start {
            return Ok(h);
        }
        anyhow::ensure!(Instant::now() < deadline, "no new block");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// (block, index) of a mined tx.
async fn position(net: &LocalNet, hash: B256) -> anyhow::Result<(u64, u64, bool)> {
    let r = net.wait_receipt(VICTIM_NODE, hash, secs(30)).await?;
    Ok((
        quantity(&r["blockNumber"]),
        quantity(&r["transactionIndex"]),
        r["status"] == json!("0x1"),
    ))
}

/// What the bot does with a pending tx it is told about.
enum BotAction {
    /// Could not read an AMM buy: nothing sent.
    Ignored { to: Option<Address>, input: String },
    /// Sent a front-run and a back-run.
    Sandwich { front: B256, back: B256 },
}

/// The MEV bot: fetch the pending tx by hash through its own node; if it is a readable
/// AMM buy, sandwich it (front-run buy at 10x the victim's tip, back-run sell at half).
async fn bot(net: &LocalNet, amm: Address, victim_tx: B256) -> anyhow::Result<BotAction> {
    let deadline = Instant::now() + secs(5);
    let tx = loop {
        let tx = net
            .call(BOT_NODE, "eth_getTransactionByHash", vec![json!(victim_tx)])
            .await?;
        if !tx.is_null() {
            break tx;
        }
        anyhow::ensure!(Instant::now() < deadline, "the bot never saw {victim_tx}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let to: Option<Address> = tx["to"].as_str().and_then(|s| s.parse().ok());
    let input = tx["input"].as_str().unwrap_or_default().to_owned();
    let readable = to == Some(amm) && input.starts_with(&format!("0x{}", hex::encode(BUY)));
    if !readable {
        return Ok(BotAction::Ignored { to, input });
    }
    let tip = quantity(&tx["maxPriorityFeePerGas"]) / GWEI;
    let n = nonce(net, ATTACKER).await?;
    let chain = net.chain_id();
    let (front_raw, front) = sign(
        chain,
        ATTACKER,
        n,
        TxKind::Call(amm),
        ether(300),
        calldata(BUY, &[U256::ZERO]),
        200_000,
        tip * 10,
    );
    let (back_raw, back) = sign(
        chain,
        ATTACKER,
        n + 1,
        TxKind::Call(amm),
        U256::ZERO,
        calldata(SELL_ALL, &[U256::ZERO]),
        200_000,
        (tip / 2).max(1),
    );
    net.send_raw(BOT_NODE, &front_raw).await?;
    net.send_raw(BOT_NODE, &back_raw).await?;
    Ok(BotAction::Sandwich { front, back })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn encrypted_mempool_prevents_a_sandwich() -> anyhow::Result<()> {
    // Node logs of the encrypted-mempool path are printed if the test fails.
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn,xdevs::encrypted=debug,xdevs::app=info");
    }
    let net = LocalNet::start(LocalNetOptions {
        validators: 4,
        full_nodes: 0,
        block_time_ms: BLOCK_MS,
        staking: None,
        encrypted_mempool: Some(EncryptedMempoolNet {
            activation: 1,
            threshold: 3,
        }),
        parallel_validators: 0,
        local_fee_markets_block: None,
        validator_power: None,
        proposer_rotation_block: None,
    })
    .await?;
    net.wait_for_height(2, secs(60)).await?;
    // Every node gossips with at least two others (txs and decryption shares).
    net.wait_gossip_mesh(2, secs(30)).await?;
    let chain = net.chain_id();

    // --- The committee key, as a client gets it ------------------------------------
    let info = net
        .call(VICTIM_NODE, "xdevs_getMempoolEncryptionKey", vec![])
        .await?;
    assert_eq!(info["active"], json!(true), "{info}");
    assert_eq!(quantity(&info["threshold"]), 3);
    assert_eq!(quantity(&info["committeeSize"]), 4);
    let inbox: Address = info["inbox"].as_str().unwrap().parse()?;
    assert_eq!(inbox, ENCRYPTED_INBOX_ADDRESS);
    let public_key = PublicPoint::from_slice(&hex::decode(
        info["publicKey"].as_str().unwrap().trim_start_matches("0x"),
    )?)
    .map_err(|e| anyhow::anyhow!("public key: {e}"))?;
    let key_id: B256 = info["keyId"].as_str().unwrap().parse()?;
    assert_eq!(key_id, encrypted_tx::key_id(&public_key));
    let key = MempoolKey {
        threshold: 3,
        public_key,
        key_id,
        verification_keys: Vec::new(),
    };
    let max_inner_gas = quantity(&info["maxInnerGas"]);

    // --- Deploy the AMM: 1,000 tXDS against 1,000,000 tokens ---------------------
    let mut code = hex::decode(AMM_CODE.trim())?;
    code.extend_from_slice(&ether(1_000_000).to_be_bytes::<32>());
    let (raw, deploy) = sign(
        chain,
        DEPLOYER,
        nonce(&net, DEPLOYER).await?,
        TxKind::Create,
        ether(1_000),
        code.into(),
        1_000_000,
        1,
    );
    net.send_raw(VICTIM_NODE, &raw).await?;
    let receipt = net.wait_receipt(VICTIM_NODE, deploy, secs(30)).await?;
    assert_eq!(receipt["status"], json!("0x1"), "{receipt}");
    let amm: Address = receipt["contractAddress"].as_str().unwrap().parse()?;
    let victim = DEV_ACCOUNTS[VICTIM];
    let attacker = DEV_ACCOUNTS[ATTACKER];
    let swap = ether(100);

    // --- 1. Plain transaction: the bot reads it and sandwiches it ------------------
    let mut sandwiched = false;
    for attempt in 1..=3 {
        next_block(&net).await?;
        let expected = quote(&net, amm, swap).await?;
        let tokens_before = tokens(&net, amm, victim).await?;
        let attacker_before = net.balance(VICTIM_NODE, attacker).await?;
        let (raw, victim_tx) = sign(
            chain,
            VICTIM,
            nonce(&net, VICTIM).await?,
            TxKind::Call(amm),
            swap,
            calldata(BUY, &[U256::ZERO]),
            200_000,
            2,
        );
        net.send_raw(VICTIM_NODE, &raw).await?;
        let BotAction::Sandwich { front, back } = bot(&net, amm, victim_tx).await? else {
            panic!("the bot must be able to read a plain AMM buy");
        };
        let (f, v, b) = (
            position(&net, front).await?,
            position(&net, victim_tx).await?,
            position(&net, back).await?,
        );
        let got = tokens(&net, amm, victim).await? - tokens_before;
        let attacker_after = net.balance(VICTIM_NODE, attacker).await?;
        let profit = attacker_after.saturating_sub(attacker_before);
        eprintln!(
            "plain attempt {attempt}: front {f:?}, victim {v:?}, back {b:?}; victim got {got} \
             tokens (quote {expected}); attacker {attacker_before} -> {attacker_after}"
        );
        if (f.0, f.1) < (v.0, v.1) && (v.0, v.1) < (b.0, b.1) {
            assert!(f.2 && v.2 && b.2, "all three succeed");
            assert!(
                got < expected,
                "the victim was front-run: {got} < {expected}"
            );
            assert!(
                attacker_after > attacker_before,
                "the sandwich is profitable after gas"
            );
            eprintln!("plain swap sandwiched: attacker profit {profit} wei");
            sandwiched = true;
            break;
        }
    }
    assert!(
        sandwiched,
        "the plain swap was never sandwiched (bot too slow?)"
    );

    // --- 2. Encrypted transaction: the bot cannot read it ---------------------------
    // Griefing: garbage calldata to the inbox. The RPC refuses it; through
    // eth_sendRawTransaction it is included, pays gas and is never decrypted.
    let attacker_nonce = nonce(&net, ATTACKER).await?;
    let (grief_raw, grief) = sign(
        chain,
        ATTACKER,
        attacker_nonce,
        TxKind::Call(ENCRYPTED_INBOX_ADDRESS),
        U256::ZERO,
        Bytes::from(vec![0x01; 300]),
        100_000,
        1,
    );
    let refused = net
        .call(
            BOT_NODE,
            "xdevs_sendEncryptedTransaction",
            vec![json!(format!("0x{}", hex::encode(&grief_raw)))],
        )
        .await;
    assert!(
        refused.is_err(),
        "a malformed envelope is refused by the RPC"
    );
    net.send_raw(BOT_NODE, &grief_raw).await?;

    next_block(&net).await?;
    let expected = quote(&net, amm, swap).await?;
    let tokens_before = tokens(&net, amm, victim).await?;
    let attacker_before = net.balance(VICTIM_NODE, attacker).await?;
    // The envelope (nonce n) executes in block N, the inner swap (nonce n + 1) at the
    // start of block N + 1.
    let n = nonce(&net, VICTIM).await?;
    let (inner_raw, inner) = sign(
        chain,
        VICTIM,
        n + 1,
        TxKind::Call(amm),
        swap,
        calldata(BUY, &[U256::ZERO]),
        200_000,
        2,
    );
    assert!(200_000 <= max_inner_gas);
    let payload = key.seal(chain, victim, &inner_raw, 200_000).encode();
    let (envelope_raw, envelope) = sign(
        chain,
        VICTIM,
        n,
        TxKind::Call(ENCRYPTED_INBOX_ADDRESS),
        U256::ZERO,
        payload,
        100_000,
        2,
    );
    let sent = net
        .call(
            VICTIM_NODE,
            "xdevs_sendEncryptedTransaction",
            vec![json!(format!("0x{}", hex::encode(&envelope_raw)))],
        )
        .await?;
    assert_eq!(sent.as_str().unwrap().parse::<B256>()?, envelope);
    match bot(&net, amm, envelope).await? {
        BotAction::Ignored { to, input } => {
            assert_eq!(to, Some(ENCRYPTED_INBOX_ADDRESS));
            assert!(
                !input.contains(&hex::encode(BUY)) && !input.contains(&hex::encode(amm)),
                "the envelope leaks neither the target nor the call"
            );
        }
        BotAction::Sandwich { .. } => panic!("the bot read an encrypted swap"),
    }
    // Ordered blind in block N, executed first in block N + 1.
    let (env_block, _, env_ok) = position(&net, envelope).await.context("envelope receipt")?;
    eprintln!("envelope included in block {env_block}");
    let (inner_block, inner_index, inner_ok) = position(&net, inner)
        .await
        .context("decrypted swap receipt")?;
    assert!(env_ok && inner_ok);
    assert_eq!(
        inner_block,
        env_block + 1,
        "decrypted one block after ordering"
    );
    assert_eq!(inner_index, 0, "decrypted txs run first");
    assert_eq!(keccak256(&inner_raw), inner);
    let got = tokens(&net, amm, victim).await? - tokens_before;
    assert_eq!(
        got, expected,
        "the victim gets exactly the quote: no front-run"
    );
    // The attacker never traded; the griefing envelope only cost it gas.
    let (_, _, grief_ok) = position(&net, grief).await?;
    assert!(grief_ok, "the griefing envelope is included and pays");
    let attacker_after = net.balance(VICTIM_NODE, attacker).await?;
    assert!(
        attacker_after <= attacker_before,
        "no profit from the encrypted swap"
    );
    assert_eq!(tokens(&net, amm, attacker).await?, U256::ZERO);
    eprintln!(
        "encrypted swap: envelope in block {env_block}, swap at block {inner_block} index 0; \
         victim got {got} tokens = quote; attacker {attacker_before} -> {attacker_after}"
    );

    // Deterministic roots across validators.
    let head = net.max_head().await;
    net.wait_for_height(head + 2, secs(30)).await?;
    net.assert_no_fork().await?;
    net.shutdown().await;
    Ok(())
}
