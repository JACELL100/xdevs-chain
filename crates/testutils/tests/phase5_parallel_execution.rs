//! Phase 5: Block-STM parallel execution on a live network (plan §10.5).
//!
//! One in-process [`LocalNet`] with 4 validators: validators 0 and 1 execute blocks with
//! Block-STM (`[execution] parallel = true`, 4 threads), validators 2 and 3
//! sequentially. Every block needs 3 of 4 prevotes, and a validator only prevotes a
//! block whose execution reproduces the header's state root, so the chain only keeps
//! going if both strategies compute identical roots. The load mixes transfers between a
//! few shared recipients with calls of one hot counter contract deployed in the test,
//! for many blocks; at the end every node must have the same block hashes and state
//! roots, the counter must hold the exact number of calls, and the parallel path must
//! actually have been used.

use serde_json::{json, Value};
use std::time::Duration;
use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
use xdevs_genesis::{DEV_ACCOUNTS, DEV_PRIVATE_KEYS};
use xdevs_primitives::{constants::GWEI, Address, Bytes, Encodable2718, TxEip1559, TxKind, U256};
use xdevs_testutils::{LocalNet, LocalNetOptions};

const BLOCK_MS: u64 = 1_000;
const SENDERS: usize = 8;
const ROUNDS: u64 = 15;
/// `slot0 += 1` (runtime), wrapped in a constructor returning it.
const COUNTER_INIT: &[u8] = &[
    0x60, 0x08, 0x60, 0x0a, 0x5f, 0x39, 0x60, 0x08, 0x5f, 0xf3, // return runtime
    0x5f, 0x54, 0x60, 0x01, 0x01, 0x5f, 0x55, 0x00,
];

const fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn quantity(v: &Value) -> anyhow::Result<U256> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("not a quantity: {v}"))?;
    Ok(U256::from_str_radix(s.trim_start_matches("0x"), 16)?)
}

fn sign(chain_id: u64, account: usize, nonce: u64, to: TxKind, value: u64, gas: u64) -> Vec<u8> {
    let input = if to == TxKind::Create {
        Bytes::from_static(COUNTER_INIT)
    } else {
        Bytes::new()
    };
    let signer = signer_from_hex(DEV_PRIVATE_KEYS[account]).expect("dev key");
    sign_tx_envelope(
        &signer,
        TxEip1559 {
            chain_id,
            nonce,
            gas_limit: gas,
            max_fee_per_gas: 50 * u128::from(GWEI),
            max_priority_fee_per_gas: u128::from(nonce % 3) * u128::from(GWEI),
            to,
            value: U256::from(value),
            input,
            access_list: Default::default(),
        },
    )
    .expect("sign")
    .encoded_2718()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_and_sequential_validators_agree() -> anyhow::Result<()> {
    let before = xdevs_execution::parallel::parallel_blocks_executed();
    let net = LocalNet::start(LocalNetOptions {
        validators: 4,
        full_nodes: 0,
        block_time_ms: BLOCK_MS,
        parallel_validators: 2,
        ..Default::default()
    })
    .await?;
    assert!(net.node(0).config().execution.parallel);
    assert!(!net.node(3).config().execution.parallel);
    net.wait_for_height(2, secs(60)).await?;
    net.wait_gossip_mesh(2, secs(30)).await?;
    let chain = net.chain_id();

    let mut nonces = Vec::new();
    for account in DEV_ACCOUNTS.iter().take(SENDERS) {
        let n = net
            .call(
                0,
                "eth_getTransactionCount",
                vec![json!(account), json!("latest")],
            )
            .await?;
        nonces.push(quantity(&n)?.to::<u64>());
    }
    // The hot contract, deployed by account 0.
    let counter = DEV_ACCOUNTS[0].create(nonces[0]);
    let deploy = net
        .send_raw(0, &sign(chain, 0, nonces[0], TxKind::Create, 0, 200_000))
        .await?;
    nonces[0] += 1;
    net.wait_receipt(0, deploy, secs(60)).await?;

    let recipients: Vec<Address> = (1..=4u8).map(Address::with_last_byte).collect();
    let mut counter_calls = 0u64;
    let mut last = deploy;
    for round in 0..ROUNDS {
        for s in 0..SENDERS {
            // Two txs per sender and round: a transfer to a shared recipient and a
            // call of the hot counter (or a second transfer).
            let to = recipients[(s + round as usize) % recipients.len()];
            let raw = sign(
                chain,
                s,
                nonces[s],
                TxKind::Call(to),
                1_000 + s as u64,
                21_000,
            );
            nonces[s] += 1;
            net.send_raw(s % 4, &raw).await?;
            let raw = if !(s + round as usize).is_multiple_of(3) {
                counter_calls += 1;
                sign(chain, s, nonces[s], TxKind::Call(counter), 0, 100_000)
            } else {
                sign(
                    chain,
                    s,
                    nonces[s],
                    TxKind::Call(DEV_ACCOUNTS[9]),
                    1,
                    21_000,
                )
            };
            nonces[s] += 1;
            last = net.send_raw((s + 1) % 4, &raw).await?;
        }
        tokio::time::sleep(Duration::from_millis(BLOCK_MS / 2)).await;
    }
    let receipt = net.wait_receipt(0, last, secs(120)).await?;
    anyhow::ensure!(
        receipt["status"] == json!("0x1"),
        "last tx failed: {receipt}"
    );

    let head = net.max_head().await;
    net.wait_for_height(head + 2, secs(60)).await?;
    let lowest = net.assert_no_fork().await?;

    // Every counter call was executed exactly once, on every node.
    for i in 0..4 {
        let v = net
            .call(
                i,
                "eth_getStorageAt",
                vec![json!(counter), json!("0x0"), json!("latest")],
            )
            .await?;
        assert_eq!(quantity(&v)?, U256::from(counter_calls), "node {i}");
    }
    let parallel = xdevs_execution::parallel::parallel_blocks_executed() - before;
    eprintln!(
        "{lowest} blocks agree on every node; {counter_calls} counter calls; \
         {parallel} blocks executed with Block-STM"
    );
    assert!(parallel > 0, "no block was executed in parallel");
    net.shutdown().await;
    Ok(())
}
