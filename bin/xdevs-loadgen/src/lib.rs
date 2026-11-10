//! `xdevs-loadgen`: load generator for xDEVS Chain (plan §23 / §26 "Load", research doc
//! §8 WP6).
//!
//! N funded senders (the 10 dev keys, a key file, or fresh keys funded from a funder)
//! send EIP-1559 txs at a target rate (token bucket) round-robin over one or more RPC
//! endpoints, with local nonce tracking and a bounded unconfirmed window per sender. The
//! mix is native transfers, `transfer()` calls on an embedded ERC-20-style token, and an
//! optional storage-heavy call. Inclusion is detected by following blocks
//! (`eth_getBlockByNumber` + `eth_getBlockReceipts` per block, never per-tx receipt
//! polling); the report has sent / accepted / rejected-by-error / included counts,
//! inclusion-latency percentiles, achieved TPS, blocks seen / empty and receipt statuses.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod accounts;
pub mod contract;
pub mod mix;
pub mod nonce;
pub mod pacing;
pub mod rpc;
pub mod runner;
pub mod stats;

pub use accounts::SenderSource;
pub use mix::{LoadKind, Mix};
pub use runner::{run, Config, ETHER};
pub use stats::Report;
