//! `xdevs-loadgen` command line.
//!
//! ```text
//! xdevs-loadgen --rpc http://127.0.0.1:8545[,URL...] --tps 100 --duration 60s \
//!     [--senders 10] [--mix transfer=80,erc20=20] \
//!     [--private-keys FILE | --fund-from KEY] [--report json] [--out report.json]
//! ```
//!
//! Exit status: 0 on success, 1 on setup errors, 2 when `--min-inclusion` is not met or
//! an included tx failed (CI / soak gate), 130 when interrupted.

#![forbid(unsafe_code)]

use clap::{Parser, ValueEnum};
use std::{path::PathBuf, time::Duration};
use xdevs_loadgen::{pacing::parse_duration, run, Config, Mix, SenderSource, ETHER};
use xdevs_primitives::U256;

/// Report format on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReportFormat {
    /// Human summary.
    Human,
    /// JSON (the human summary goes to stderr).
    Json,
}

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(name = "xdevs-loadgen", version, about = "xDEVS Chain load generator")]
struct Args {
    /// JSON-RPC endpoint(s), comma-separated; sends are spread round-robin, setup and
    /// block polling use the first.
    #[arg(long, value_delimiter = ',', default_value = "http://127.0.0.1:8545")]
    rpc: Vec<String>,
    /// Target transactions per second (all senders together).
    #[arg(long, default_value_t = 100.0)]
    tps: f64,
    /// Send-phase length, e.g. `60s`, `5m`, `500ms`.
    #[arg(long, default_value = "60s", value_parser = parse_duration)]
    duration: Duration,
    /// Number of sender accounts.
    #[arg(long, default_value_t = 10)]
    senders: usize,
    /// Tx mix: `transfer=80,erc20=20[,storage=5]`.
    #[arg(long, default_value = "transfer=100")]
    mix: Mix,
    /// Sender private keys, one hex key per line (default: the 10 dev keys of the
    /// standard Anvil / Hardhat mnemonic, funded by the dev genesis).
    #[arg(long, value_name = "FILE", conflicts_with = "fund_from")]
    private_keys: Option<PathBuf>,
    /// Create `--senders` fresh accounts and fund them from this key (hex private key,
    /// or `dev:<i>` / `<i>` for dev key i).
    #[arg(long, value_name = "KEY")]
    fund_from: Option<String>,
    /// tXDS sent to each fresh sender with `--fund-from`.
    #[arg(long, default_value_t = 100)]
    fund_amount: u64,
    /// Fresh storage slots written by each `storage` tx.
    #[arg(long, default_value_t = 8)]
    storage_slots: u64,
    /// Max unconfirmed txs per sender (the node's pool holds at most 64 per sender).
    #[arg(long, default_value_t = 48)]
    window: u64,
    /// Max unconfirmed txs over all senders.
    #[arg(long, default_value_t = 4096)]
    max_inflight: u64,
    /// Max concurrent `eth_sendRawTransaction` calls.
    #[arg(long, default_value_t = 256)]
    concurrency: usize,
    /// Per-request RPC timeout.
    #[arg(long, default_value = "10s", value_parser = parse_duration)]
    timeout: Duration,
    /// Block poll interval.
    #[arg(long, default_value = "100ms", value_parser = parse_duration)]
    poll_interval: Duration,
    /// After the send phase, wait up to this long for accepted txs to be included.
    #[arg(long, default_value = "30s", value_parser = parse_duration)]
    drain: Duration,
    /// Priority fee in gwei (default: `eth_maxPriorityFeePerGas`).
    #[arg(long)]
    tip_gwei: Option<f64>,
    /// Max fee per gas in gwei (default: 2 x current base fee + tip).
    #[arg(long)]
    max_fee_gwei: Option<f64>,
    /// Report format on stdout.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    report: ReportFormat,
    /// Also write the JSON report to this file.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Exit with status 2 if fewer than this fraction of accepted txs were included
    /// (e.g. 0.95), or if any included tx failed.
    #[arg(long)]
    min_inclusion: Option<f64>,
    /// No progress lines on stderr.
    #[arg(long)]
    quiet: bool,
}

fn gwei(x: f64) -> u128 {
    (x * 1e9).round().max(0.0) as u128
}

impl Args {
    fn config(&self) -> Config {
        let source = match (&self.private_keys, &self.fund_from) {
            (Some(p), _) => SenderSource::File(p.clone()),
            (None, Some(k)) => SenderSource::Fund(k.clone()),
            (None, None) => SenderSource::Dev,
        };
        Config {
            rpc: self.rpc.clone(),
            tps: self.tps,
            duration: self.duration,
            senders: self.senders,
            source,
            fund_amount: U256::from(self.fund_amount) * U256::from(ETHER),
            mix: self.mix.clone(),
            storage_slots: self.storage_slots,
            window: self.window,
            max_inflight: self.max_inflight,
            max_concurrent_sends: self.concurrency,
            request_timeout: self.timeout,
            poll_interval: self.poll_interval,
            drain: self.drain,
            tip: self.tip_gwei.map(gwei),
            max_fee: self.max_fee_gwei.map(gwei),
            handle_ctrl_c: true,
            progress: !self.quiet,
        }
    }
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: cannot start the tokio runtime: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    let report = match rt.block_on(run(args.config())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return std::process::ExitCode::from(1);
        }
    };
    match args.report {
        ReportFormat::Human => print!("{}", report.human()),
        ReportFormat::Json => {
            eprint!("{}", report.human());
            println!("{}", report.to_json());
        }
    }
    if let Some(path) = &args.out {
        if let Err(e) = std::fs::write(path, report.to_json()) {
            eprintln!("error: cannot write {}: {e}", path.display());
            return std::process::ExitCode::from(1);
        }
    }
    if report.interrupted {
        return std::process::ExitCode::from(130);
    }
    if let Some(min) = args.min_inclusion {
        if report.inclusion_rate < min || report.receipts_failed > 0 {
            eprintln!(
                "FAIL: inclusion {:.3} (min {min}), {} failed receipts",
                report.inclusion_rate, report.receipts_failed
            );
            return std::process::ExitCode::from(2);
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use xdevs_loadgen::LoadKind;

    #[test]
    fn cli_is_valid() {
        Args::command().debug_assert();
        let a = Args::parse_from(["xdevs-loadgen", "--tps", "5"]);
        assert_eq!((a.tps, a.senders), (5.0, 10));
        assert_eq!(a.config().source, SenderSource::Dev);
    }

    #[test]
    fn cli_full() {
        let a = Args::parse_from([
            "xdevs-loadgen",
            "--rpc",
            "http://a:1,http://b:2",
            "--tps",
            "250",
            "--duration",
            "2m",
            "--mix",
            "transfer=80,erc20=20",
            "--fund-from",
            "dev:0",
            "--senders",
            "40",
            "--report",
            "json",
            "--tip-gwei",
            "1.5",
        ]);
        let c = a.config();
        assert_eq!(c.rpc, vec!["http://a:1", "http://b:2"]);
        assert_eq!(c.duration, Duration::from_secs(120));
        assert_eq!(c.mix.weight(LoadKind::Erc20), 20);
        assert_eq!(c.source, SenderSource::Fund("dev:0".into()));
        assert_eq!(c.tip, Some(1_500_000_000));
        assert_eq!(a.report, ReportFormat::Json);
        assert!(Args::try_parse_from([
            "xdevs-loadgen",
            "--private-keys",
            "k.txt",
            "--fund-from",
            "0"
        ])
        .is_err());
        assert!(Args::try_parse_from(["xdevs-loadgen", "--mix", "swap=1"]).is_err());
    }
}
