//! `proposerRotationBlock` on a [`LocalNet`] shaped like Enclave: 4 validators with equal
//! voting power 12,500,000 each.
//!
//! * Before the activation the original rule applies unchanged: `(height + round) mod
//!   total_power` stays below the first validator's power, so the validator with the
//!   lowest address proposes every block (what Enclave shows since genesis).
//! * From the activation on the proposer rotates: `(height + round) mod 4` over the
//!   validators in address order, so all four propose.
//! * Every header's `miner` is the fee recipient of the validator that proposed it, and
//!   all nodes hold identical block hashes and state roots.

use anyhow::Context as _;
use std::collections::BTreeSet;
use std::time::Duration;
use xdevs_consensus::ValidatorAddress;
use xdevs_primitives::Address;
use xdevs_testutils::{LocalNet, LocalNetOptions};

const ACTIVATION: u64 = 12;
const LAST: u64 = ACTIVATION + 20;

fn fee_recipient(net: &LocalNet, validator: ValidatorAddress) -> Address {
    net.nodes()
        .iter()
        .find(|n| n.validator_address() == Some(validator))
        .map(|n| n.config().validator.fee_recipient)
        .expect("every member runs on a node")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proposers_rotate_from_the_activation_block() -> anyhow::Result<()> {
    let net = LocalNet::start(LocalNetOptions {
        validator_power: Some(12_500_000),
        proposer_rotation_block: Some(ACTIVATION),
        ..Default::default()
    })
    .await?;
    net.wait_for_height(LAST, Duration::from_secs(120)).await?;

    let genesis_set = net.validator_set().clone();
    let mut before = BTreeSet::new();
    let mut after = BTreeSet::new();
    for h in 1..=LAST {
        let block = net.block(0, h).await?;
        let miner: Address = block["miner"]
            .as_str()
            .context("miner")?
            .parse()
            .context("miner address")?;
        let cert = net
            .certificate(0, h)
            .await?
            .with_context(|| format!("certificate of block {h}"))?;
        // The decided block was built in the decision round or, re-proposed with a
        // proof-of-lock, in an earlier one.
        let rule = genesis_set
            .clone()
            .with_proposer_rotation(h >= ACTIVATION);
        let proposers: Vec<Address> = (0..=cert.round)
            .map(|r| fee_recipient(&net, rule.select_proposer(h, r).address))
            .collect();
        assert!(
            proposers.contains(&miner),
            "block {h} (round {}): miner {miner} is not the proposer's fee recipient {proposers:?}",
            cert.round
        );
        if h < ACTIVATION {
            before.insert(miner);
        } else {
            after.insert(miner);
        }
    }
    eprintln!("proposers before {before:?}, after {after:?}");
    assert_eq!(before.len(), 1, "unchanged before the activation: {before:?}");
    let lowest = fee_recipient(&net, genesis_set.validators()[0].address);
    assert!(before.contains(&lowest));
    assert_eq!(after.len(), 4, "all four propose after the activation: {after:?}");

    let checked = net.assert_no_fork().await?;
    eprintln!("no fork over {checked} heights on 4 nodes");
    net.shutdown().await;
    Ok(())
}
