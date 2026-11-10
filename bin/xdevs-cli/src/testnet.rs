//! `xdevs-cli testnet init`: a local multi-validator network in one directory.
//!
//! ```text
//! DIR/
//!   genesis.json              shared genesis: N equal-power validators + dev accounts
//!   node0/                    validator 0
//!     consensus_key.hex       Ed25519 consensus seed (or consensus_key.json, encrypted)
//!     node_key.json           libp2p identity (stable peer id)
//!     config.toml             `xdevs run --config DIR/node0/config.toml`
//!     data/                   chain + consensus databases (created by the node)
//!   ...
//!   node{N}/                  full nodes (`--full-nodes M`): no consensus key
//! ```
//!
//! Node `k` uses the ports `base + 10k + {0: p2p, 1: gossip, 2: rpc, 3: metrics}`. Every
//! node lists every other node as a consensus persistent peer and a tx-gossip bootnode,
//! both as `/ip4/<host>/tcp/<port>/p2p/<peer id>` (the node key is used on both
//! networks, so both addresses carry the same peer id).

use std::{
    fmt::Write as _,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, ensure, Context};
use xdevs_crypto::{
    ed25519::format_ed25519_public_key, validator_address, ConsensusKeystore, Ed25519Seed, NodeKey,
    ScryptParams,
};
use xdevs_genesis::{Genesis, GenesisValidator};
use xdevs_primitives::hex;
use zeroize::Zeroizing;

use crate::write_new_file;

/// Ports reserved per node (`base + PORTS_PER_NODE * k + offset`).
pub const PORTS_PER_NODE: u16 = 10;
/// Port offsets inside a node's block.
const P2P_OFFSET: u16 = 0;
const GOSSIP_OFFSET: u16 = 1;
const RPC_OFFSET: u16 = 2;
const METRICS_OFFSET: u16 = 3;
/// Environment variable the node reads the consensus keystore password from
/// (`xdevs_node::consensus::KEY_PASSWORD_ENV`).
pub const KEY_PASSWORD_ENV: &str = "XDEVS_KEY_PASSWORD";
/// Upper bound on the number of nodes of a local testnet.
const MAX_NODES: usize = 100;

/// Options of `testnet init`.
#[derive(Debug, Clone)]
pub struct TestnetOptions {
    /// Number of validators (≥ 1).
    pub validators: usize,
    /// Number of extra full nodes.
    pub full_nodes: usize,
    /// Output directory.
    pub out: PathBuf,
    /// EVM chain id.
    pub chain_id: u64,
    /// Target block time.
    pub block_time: Duration,
    /// First port of node 0.
    pub base_port: u16,
    /// Host / IP every node listens on and is reached at.
    pub host: String,
    /// Replace the generated files of an existing testnet in `out`.
    pub force: bool,
    /// Encrypt consensus keys with this password (`None` = plain hex seed, dev only).
    pub password: Option<Zeroizing<String>>,
    /// Where the password came from (written to `[validator] password_file`).
    pub password_file: Option<PathBuf>,
    /// Cheap scrypt parameters for the keystores.
    pub light_kdf: bool,
}

/// One generated node.
#[derive(Debug, Clone)]
pub struct TestnetNode {
    /// `node<k>`.
    pub name: String,
    /// Validator (has a consensus key) or full node.
    pub validator: bool,
    /// Node home directory.
    pub dir: PathBuf,
    /// `config.toml`.
    pub config: PathBuf,
    /// libp2p peer id (both networks).
    pub peer_id: String,
    /// Consensus p2p multiaddr, with `/p2p/<peer id>`.
    pub p2p: String,
    /// Tx-gossip multiaddr, with `/p2p/<peer id>`.
    pub gossip: String,
    /// JSON-RPC HTTP URL.
    pub rpc_url: String,
    /// `[metrics] listen` address.
    pub metrics: SocketAddr,
    /// `ed25519:0x...` consensus public key (validators).
    pub consensus_key: Option<String>,
}

/// Result of `testnet init`.
#[derive(Debug, Clone)]
pub struct Testnet {
    /// Chain id.
    pub chain_id: u64,
    /// Shared `genesis.json`.
    pub genesis: PathBuf,
    /// Generated nodes, validators first.
    pub nodes: Vec<TestnetNode>,
    /// Consensus keys are encrypted keystores.
    pub encrypted: bool,
    /// Password file referenced by the configs, if any.
    pub password_file: Option<PathBuf>,
}

/// Parse a block time such as `2s`, `500ms` or `1.5s` (millisecond precision).
pub fn parse_block_time(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let err = || format!("invalid duration `{s}` (use e.g. 2s, 500ms)");
    let ms = if let Some(ms) = s.strip_suffix("ms") {
        ms.trim().parse::<u64>().map_err(|_| err())?
    } else {
        let secs = s.strip_suffix('s').unwrap_or(s).trim();
        let (int, frac) = secs.split_once('.').unwrap_or((secs, ""));
        if frac.len() > 3 || (int.is_empty() && frac.is_empty()) {
            return Err(err());
        }
        let int: u64 = if int.is_empty() {
            0
        } else {
            int.parse().map_err(|_| err())?
        };
        let frac: u64 = if frac.is_empty() {
            0
        } else {
            format!("{frac:0<3}").parse().map_err(|_| err())?
        };
        int.checked_mul(1000)
            .and_then(|v| v.checked_add(frac))
            .ok_or_else(err)?
    };
    if ms == 0 {
        return Err("block time must be > 0".into());
    }
    Ok(Duration::from_millis(ms))
}

/// A TOML basic string (quoted and escaped).
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_path(p: &Path) -> String {
    toml_str(&p.to_string_lossy())
}

fn toml_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let body: Vec<String> = items.iter().map(|s| format!("  {}", toml_str(s))).collect();
    format!("[\n{},\n]", body.join(",\n"))
}

/// Multiaddr of `host:port` (`/ip4`, `/ip6` or `/dns4`).
fn multiaddr(host: &str, port: u16) -> String {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => format!("/ip4/{ip}/tcp/{port}"),
        Ok(IpAddr::V6(ip)) => format!("/ip6/{ip}/tcp/{port}"),
        Err(_) => format!("/dns4/{host}/tcp/{port}"),
    }
}

/// IP the nodes bind to: `host` itself, or all interfaces for a DNS name.
fn bind_ip(host: &str) -> IpAddr {
    host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Remove what a previous `testnet init` wrote (with `--force`), or refuse a non-empty
/// directory.
fn prepare_out_dir(out: &Path, nodes: usize, force: bool) -> anyhow::Result<()> {
    if out.exists() {
        ensure!(out.is_dir(), "{} is not a directory", out.display());
        let non_empty = std::fs::read_dir(out)
            .with_context(|| format!("reading {}", out.display()))?
            .next()
            .is_some();
        if non_empty {
            if !force {
                bail!(
                    "{} is not empty (use --force to replace a generated testnet)",
                    out.display()
                );
            }
            let genesis = out.join("genesis.json");
            if genesis.exists() {
                std::fs::remove_file(&genesis)
                    .with_context(|| format!("removing {}", genesis.display()))?;
            }
            for k in 0..nodes {
                let dir = out.join(format!("node{k}"));
                if dir.exists() {
                    std::fs::remove_dir_all(&dir)
                        .with_context(|| format!("removing {}", dir.display()))?;
                }
            }
        }
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))
}

struct Planned {
    name: String,
    validator: bool,
    dir: PathBuf,
    consensus_seed: Option<Ed25519Seed>,
    node_key: NodeKey,
    p2p_port: u16,
    gossip_port: u16,
    rpc: SocketAddr,
    metrics: SocketAddr,
}

/// Generate keys, `genesis.json` and one `config.toml` per node.
pub fn init(opts: &TestnetOptions) -> anyhow::Result<Testnet> {
    ensure!(opts.validators >= 1, "--validators must be at least 1");
    let total = opts.validators + opts.full_nodes;
    ensure!(total <= MAX_NODES, "at most {MAX_NODES} nodes");
    let last = u32::from(opts.base_port) + u32::from(PORTS_PER_NODE) * total as u32;
    ensure!(
        opts.base_port > 0 && last <= u32::from(u16::MAX),
        "--base-port {} leaves no room for {total} nodes ({PORTS_PER_NODE} ports each)",
        opts.base_port
    );
    ensure!(!opts.host.trim().is_empty(), "--host is empty");
    let block_time_ms = u64::try_from(opts.block_time.as_millis()).context("block time")?;
    ensure!(block_time_ms > 0, "block time must be > 0");

    let out = std::path::absolute(&opts.out)
        .with_context(|| format!("resolving {}", opts.out.display()))?;
    prepare_out_dir(&out, total, opts.force)?;
    let password_file = opts
        .password_file
        .as_deref()
        .map(std::path::absolute)
        .transpose()
        .context("resolving --password-file")?;
    let params = if opts.light_kdf {
        ScryptParams::LIGHT
    } else {
        ScryptParams::STANDARD
    };
    let ip = bind_ip(&opts.host);

    let planned: Vec<Planned> = (0..total)
        .map(|k| {
            let base = opts.base_port + PORTS_PER_NODE * k as u16;
            let validator = k < opts.validators;
            Planned {
                name: format!("node{k}"),
                validator,
                dir: out.join(format!("node{k}")),
                consensus_seed: validator.then(Ed25519Seed::generate),
                node_key: NodeKey::generate(),
                p2p_port: base + P2P_OFFSET,
                gossip_port: base + GOSSIP_OFFSET,
                rpc: SocketAddr::new(ip, base + RPC_OFFSET),
                metrics: SocketAddr::new(ip, base + METRICS_OFFSET),
            }
        })
        .collect();
    let p2p_addr = |p: &Planned| {
        format!(
            "{}/p2p/{}",
            multiaddr(&opts.host, p.p2p_port),
            p.node_key.peer_id()
        )
    };
    let gossip_addr = |p: &Planned| {
        format!(
            "{}/p2p/{}",
            multiaddr(&opts.host, p.gossip_port),
            p.node_key.peer_id()
        )
    };

    // Shared genesis: the dev allocation (10 Anvil accounts + treasury, XIP-0001 from
    // block 0) plus N equal-power validators.
    let mut genesis = Genesis::dev(opts.chain_id);
    genesis.config.block_time_ms = block_time_ms;
    genesis.validators = planned
        .iter()
        .filter_map(|p| {
            p.consensus_seed.as_ref().map(|s| GenesisValidator {
                name: Some(p.name.clone()),
                consensus_key: s.public_key(),
                power: 1,
                p2p: Some(p2p_addr(p)),
                extra: Default::default(),
            })
        })
        .collect();
    genesis.validate().context("generated genesis")?;
    let genesis_path = out.join("genesis.json");
    std::fs::write(&genesis_path, genesis.to_json_string() + "\n")
        .with_context(|| format!("writing {}", genesis_path.display()))?;

    let mut nodes = Vec::with_capacity(total);
    for (k, p) in planned.iter().enumerate() {
        std::fs::create_dir_all(&p.dir).with_context(|| format!("creating {}", p.dir.display()))?;
        let node_key_file = p.dir.join("node_key.json");
        write_new_file(&node_key_file, &p.node_key.to_json_string(), false)?;

        let mut validator_section = String::new();
        if let Some(seed) = &p.consensus_seed {
            let key_file = match &opts.password {
                Some(pw) => {
                    let path = p.dir.join("consensus_key.json");
                    let ks = ConsensusKeystore::encrypt(seed, pw.as_bytes(), params)?;
                    write_new_file(&path, &ks.to_json_string(), false)?;
                    path
                }
                None => {
                    let path = p.dir.join("consensus_key.hex");
                    let hex = Zeroizing::new(hex::encode_prefixed(seed.as_bytes()));
                    write_new_file(&path, &hex, false)?;
                    path
                }
            };
            let _ = writeln!(
                validator_section,
                "[validator]\nenabled = true\nkey_file = {}",
                toml_path(&key_file)
            );
            match &password_file {
                Some(pf) => {
                    let _ = writeln!(validator_section, "password_file = {}", toml_path(pf));
                }
                None if opts.password.is_some() => {
                    let _ = writeln!(
                        validator_section,
                        "# keystore password: ${KEY_PASSWORD_ENV} (or set password_file)"
                    );
                }
                None => {}
            }
            let _ = writeln!(
                validator_section,
                "# beneficiary of priority fees of the blocks this validator proposes\n\
                 fee_recipient = \"0x0000000000000000000000000000000000000000\""
            );
        } else {
            validator_section.push_str("[validator]\nenabled = false   # full node\n");
        }

        let others: Vec<&Planned> = planned
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k)
            .map(|(_, o)| o)
            .collect();
        let peers: Vec<String> = others.iter().map(|o| p2p_addr(o)).collect();
        let bootnodes: Vec<String> = others.iter().map(|o| gossip_addr(o)).collect();
        let role = if p.validator {
            "validator"
        } else {
            "full node"
        };
        let config = format!(
            "# xDEVS Chain local testnet: {name} ({role}), chain id {chain_id}.\n\
             # Generated by `xdevs-cli testnet init`. DEV / TESTNET ONLY.\n\
             # Run: xdevs run --config {config_path}\n\
             \n\
             [chain]\n\
             genesis = {genesis}\n\
             data_dir = {data}\n\
             \n\
             {validator_section}\
             \n\
             [consensus]\n\
             block_time_ms = {block_time_ms}\n\
             \n\
             [p2p]\n\
             listen = {p2p_listen}\n\
             node_key_file = {node_key}\n\
             persistent_peers = {peers}\n\
             \n\
             [gossip]\n\
             enabled = true\n\
             listen = {gossip_listen}\n\
             bootnodes = {bootnodes}\n\
             \n\
             [rpc]\n\
             http = {rpc}\n\
             \n\
             [metrics]   # Prometheus /metrics (xdevs-metrics)\n\
             enabled = true\n\
             listen = {metrics}\n\
             \n\
             [log]\n\
             level = \"info\"\n",
            name = p.name,
            chain_id = opts.chain_id,
            config_path = p.dir.join("config.toml").display(),
            genesis = toml_path(&genesis_path),
            data = toml_path(&p.dir.join("data")),
            p2p_listen = toml_str(&multiaddr(&ip.to_string(), p.p2p_port)),
            node_key = toml_path(&node_key_file),
            peers = toml_list(&peers),
            gossip_listen = toml_str(&multiaddr(&ip.to_string(), p.gossip_port)),
            bootnodes = toml_list(&bootnodes),
            rpc = toml_str(&p.rpc.to_string()),
            metrics = toml_str(&p.metrics.to_string()),
        );
        let config_path = p.dir.join("config.toml");
        std::fs::write(&config_path, config)
            .with_context(|| format!("writing {}", config_path.display()))?;

        let rpc_host = if opts.host.contains(':') {
            format!("[{}]", opts.host)
        } else {
            opts.host.clone()
        };
        nodes.push(TestnetNode {
            name: p.name.clone(),
            validator: p.validator,
            dir: p.dir.clone(),
            config: config_path,
            peer_id: p.node_key.peer_id(),
            p2p: p2p_addr(p),
            gossip: gossip_addr(p),
            rpc_url: format!("http://{rpc_host}:{}", p.rpc.port()),
            metrics: p.metrics,
            consensus_key: p
                .consensus_seed
                .as_ref()
                .map(|s| format_ed25519_public_key(&s.public_key())),
        });
    }

    Ok(Testnet {
        chain_id: opts.chain_id,
        genesis: genesis_path,
        nodes,
        encrypted: opts.password.is_some(),
        password_file,
    })
}

/// Human-readable summary of a generated testnet.
pub fn summary(t: &Testnet) -> String {
    let validators = t.nodes.iter().filter(|n| n.validator).count();
    let mut s = format!(
        "testnet: chain id {}, {validators} validator(s), {} full node(s)\ngenesis: {}\n",
        t.chain_id,
        t.nodes.len() - validators,
        t.genesis.display()
    );
    for n in &t.nodes {
        let _ = write!(
            s,
            "\n{} ({})\n  home:    {}\n  rpc:     {}\n  metrics: {}\n  peer id: {}\n  \
             p2p:     {}\n  gossip:  {}\n",
            n.name,
            if n.validator {
                "validator"
            } else {
                "full node"
            },
            n.dir.display(),
            n.rpc_url,
            n.metrics,
            n.peer_id,
            n.p2p,
            n.gossip,
        );
        if let Some(key) = &n.consensus_key {
            let pk = xdevs_crypto::ed25519::parse_ed25519_public_key(key).unwrap_or_default();
            let _ = writeln!(
                s,
                "  key:     {key}\n  address: {}",
                hex::encode_prefixed(validator_address(&pk))
            );
        }
        let _ = writeln!(s, "  start:   xdevs run --config {}", n.config.display());
    }
    if t.encrypted && t.password_file.is_none() {
        let _ = writeln!(
            s,
            "\nconsensus keys are encrypted: export {KEY_PASSWORD_ENV}=<password> before `xdevs run`"
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use xdevs_genesis::{DEV_ACCOUNTS, DEV_ACCOUNT_BALANCE};
    use xdevs_node::{NodeConfig, NodeMode};

    fn opts(out: &Path, validators: usize, full_nodes: usize) -> TestnetOptions {
        TestnetOptions {
            validators,
            full_nodes,
            out: out.to_path_buf(),
            chain_id: 7117,
            block_time: Duration::from_millis(1500),
            base_port: 41000,
            host: "127.0.0.1".into(),
            force: false,
            password: None,
            password_file: None,
            light_kdf: true,
        }
    }

    fn port_of(addr: &str) -> u16 {
        let parts: Vec<&str> = addr.split('/').collect();
        let i = parts.iter().position(|p| *p == "tcp").unwrap();
        parts[i + 1].parse().unwrap()
    }

    fn peer_of(addr: &str) -> &str {
        addr.rsplit_once("/p2p/").unwrap().1
    }

    #[test]
    fn block_time_parsing() {
        assert_eq!(parse_block_time("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(
            parse_block_time("500ms").unwrap(),
            Duration::from_millis(500)
        );
        assert_eq!(
            parse_block_time("1.5s").unwrap(),
            Duration::from_millis(1500)
        );
        assert_eq!(parse_block_time("3").unwrap(), Duration::from_secs(3));
        for bad in ["", "0s", "0ms", "1.2345s", "x", "-1s"] {
            assert!(parse_block_time(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn toml_strings_are_escaped() {
        assert_eq!(toml_str(r"C:\a\b"), r#""C:\\a\\b""#);
        assert_eq!(toml_str("a\"b\n"), r#""a\"b\n""#);
        assert_eq!(multiaddr("::1", 5), "/ip6/::1/tcp/5");
        assert_eq!(multiaddr("val1", 5), "/dns4/val1/tcp/5");
    }

    #[test]
    fn generated_files_parse_with_the_node_types() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("net");
        let t = init(&opts(&out, 3, 1)).unwrap();
        assert_eq!(t.nodes.len(), 4);
        summary(&t);

        // One shared genesis: 3 equal-power validators + the dev allocation.
        let genesis = Genesis::from_file(&out.join("genesis.json")).unwrap();
        assert_eq!(genesis.config.chain_id, 7117);
        assert_eq!(genesis.config.block_time_ms, 1500);
        assert_eq!(genesis.config.xip0001_block, Some(0));
        let set = genesis.validator_set().unwrap();
        assert_eq!(set.len(), 3);
        assert!(set.iter().all(|v| v.power == 1));
        for a in DEV_ACCOUNTS {
            assert_eq!(genesis.alloc[&a].balance, DEV_ACCOUNT_BALANCE);
        }
        assert!(genesis.alloc.contains_key(&genesis.config.treasury));

        let mut ports = BTreeSet::new();
        let mut peer_ids = Vec::new();
        for (k, n) in t.nodes.iter().enumerate() {
            let config = NodeConfig::load(&n.config).unwrap();
            config.validate().unwrap();
            assert_eq!(config.chain.genesis.as_deref(), Some(t.genesis.as_path()));
            assert_eq!(config.chain.data_dir, n.dir.join("data"));
            assert_eq!(config.consensus.block_time_ms, 1500);
            let mode = config.mode(true).unwrap();
            assert_eq!(mode == NodeMode::Validator, k < 3, "{}", n.name);
            assert_eq!(mode == NodeMode::FullNode, k == 3, "{}", n.name);

            // Keys: the consensus key is the genesis validator k, the node key gives the
            // advertised peer id.
            if k < 3 {
                let seed = xdevs_node::consensus::load_consensus_key(&config.validator).unwrap();
                let v = &genesis.validators[k];
                assert_eq!(seed.public_key(), v.consensus_key);
                assert_eq!(v.name.as_deref(), Some(n.name.as_str()));
                assert_eq!(v.p2p.as_deref(), Some(n.p2p.as_str()));
            }
            let node_key = NodeKey::load(config.p2p.node_key_file.as_ref().unwrap()).unwrap();
            assert_eq!(node_key.peer_id(), n.peer_id);
            peer_ids.push(n.peer_id.clone());

            // Mutual peers: every other node, with its peer id, on both networks.
            let others: Vec<&TestnetNode> = t.nodes.iter().filter(|o| o.name != n.name).collect();
            assert_eq!(
                config.p2p.persistent_peers,
                others.iter().map(|o| o.p2p.clone()).collect::<Vec<_>>()
            );
            assert_eq!(
                config.gossip.network.bootnodes,
                others.iter().map(|o| o.gossip.clone()).collect::<Vec<_>>()
            );
            for (o, (peer, boot)) in others.iter().zip(
                config
                    .p2p
                    .persistent_peers
                    .iter()
                    .zip(&config.gossip.network.bootnodes),
            ) {
                assert_eq!(peer_of(peer), o.peer_id);
                assert_eq!(peer_of(boot), o.peer_id);
            }
            assert_eq!(port_of(&config.p2p.listen), port_of(&n.p2p));
            assert_eq!(port_of(&config.gossip.network.listen), port_of(&n.gossip));
            assert_eq!(n.rpc_url, format!("http://{}", config.rpc.http));

            // [metrics] is read by xdevs-metrics, not by NodeConfig.
            let raw: toml::Value =
                toml::from_str(&std::fs::read_to_string(&n.config).unwrap()).unwrap();
            let section: xdevs_metrics::MetricsConfig = raw["metrics"].clone().try_into().unwrap();
            assert!(section.enabled);
            let metrics: SocketAddr = section.listen.parse().unwrap();
            assert_eq!(metrics, n.metrics);

            for port in [
                port_of(&config.p2p.listen),
                port_of(&config.gossip.network.listen),
                config.rpc.http.port(),
                metrics.port(),
            ] {
                assert!(ports.insert(port), "port {port} used twice");
            }
        }
        assert_eq!(ports.len(), 16);
        peer_ids.sort();
        peer_ids.dedup();
        assert_eq!(peer_ids.len(), 4, "distinct peer ids");
        assert!(!t.nodes[3].dir.join("consensus_key.hex").exists());
    }

    #[test]
    fn refuses_a_non_empty_dir_unless_forced() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("net");
        let first = init(&opts(&out, 2, 0)).unwrap();
        let err = init(&opts(&out, 2, 0)).unwrap_err();
        assert!(format!("{err:#}").contains("--force"), "{err:#}");
        std::fs::create_dir_all(out.join("node0").join("data")).unwrap();
        let mut forced = opts(&out, 2, 0);
        forced.force = true;
        let second = init(&forced).unwrap();
        assert_ne!(first.nodes[0].peer_id, second.nodes[0].peer_id);
        assert!(
            !out.join("node0").join("data").exists(),
            "stale data removed"
        );

        // Other invalid inputs.
        let mut bad = opts(&dir.path().join("x"), 0, 0);
        assert!(init(&bad).is_err());
        bad.validators = 4;
        bad.base_port = 65500;
        assert!(init(&bad).is_err());
    }

    #[test]
    fn encrypted_keys_with_a_password_file() {
        let dir = tempfile::tempdir().unwrap();
        let pw = dir.path().join("pw");
        std::fs::write(&pw, "s3cret\n").unwrap();
        let mut o = opts(&dir.path().join("net"), 2, 0);
        o.password = Some(Zeroizing::new("s3cret".into()));
        o.password_file = Some(pw.clone());
        let t = init(&o).unwrap();
        let genesis = Genesis::from_file(&t.genesis).unwrap();
        for (k, n) in t.nodes.iter().enumerate() {
            let config = NodeConfig::load(&n.config).unwrap();
            assert!(n.dir.join("consensus_key.json").exists());
            assert_eq!(
                config.validator.password_file.as_deref(),
                Some(std::path::absolute(&pw).unwrap().as_path())
            );
            let seed = xdevs_node::consensus::load_consensus_key(&config.validator).unwrap();
            assert_eq!(seed.public_key(), genesis.validators[k].consensus_key);
        }
        assert_eq!(KEY_PASSWORD_ENV, xdevs_node::consensus::KEY_PASSWORD_ENV);
    }
}
