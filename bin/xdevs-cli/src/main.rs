//! `xdevs-cli` — keys and transactions for xDEVS Chain (plan §23, §33.3).
//!
//! ```text
//! xdevs-cli keygen --out alice.json
//! xdevs-cli keygen --consensus --out consensus_key.json      # Ed25519 validator key
//! xdevs-cli keygen --node-key --out node_key.json            # libp2p identity
//! xdevs-cli testnet init --validators 4 --out ./testnet      # local network
//! xdevs-cli address --key alice.json
//! xdevs-cli send --from alice.json --to 0xBob... --value 10 --rpc http://localhost:8545
//! xdevs-cli balance 0xBob... --rpc http://localhost:8545
//! ```
//!
//! Passwords come from `--password`, `--password-file`, the `XDEVS_PASSWORD`
//! environment variable, or a hidden prompt, in that order.

#![forbid(unsafe_code)]

mod rpc;
mod testnet;
mod units;

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};

use alloy_consensus::TxEip1559;
use alloy_rpc_types_eth::TransactionRequest;
use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand};
use xdevs_crypto::{
    ed25519::format_ed25519_public_key, keystore::ScryptParams, sign_tx_envelope, signer_from_hex,
    validator_address, ConsensusKeystore, Ed25519Seed, Keystore, NodeKey, PrivateKeySigner,
};
use xdevs_primitives::{Address, Encodable2718, TxKind, B256, U256};
use zeroize::Zeroizing;

use crate::{
    rpc::{receipt_summary, Rpc},
    units::{format_xds, parse_amount},
};

/// Default JSON-RPC endpoint (a local `xdevs node --dev`).
const DEFAULT_RPC: &str = "http://127.0.0.1:8545";
/// Environment variable consulted for keystore passwords.
const PASSWORD_ENV: &str = "XDEVS_PASSWORD";
/// Gas of a plain value transfer.
const TRANSFER_GAS: u64 = 21_000;

/// xDEVS Chain command-line tool.
#[derive(Debug, Parser)]
#[command(
    name = "xdevs-cli",
    version,
    about = "xDEVS Chain CLI: keys, transactions and local testnets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Where to get a keystore password.
#[derive(Debug, Clone, Default, Args)]
struct PasswordArgs {
    /// Keystore password (visible in the process list: prefer the prompt or a file).
    #[arg(long)]
    password: Option<String>,
    /// Read the keystore password from the first line of this file.
    #[arg(long, conflicts_with = "password")]
    password_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate a new key: a secp256k1 account key (default), an Ed25519 validator
    /// consensus key (`--consensus`) or a libp2p node identity (`--node-key`).
    Keygen {
        /// Write an encrypted keystore (Web3 Secret Storage v3) to this path.
        /// Without it the private key is printed (dev only, account keys only).
        #[arg(long, required_if_eq_any = [("consensus", "true"), ("node_key", "true")])]
        out: Option<PathBuf>,
        #[command(flatten)]
        password: PasswordArgs,
        /// Cheap scrypt parameters (geth --lightkdf): fast, for throwaway dev keys.
        #[arg(long)]
        light_kdf: bool,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
        /// Ed25519 validator consensus key (encrypted keystore v3 around the seed).
        #[arg(long, conflicts_with = "node_key")]
        consensus: bool,
        /// libp2p node identity (`[p2p] node_key_file`; unencrypted, owner-only file).
        #[arg(long, conflicts_with_all = ["password", "password_file", "light_kdf"])]
        node_key: bool,
        /// With --consensus: write the seed as plain hex, unencrypted (dev/testnet only).
        #[arg(
            long,
            requires = "consensus",
            conflicts_with_all = ["password", "password_file", "light_kdf"]
        )]
        insecure_hex: bool,
    },
    /// Local multi-node networks.
    #[command(subcommand)]
    Testnet(TestnetCommand),
    /// Print the address of a keystore.
    Address {
        /// Keystore file.
        #[arg(long)]
        key: PathBuf,
        #[command(flatten)]
        password: PasswordArgs,
    },
    /// Sign and send an EIP-1559 value transfer; prints the transaction hash.
    Send {
        /// RPC endpoint.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,
        /// Sender keystore file.
        #[arg(long, required_unless_present = "private_key")]
        from: Option<PathBuf>,
        /// Hex private key of the sender instead of a keystore (dev only).
        #[arg(long, conflicts_with = "from")]
        private_key: Option<String>,
        #[command(flatten)]
        password: PasswordArgs,
        /// Recipient address.
        #[arg(long)]
        to: Address,
        /// Amount in tXDS (decimal, e.g. `1.5`); suffixes `wei` / `gwei` also accepted.
        #[arg(long)]
        value: String,
        /// Gas limit (default: eth_estimateGas, 21000 if that fails).
        #[arg(long)]
        gas_limit: Option<u64>,
        /// Max fee per gas in gwei (default: 2 × latest base fee + priority fee).
        #[arg(long)]
        max_fee: Option<String>,
        /// Max priority fee per gas in gwei (default: eth_maxPriorityFeePerGas).
        #[arg(long)]
        priority_fee: Option<String>,
        /// Nonce (default: pending nonce from the node).
        #[arg(long)]
        nonce: Option<u64>,
        /// Wait for the receipt (up to 60 s) and fail if the transaction reverted.
        #[arg(long)]
        wait: bool,
    },
    /// Print an account balance in tXDS.
    Balance {
        /// Account address.
        address: Address,
        /// RPC endpoint.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,
        /// Block tag or number.
        #[arg(long, default_value = "latest")]
        block: String,
        /// Print the raw wei amount only.
        #[arg(long)]
        wei: bool,
    },
}

/// `testnet` subcommands.
#[derive(Debug, Subcommand)]
enum TestnetCommand {
    /// Write keys, one shared genesis.json and a config.toml per node, ready for
    /// `xdevs run --config DIR/nodeK/config.toml`.
    Init(TestnetInitArgs),
}

/// `testnet init` arguments.
#[derive(Debug, Args)]
struct TestnetInitArgs {
    /// Number of validators (equal voting power).
    #[arg(long)]
    validators: usize,
    /// Output directory (must be empty unless --force).
    #[arg(long)]
    out: PathBuf,
    /// Chain id.
    #[arg(long, default_value_t = 7117)]
    chain_id: u64,
    /// Target block time, e.g. `2s`, `500ms`.
    #[arg(long, default_value = "2s", value_parser = testnet::parse_block_time)]
    block_time: Duration,
    /// Node k uses ports base + 10k (p2p), +1 (gossip), +2 (rpc), +3 (metrics).
    #[arg(long, default_value_t = 30300)]
    base_port: u16,
    /// Host / IP the nodes listen on and reach each other at.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Extra full nodes (no consensus key).
    #[arg(long, default_value_t = 0)]
    full_nodes: usize,
    /// Encrypt the consensus keys with this password (default: plain hex, dev only).
    #[arg(long)]
    password: Option<String>,
    /// Encrypt the consensus keys with the password in this file (the configs point
    /// `[validator] password_file` at it).
    #[arg(long, conflicts_with = "password")]
    password_file: Option<PathBuf>,
    /// Cheap scrypt parameters for the encrypted keys.
    #[arg(long)]
    light_kdf: bool,
    /// Replace the genesis and node directories of an existing testnet in --out.
    #[arg(long)]
    force: bool,
}

// ---------------------------------------------------------------------------
// Passwords & key files
// ---------------------------------------------------------------------------

/// Resolve the password: flag, file, env, then an interactive hidden prompt.
fn get_password(args: &PasswordArgs, confirm: bool) -> anyhow::Result<Zeroizing<String>> {
    if let Some(p) = &args.password {
        return Ok(Zeroizing::new(p.clone()));
    }
    if let Some(path) = &args.password_file {
        let content = Zeroizing::new(
            std::fs::read_to_string(path)
                .with_context(|| format!("reading password file {}", path.display()))?,
        );
        let first = content.lines().next().unwrap_or_default();
        return Ok(Zeroizing::new(first.to_string()));
    }
    if let Ok(p) = std::env::var(PASSWORD_ENV) {
        return Ok(Zeroizing::new(p));
    }
    let pw = Zeroizing::new(rpassword::prompt_password("Keystore password: ")?);
    if confirm {
        let again = Zeroizing::new(rpassword::prompt_password("Repeat password: ")?);
        if *pw != *again {
            bail!("passwords do not match");
        }
        if pw.is_empty() {
            eprintln!("warning: empty password");
        }
    }
    Ok(pw)
}

fn read_keystore(path: &Path) -> anyhow::Result<Keystore> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("reading keystore {}", path.display()))?;
    Keystore::from_json_str(&json).with_context(|| format!("parsing keystore {}", path.display()))
}

/// Write `content` to a new file (owner-only permissions on Unix).
fn write_new_file(path: &Path, content: &str, force: bool) -> anyhow::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory {}", dir.display()))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path).with_context(|| {
        if path.exists() && !force {
            format!(
                "{} already exists (use --force to overwrite)",
                path.display()
            )
        } else {
            format!("creating {}", path.display())
        }
    })?;
    file.write_all(content.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn load_signer(
    from: Option<&Path>,
    private_key: Option<&str>,
    password: &PasswordArgs,
) -> anyhow::Result<PrivateKeySigner> {
    match (from, private_key) {
        (_, Some(pk)) => signer_from_hex(pk).context("invalid --private-key"),
        (Some(path), None) => {
            let ks = read_keystore(path)?;
            let pw = get_password(password, false)?;
            ks.decrypt(pw.as_bytes())
                .with_context(|| format!("decrypting {}", path.display()))
        }
        (None, None) => bail!("either --from <keystore> or --private-key is required"),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_keygen(
    out: Option<&Path>,
    password: &PasswordArgs,
    light_kdf: bool,
    force: bool,
) -> anyhow::Result<String> {
    let signer = xdevs_crypto::generate_signer();
    let Some(out) = out else {
        eprintln!("warning: unencrypted private key below; use --out <file> for a keystore");
        let key = Zeroizing::new(xdevs_primitives::hex::encode_prefixed(signer.to_bytes().0));
        return Ok(format!(
            "address:     {}\nprivate key: {}",
            signer.address(),
            *key
        ));
    };
    if out.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            out.display()
        );
    }
    let pw = get_password(password, true)?;
    let params = if light_kdf {
        ScryptParams::LIGHT
    } else {
        ScryptParams::STANDARD
    };
    let ks = xdevs_crypto::encrypt_signer(&signer, pw.as_bytes(), params)?;
    write_new_file(out, &ks.to_json_string(), force)?;
    eprintln!("keystore written to {}", out.display());
    Ok(signer.address().to_string())
}

/// Loud warning printed for unencrypted consensus keys.
const INSECURE_KEY_WARNING: &str = "\
WARNING: UNENCRYPTED consensus key(s) written. DEV / TESTNET ONLY:
WARNING: anyone who reads these files can sign as the validator. Use --password or
WARNING: --password-file for anything that holds value.";

fn cmd_keygen_consensus(
    out: &Path,
    password: &PasswordArgs,
    light_kdf: bool,
    insecure_hex: bool,
    force: bool,
) -> anyhow::Result<String> {
    if out.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            out.display()
        );
    }
    let seed = Ed25519Seed::generate();
    if insecure_hex {
        let hex = Zeroizing::new(xdevs_primitives::hex::encode_prefixed(seed.as_bytes()));
        write_new_file(out, &hex, force)?;
        eprintln!("{INSECURE_KEY_WARNING}");
    } else {
        let pw = get_password(password, true)?;
        let params = if light_kdf {
            ScryptParams::LIGHT
        } else {
            ScryptParams::STANDARD
        };
        let ks = ConsensusKeystore::encrypt(&seed, pw.as_bytes(), params)?;
        write_new_file(out, &ks.to_json_string(), force)?;
    }
    eprintln!("consensus key written to {}", out.display());
    let public = seed.public_key();
    Ok(format!(
        "public key:        {}\nvalidator address: {}",
        format_ed25519_public_key(&public),
        xdevs_primitives::hex::encode_prefixed(validator_address(&public))
    ))
}

fn cmd_keygen_node_key(out: &Path, force: bool) -> anyhow::Result<String> {
    let key = NodeKey::generate();
    write_new_file(out, &key.to_json_string(), force)?;
    eprintln!("node key written to {}", out.display());
    Ok(format!("peer id: {}", key.peer_id()))
}

fn cmd_testnet_init(args: TestnetInitArgs) -> anyhow::Result<String> {
    let password = match (&args.password, &args.password_file) {
        (Some(p), _) => Some(Zeroizing::new(p.clone())),
        (None, Some(_)) => Some(get_password(
            &PasswordArgs {
                password: None,
                password_file: args.password_file.clone(),
            },
            false,
        )?),
        (None, None) => None,
    };
    let encrypted = password.is_some();
    let t = testnet::init(&testnet::TestnetOptions {
        validators: args.validators,
        full_nodes: args.full_nodes,
        out: args.out,
        chain_id: args.chain_id,
        block_time: args.block_time,
        base_port: args.base_port,
        host: args.host,
        force: args.force,
        password,
        password_file: args.password_file,
        light_kdf: args.light_kdf,
    })?;
    if !encrypted {
        eprintln!("{INSECURE_KEY_WARNING}");
    }
    Ok(testnet::summary(&t))
}

fn cmd_address(key: &Path, password: &PasswordArgs) -> anyhow::Result<Address> {
    let ks = read_keystore(key)?;
    if let Some(addr) = ks.stored_address() {
        return Ok(addr);
    }
    let pw = get_password(password, false)?;
    Ok(ks.decrypt(pw.as_bytes())?.address())
}

/// Fee and gas choices for [`cmd_send`].
#[derive(Debug, Default, Clone)]
struct SendOptions {
    gas_limit: Option<u64>,
    max_fee: Option<String>,
    priority_fee: Option<String>,
    nonce: Option<u64>,
    wait: bool,
}

fn gwei_amount(s: &str) -> anyhow::Result<u128> {
    u128::try_from(parse_amount(s, 9)?).context("fee too large")
}

/// Build, sign and submit an EIP-1559 transfer. Returns the tx hash.
async fn cmd_send(
    rpc: &Rpc,
    signer: &PrivateKeySigner,
    to: Address,
    value: U256,
    opts: &SendOptions,
) -> anyhow::Result<B256> {
    let from = signer.address();
    let chain_id = rpc.chain_id().await?;
    let nonce = match opts.nonce {
        Some(n) => n,
        None => rpc.pending_nonce(from).await?,
    };
    let priority_fee = match &opts.priority_fee {
        Some(s) => gwei_amount(s)?,
        None => rpc.max_priority_fee().await?,
    };
    let max_fee = match &opts.max_fee {
        Some(s) => gwei_amount(s)?,
        None => rpc
            .latest_base_fee()
            .await?
            .saturating_mul(2)
            .saturating_add(priority_fee),
    };
    let priority_fee = priority_fee.min(max_fee);
    let gas_limit = match opts.gas_limit {
        Some(g) => g,
        None => {
            let req = TransactionRequest::default()
                .from(from)
                .to(to)
                .value(value)
                .max_fee_per_gas(max_fee)
                .max_priority_fee_per_gas(priority_fee);
            match rpc.estimate_gas(&req).await {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("warning: gas estimation failed ({e}); using {TRANSFER_GAS}");
                    TRANSFER_GAS
                }
            }
        }
    };

    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas: max_fee,
        max_priority_fee_per_gas: priority_fee,
        to: TxKind::Call(to),
        value,
        access_list: Default::default(),
        input: Default::default(),
    };
    let envelope = sign_tx_envelope(signer, tx)?;
    let local_hash = *envelope.tx_hash();
    let hash = rpc.send_raw(envelope.encoded_2718().into()).await?;
    if hash != local_hash {
        eprintln!("warning: node returned hash {hash}, expected {local_hash}");
    }
    eprintln!(
        "sent {} tXDS from {from} to {to} (chain {chain_id}, nonce {nonce}, gas {gas_limit})",
        format_xds(value)
    );

    if opts.wait {
        for _ in 0..120 {
            if let Some(receipt) = rpc.receipt(hash).await? {
                let (ok, block) = receipt_summary(&receipt)?;
                let block = block.map(|b| b.to_string()).unwrap_or_else(|| "?".into());
                if !ok {
                    bail!("transaction {hash} reverted in block {block}");
                }
                eprintln!("included in block {block}");
                return Ok(hash);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        bail!("timed out waiting for receipt of {hash}");
    }
    Ok(hash)
}

async fn run(cli: Cli) -> anyhow::Result<String> {
    match cli.command {
        Command::Keygen {
            out,
            password,
            light_kdf,
            force,
            consensus,
            node_key,
            insecure_hex,
        } => match (consensus, node_key, out) {
            (true, _, Some(out)) => {
                cmd_keygen_consensus(&out, &password, light_kdf, insecure_hex, force)
            }
            (_, true, Some(out)) => cmd_keygen_node_key(&out, force),
            (false, false, out) => cmd_keygen(out.as_deref(), &password, light_kdf, force),
            _ => bail!("--out is required for consensus and node keys"),
        },
        Command::Testnet(TestnetCommand::Init(args)) => cmd_testnet_init(args),
        Command::Address { key, password } => Ok(cmd_address(&key, &password)?.to_string()),
        Command::Send {
            rpc,
            from,
            private_key,
            password,
            to,
            value,
            gas_limit,
            max_fee,
            priority_fee,
            nonce,
            wait,
        } => {
            let value = parse_amount(&value, 18).context("invalid --value")?;
            let signer = load_signer(from.as_deref(), private_key.as_deref(), &password)?;
            let rpc = Rpc::new(&rpc)?;
            let opts = SendOptions {
                gas_limit,
                max_fee,
                priority_fee,
                nonce,
                wait,
            };
            Ok(cmd_send(&rpc, &signer, to, value, &opts).await?.to_string())
        }
        Command::Balance {
            address,
            rpc,
            block,
            wei,
        } => {
            let balance = Rpc::new(&rpc)?.balance(address, &block).await?;
            Ok(if wei {
                balance.to_string()
            } else {
                format!("{} tXDS", format_xds(balance))
            })
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let out = run(Cli::parse()).await?;
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests;
