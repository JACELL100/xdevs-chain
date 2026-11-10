//! `xdevs` — the xDEVS Chain node binary. **Owner: node agent.**
//!
//! ```text
//! xdevs init --dev [--data-dir .xdevs] [--force]      # write dev genesis.json + config.toml
//! xdevs run --dev [--block-time 2s] [--http 127.0.0.1:8545] [--data-dir .xdevs]
//! xdevs run --config .xdevs/config.toml
//! ```
//! (`xdevs node` is an alias of `xdevs run`.)

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use xdevs_genesis::Genesis;
use xdevs_node::{Node, NodeConfig};
use xdevs_primitives::{constants::DEV_CHAIN_ID, Address};

/// Default node home directory (gitignored).
const DEFAULT_HOME: &str = ".xdevs";
const CONFIG_FILE: &str = "config.toml";
const GENESIS_FILE: &str = "genesis.json";
const DB_DIR: &str = "data";

/// xDEVS Chain node.
#[derive(Debug, Parser)]
#[command(name = "xdevs", version, about = "xDEVS Chain node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write a genesis.json and config.toml into the node home directory.
    Init {
        /// Use the built-in local dev genesis (chain id 7117, 10 funded dev accounts).
        #[arg(long)]
        dev: bool,
        /// Node home directory.
        #[arg(long, default_value = DEFAULT_HOME)]
        data_dir: PathBuf,
        /// Overwrite existing files.
        #[arg(long)]
        force: bool,
    },
    /// Run a node (single-node dev chain in Phase 1).
    #[command(alias = "node")]
    Run(RunArgs),
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// Single-node dev chain (built-in dev genesis unless --genesis / config says otherwise).
    #[arg(long)]
    dev: bool,
    /// Path to config.toml (default: `<data-dir>/config.toml` if it exists).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Path to genesis.json (overrides the config file).
    #[arg(long)]
    genesis: Option<PathBuf>,
    /// Node home directory; the database lives in `<data-dir>/data`.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Dev block interval, e.g. `2s`, `500ms`, `1.5s` (overrides the config file).
    #[arg(long, value_parser = parse_duration)]
    block_time: Option<Duration>,
    /// JSON-RPC listen address (HTTP + WS).
    #[arg(long)]
    http: Option<SocketAddr>,
    /// Beneficiary of produced blocks (receives priority fees).
    #[arg(long)]
    fee_recipient: Option<Address>,
    /// Log filter (e.g. `info`, `debug`, `xdevs::producer=debug`); `RUST_LOG` wins.
    #[arg(long)]
    log: Option<String>,
}

/// Parse `2s`, `500ms`, `1.5s` or a bare number of seconds.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let err = || format!("invalid duration `{s}` (use e.g. 2s, 500ms)");
    if let Some(ms) = s.strip_suffix("ms") {
        return ms
            .trim()
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| err());
    }
    let secs = s.strip_suffix('s').unwrap_or(s).trim();
    // Decimal seconds without floats: split integer and up to 3 fractional digits.
    let (int, frac) = secs.split_once('.').unwrap_or((secs, ""));
    if frac.len() > 3 || (int.is_empty() && frac.is_empty()) {
        return Err(err());
    }
    let int: u64 = if int.is_empty() {
        0
    } else {
        int.parse().map_err(|_| err())?
    };
    let frac_ms: u64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<3}").parse().map_err(|_| err())?
    };
    Ok(Duration::from_millis(int * 1000 + frac_ms))
}

fn init(dev: bool, home: &Path, force: bool) -> anyhow::Result<()> {
    if !dev {
        bail!("only `xdevs init --dev` is supported in Phase 1");
    }
    std::fs::create_dir_all(home).with_context(|| format!("create {}", home.display()))?;
    let genesis_path = home.join(GENESIS_FILE);
    let config_path = home.join(CONFIG_FILE);
    for p in [&genesis_path, &config_path] {
        if p.exists() && !force {
            bail!("{} already exists (use --force to overwrite)", p.display());
        }
    }
    let mut config = NodeConfig::default();
    config.chain.genesis = Some(genesis_path.clone());
    config.chain.data_dir = home.join(DB_DIR);
    config.dev.enabled = true;
    std::fs::write(&genesis_path, Genesis::dev(DEV_CHAIN_ID).to_json_string())?;
    std::fs::write(&config_path, config.to_toml_string()?)?;
    println!("wrote {}", genesis_path.display());
    println!("wrote {}", config_path.display());
    println!(
        "start the node with: xdevs run --config {}",
        config_path.display()
    );
    Ok(())
}

fn build_config(args: RunArgs) -> anyhow::Result<(NodeConfig, Option<String>)> {
    let home = args
        .data_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_HOME));
    let config_path = args.config.clone().or_else(|| {
        let p = home.join(CONFIG_FILE);
        p.exists().then_some(p)
    });
    let mut cfg = match &config_path {
        Some(path) => NodeConfig::load(path)
            .with_context(|| format!("failed to load config {}", path.display()))?,
        None => {
            let mut c = NodeConfig::default();
            c.chain.data_dir = home.join(DB_DIR);
            c
        }
    };
    if !args.dev && config_path.is_none() && args.genesis.is_none() {
        bail!("nothing to run: pass --dev, --config <file> or --genesis <file> (or run `xdevs init --dev`)");
    }
    cfg.dev.enabled |= args.dev;
    if args.genesis.is_some() {
        cfg.chain.genesis = args.genesis;
    }
    if args.data_dir.is_some() {
        cfg.chain.data_dir = home.join(DB_DIR);
    }
    if let Some(bt) = args.block_time {
        if bt.is_zero() {
            bail!("--block-time must be > 0");
        }
        cfg.dev.block_time_ms = Some(bt.as_millis() as u64);
    }
    if let Some(http) = args.http {
        cfg.rpc.http = http;
    }
    if let Some(fr) = args.fee_recipient {
        cfg.dev.fee_recipient = fr;
    }
    Ok((cfg, args.log))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init {
            dev,
            data_dir,
            force,
        } => init(dev, &data_dir, force),
        Command::Run(args) => {
            let (cfg, log) = build_config(args)?;
            let filter = log.unwrap_or_else(|| cfg.log.level.clone());
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
                )
                .init();
            if cfg.log.format != "text" {
                tracing::warn!(format = %cfg.log.format, "only text logs are supported yet");
            }
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting xdevs node");
            let node = Node::new(cfg)?;
            tracing::info!(
                chain_id = node.chain_config().chain_id,
                rpc = %node.config().rpc.http,
                "chain loaded"
            );
            node.run().await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert_eq!(parse_duration("3").unwrap(), Duration::from_secs(3));
        assert_eq!(parse_duration("0.2").unwrap(), Duration::from_millis(200));
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("1.2345s").is_err());
        assert!(parse_duration("s").is_err());
    }

    #[test]
    fn cli_parses_run_and_alias() {
        let cli = Cli::try_parse_from([
            "xdevs",
            "run",
            "--dev",
            "--block-time",
            "200ms",
            "--http",
            "127.0.0.1:0",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected run")
        };
        let dir = tempfile::tempdir().unwrap();
        let args = RunArgs {
            data_dir: Some(dir.path().to_path_buf()),
            ..args
        };
        let (cfg, _) = build_config(args).unwrap();
        assert!(cfg.dev.enabled);
        assert_eq!(cfg.dev.block_time_ms, Some(200));
        assert_eq!(cfg.chain.data_dir, dir.path().join(DB_DIR));
        assert!(Cli::try_parse_from(["xdevs", "node", "--dev"]).is_ok());
    }

    #[test]
    fn init_writes_loadable_files() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        init(true, &home, false).unwrap();
        assert!(init(true, &home, false).is_err(), "refuses to overwrite");
        init(true, &home, true).unwrap();
        let cfg = NodeConfig::load(&home.join(CONFIG_FILE)).unwrap();
        assert_eq!(cfg.chain.data_dir, home.join(DB_DIR));
        let g = Genesis::from_file(cfg.chain.genesis.as_ref().unwrap()).unwrap();
        assert_eq!(g, Genesis::dev(DEV_CHAIN_ID));
    }
}
