//! `xdevs-loadgen`: load generator for xDEVS Chain (research doc §8 WP6): N senders at a
//! target TPS against one RPC endpoint, reporting inclusion-latency percentiles.
//!
//! **Owner: WP6.** Day-0 skeleton: the CLI is fixed, the generator is not built yet.

#![forbid(unsafe_code)]

use clap::Parser;

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(name = "xdevs-loadgen", version, about = "xDEVS Chain load generator")]
struct Args {
    /// JSON-RPC endpoint to send transactions to.
    #[arg(long, default_value = "http://127.0.0.1:8545")]
    rpc: String,
    /// Number of concurrent sender accounts.
    #[arg(long, default_value_t = 10)]
    senders: u32,
    /// Target transactions per second (all senders together).
    #[arg(long, default_value_t = 100)]
    tps: u32,
    /// Run time in seconds.
    #[arg(long, default_value_t = 60)]
    duration_secs: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::bail!(
        "xdevs-loadgen is not implemented yet (WP6); args: {} senders at {} tps for {} s against {}",
        args.senders,
        args.tps,
        args.duration_secs,
        args.rpc
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_valid() {
        Args::command().debug_assert();
        let a = Args::parse_from(["xdevs-loadgen", "--tps", "5"]);
        assert_eq!((a.tps, a.senders), (5, 10));
    }
}
