//! Phase 5: local fee markets on a live network (plan §15.2).
//!
//! One in-process [`LocalNet`] (4 validators; 2 execute with Block-STM) with
//! `localFeeMarketsBlock = 1`. One account sends a burst of transactions to a "hot"
//! contract that burns all its gas (12M each, two per block = 80% of the 30M block)
//! while another account sends plain transfers. Checked from the RPC and the balances
//! at each block:
//!
//! * `xdevs_getLocalFee(hot)` starts at zero and rises while the contract is congested;
//! * every hot transaction paid `gasUsed × (baseFee + localFee + tip)` with the local
//!   fee reported for its block, i.e. more than the global price;
//! * every unrelated transfer paid exactly `21000 × (baseFee + tip)`;
//! * all nodes agree on every block hash and state root.

use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
use xdevs_primitives::{constants::GWEI, Bytes, Encodable2718, TxEip1559, TxKind, U256};
use xdevs_testutils::{LocalNet, LocalNetOptions};

const BLOCK_MS: u64 = 1_000;
const HOT_TXS: u64 = 10;
const PLAIN_TXS: u64 = 5;
const HOT_GAS: u64 = 12_000_000;
const TIP: u64 = GWEI;
/// Constructor returning the runtime `JUMPDEST PUSH1 0 JUMP` (burns all gas).
const BURNER_INIT: &[u8] = &[
    0x60, 0x04, 0x60, 0x0a, 0x5f, 0x39, 0x60, 0x04, 0x5f, 0xf3, 0x5b, 0x60, 0x00, 0x56,
];
const DEPLOYER: usize = 0;
const HOT_SENDER: usize = 1;
const PLAIN_SENDER: usize = 2;

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn quantity(v: &Value) -> anyhow::Result<U256> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("not a quantity: {v}"))?;
    Ok(U256::from_str_radix(s.trim_start_matches("0x"), 16)?)
}

fn sign(chain_id: u64, account: usize, nonce: u64, to: TxKind, gas: u64) -> Vec<u8> {
    let (input, value) = match to {
        TxKind::Create => (Bytes::from_static(BURNER_INIT), U256::ZERO),
        TxKind::Call(_) if gas == 21_000 => (Bytes::new(), U256::from(1u64)),
        TxKind::Call(_) => (Bytes::new(), U256::ZERO),
    };
    let signer = signer_from_hex(DEV_PRIVATE_KEYS[account]).expect("dev key");
    sign_tx_envelope(
        &signer,
        TxEip1559 {
            chain_id,
            nonce,
            gas_limit: gas,
            max_fee_per_gas: 1_000 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(TIP),
            to,
            value,
            input,
            access_list: Default::default(),
        },
    )
    .expect("sign")
    .encoded_2718()
}

async fn nonce(net: &LocalNet, account: usize) -> anyhow::Result<u64> {
    let v = net
        .call(
            0,
            "eth_getTransactionCount",
            vec![json!(DEV_ACCOUNTS[account]), json!("latest")],
        )
        .await?;
    Ok(quantity(&v)?.to::<u64>())
}

async fn balance_at(net: &LocalNet, account: usize, block: u64) -> anyhow::Result<U256> {
    let v = net
        .call(
            0,
            "eth_getBalance",
            vec![json!(DEV_ACCOUNTS[account]), json!(format!("{block:#x}"))],
        )
        .await?;
    quantity(&v)
}

async fn base_fee_of(net: &LocalNet, block: u64) -> anyhow::Result<U256> {
    quantity(&net.block(0, block).await?["baseFeePerGas"])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hot_contract_fee_rises_unrelated_transfers_pay_global_fee() -> anyhow::Result<()> {
    let net = LocalNet::start(LocalNetOptions {
        validators: 4,
        full_nodes: 0,
        block_time_ms: BLOCK_MS,
        parallel_validators: 2,
        local_fee_markets_block: Some(1),
        ..Default::default()
    })
    .await?;
    net.wait_for_height(2, secs(60)).await?;
    net.wait_gossip_mesh(2, secs(30)).await?;
    let chain = net.chain_id();

    let deploy_nonce = nonce(&net, DEPLOYER).await?;
    let hot = DEV_ACCOUNTS[DEPLOYER].create(deploy_nonce);
    let deploy = net
        .send_raw(
            0,
            &sign(chain, DEPLOYER, deploy_nonce, TxKind::Create, 200_000),
        )
        .await?;
    net.wait_receipt(0, deploy, secs(60)).await?;
    let fee0 = net.call(0, "xdevs_getLocalFee", vec![json!(hot)]).await?;
    assert_eq!(fee0["active"], json!(true));
    assert_eq!(quantity(&fee0["localBaseFee"])?, U256::ZERO);

    let (mut hot_nonce, mut plain_nonce) = (
        nonce(&net, HOT_SENDER).await?,
        nonce(&net, PLAIN_SENDER).await?,
    );
    // One burst: the hot txs fill consecutive blocks (two per block).
    let mut hot_txs = Vec::new();
    let mut plain_txs = Vec::new();
    for _ in 0..HOT_TXS {
        let raw = sign(chain, HOT_SENDER, hot_nonce, TxKind::Call(hot), HOT_GAS);
        hot_txs.push(net.send_raw(0, &raw).await?);
        hot_nonce += 1;
    }
    for _ in 0..PLAIN_TXS {
        let to = TxKind::Call(DEV_ACCOUNTS[9]);
        let raw = sign(chain, PLAIN_SENDER, plain_nonce, to, 21_000);
        plain_txs.push(net.send_raw(1, &raw).await?);
        plain_nonce += 1;
    }
    net.wait_receipt(0, *hot_txs.last().unwrap(), secs(120))
        .await?;
    net.wait_receipt(0, *plain_txs.last().unwrap(), secs(60))
        .await?;

    // Per block: (hot gas, plain txs) of our two senders.
    let mut hot_gas: BTreeMap<u64, u64> = BTreeMap::new();
    let mut plain_count: BTreeMap<u64, u64> = BTreeMap::new();
    for h in &hot_txs {
        let r = net.wait_receipt(0, *h, secs(10)).await?;
        let block = quantity(&r["blockNumber"])?.to::<u64>();
        *hot_gas.entry(block).or_default() += quantity(&r["gasUsed"])?.to::<u64>();
    }
    for h in &plain_txs {
        let r = net.wait_receipt(0, *h, secs(10)).await?;
        let block = quantity(&r["blockNumber"])?.to::<u64>();
        *plain_count.entry(block).or_default() += 1;
    }

    let mut locals = Vec::new();
    for (&block, &gas) in &hot_gas {
        let base = base_fee_of(&net, block).await?;
        let info = net
            .call(
                0,
                "xdevs_getLocalFee",
                vec![json!(hot), json!(format!("{:#x}", block - 1))],
            )
            .await?;
        assert_eq!(quantity(&info["baseFee"])?, base, "block {block}");
        let local = quantity(&info["localBaseFee"])?;
        let paid = balance_at(&net, HOT_SENDER, block - 1).await?
            - balance_at(&net, HOT_SENDER, block).await?;
        assert_eq!(
            paid,
            U256::from(gas) * (base + local + U256::from(TIP)),
            "hot sender, block {block}"
        );
        locals.push(local);
    }
    for (&block, &count) in &plain_count {
        let base = base_fee_of(&net, block).await?;
        let paid = balance_at(&net, PLAIN_SENDER, block - 1).await?
            - balance_at(&net, PLAIN_SENDER, block).await?;
        assert_eq!(
            paid,
            U256::from(count)
                * (U256::from(21_000u64) * (base + U256::from(TIP)) + U256::from(1u64)),
            "plain sender, block {block}"
        );
    }
    eprintln!("local fee of the hot contract per hot block: {locals:?}");
    // The first hot block had no local fee yet; afterwards it is positive and rises
    // (+27.5% per congested block; an idle block in between would decay it by 1/8).
    assert_eq!(locals[0], U256::ZERO);
    let positive: Vec<U256> = locals.iter().copied().filter(|l| !l.is_zero()).collect();
    assert!(positive.len() >= 2, "local fee never rose: {locals:?}");
    assert!(
        positive.iter().max() > positive.first(),
        "local fee did not rise while congested: {locals:?}"
    );
    let now = net.call(0, "xdevs_getLocalFee", vec![json!(hot)]).await?;
    assert_eq!(
        quantity(&now["totalBaseFee"])?,
        quantity(&now["baseFee"])? + quantity(&now["localBaseFee"])?
    );

    let head = net.max_head().await;
    net.wait_for_height(head + 2, secs(60)).await?;
    net.assert_no_fork().await?;
    net.shutdown().await;
    Ok(())
}
