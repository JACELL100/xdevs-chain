//! [`LocalNet`]: N validators + M full nodes of a real xDEVS network, in-process
//! (research doc §6.12, WP5).
//!
//! * Fresh deterministic Ed25519 consensus keys (hex seed files), a genesis with
//!   `validators` equal-power validators plus the dev alloc (chain id 7117).
//! * 127.0.0.1 ports picked at creation and **kept** for the life of the net, so a
//!   stopped node restarts on the same addresses its peers dial. Every node lists every
//!   other initial node as a consensus persistent peer and tx-gossip bootnode; a late
//!   joiner ([`LocalNet::add_full_node`]) lists every node that exists when it is added.
//! * One directory per node (data dir, consensus key, libp2p node key) under a temp
//!   root; [`LocalNet::stop`] keeps it, [`LocalNet::restart`] reopens it.
//! * Assertions over JSON-RPC: [`LocalNet::wait_for_height`],
//!   [`LocalNet::assert_no_fork`] (same block hash and state root at every height on
//!   every live node, every stored commit certificate verifies against the validator
//!   set, no safety alert in the logs).
//!
//! Only one `LocalNet` runs at a time per process: [`LocalNet::start`] waits for a
//! process-wide lock, so the tests of one binary do not compete for CPU and skew
//! consensus timing.

use crate::alerts::{init_tracing, safety_alerts, SafetyAlerts};
use anyhow::{anyhow, bail, ensure, Context};
use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    net::TcpListener,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use xdevs_consensus::ValidatorSet;
use xdevs_consensus::{CommitCertificate, ConsensusTimeouts, ValidatorAddress, ValidatorKey};
use xdevs_crypto::{sign_tx_envelope, signer_from_hex, Ed25519Seed};
use xdevs_genesis::{Genesis, GenesisValidator, DEV_PRIVATE_KEYS};
use xdevs_node::{Node, NodeConfig, NodeHandle, NodeMode};
use xdevs_primitives::{
    constants::{DEV_CHAIN_ID, GWEI},
    hex, Address, Bytes, Encodable2718, TxEip1559, TxKind, B256, U256,
};

/// Serialises the nets of one process (see the module docs).
static NET_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// RPC request timeout of the harness clients.
const RPC_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll interval of the `wait_*` helpers.
const POLL: Duration = Duration::from_millis(100);

/// Shape of a [`LocalNet`].
#[derive(Clone, Debug)]
pub struct LocalNetOptions {
    /// Genesis validators (equal power 1), all started.
    pub validators: usize,
    /// Full nodes started with the validators.
    pub full_nodes: usize,
    /// Target block time; the round timeouts scale with it
    /// ([`ConsensusTimeouts::fast_for_tests`]).
    pub block_time_ms: u64,
}

impl Default for LocalNetOptions {
    fn default() -> Self {
        Self {
            validators: 4,
            full_nodes: 0,
            block_time_ms: 400,
        }
    }
}

/// Role of a node in the net.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Genesis validator with a consensus key.
    Validator,
    /// Follows consensus and block sync without a key.
    FullNode,
}

struct Running {
    handle: NodeHandle,
    rpc: HttpClient,
}

/// One node of a [`LocalNet`] (running or stopped).
pub struct NetNode {
    name: String,
    role: Role,
    home: PathBuf,
    config: NodeConfig,
    /// Consensus address (validators).
    address: Option<ValidatorAddress>,
    running: Option<Running>,
    starts: u32,
}

impl NetNode {
    /// `val1`, `val2`, …, `full1`, ….
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Validator or full node.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Node directory (data dir, keys).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The node's config (fixed ports, peers, keys).
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Consensus address (validators only).
    pub fn validator_address(&self) -> Option<ValidatorAddress> {
        self.address
    }

    /// `true` between start and stop.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// How many times the node was started.
    pub fn starts(&self) -> u32 {
        self.starts
    }

    /// The running node's handle.
    pub fn handle(&self) -> Option<&NodeHandle> {
        self.running.as_ref().map(|r| &r.handle)
    }
}

/// An in-process multi-node network (see the module docs).
pub struct LocalNet {
    nodes: Vec<NetNode>,
    genesis_path: PathBuf,
    chain_id: u64,
    set: ValidatorSet,
    block_time_ms: u64,
    alerts_at_start: SafetyAlerts,
    // Dropped last: the directory outlives the nodes, the lock outlives everything.
    root: TempDir,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl std::fmt::Debug for LocalNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalNet")
            .field("root", &self.root.path())
            .field(
                "nodes",
                &self
                    .nodes
                    .iter()
                    .map(|n| (n.name.as_str(), n.is_running()))
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// `n` currently free 127.0.0.1 TCP ports (bound together, so all distinct).
pub fn free_ports(n: usize) -> Vec<u16> {
    let listeners: Vec<_> = (0..n)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port"))
        .collect();
    listeners
        .iter()
        .map(|l| l.local_addr().expect("local addr").port())
        .collect()
}

/// `/ip4/127.0.0.1/tcp/<port>`.
pub fn tcp_addr(port: u16) -> String {
    format!("/ip4/127.0.0.1/tcp/{port}")
}

/// Parse a `0x…` quantity.
fn quantity(v: &Value) -> anyhow::Result<u64> {
    let s = v.as_str().with_context(|| format!("not a quantity: {v}"))?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).with_context(|| format!("quantity {s}"))
}

fn b256(v: &Value) -> anyhow::Result<B256> {
    v.as_str()
        .with_context(|| format!("not a hash: {v}"))?
        .parse()
        .with_context(|| format!("hash {v}"))
}

impl LocalNet {
    /// Create and start a net (waits for any other net of this process to finish).
    pub async fn start(opts: LocalNetOptions) -> anyhow::Result<Self> {
        ensure!(
            opts.validators > 0,
            "a LocalNet needs at least one validator"
        );
        let lock = NET_LOCK.lock().await;
        init_tracing();
        let root = tempfile::Builder::new()
            .prefix("xdevs-localnet")
            .tempdir()?;
        let total = opts.validators + opts.full_nodes;
        let ports = free_ports(2 * total);
        let (p2p, gossip) = ports.split_at(total);

        let seeds: Vec<Ed25519Seed> = (0..opts.validators)
            .map(|i| Ed25519Seed::from_bytes(&[0x40 + i as u8; 32]))
            .collect();
        let mut genesis = Genesis::dev(DEV_CHAIN_ID);
        genesis.validators = seeds
            .iter()
            .enumerate()
            .map(|(i, s)| GenesisValidator {
                name: Some(format!("val{}", i + 1)),
                consensus_key: s.public_key(),
                power: 1,
                p2p: Some(tcp_addr(p2p[i])),
                extra: Default::default(),
            })
            .collect();
        let set = xdevs_node::consensus::genesis_validator_set(&genesis)?
            .context("genesis has validators")?;
        let genesis_path = root.path().join("genesis.json");
        std::fs::write(&genesis_path, genesis.to_json_string())?;

        let mut net = Self {
            nodes: Vec::new(),
            genesis_path,
            chain_id: genesis.config.chain_id,
            set,
            block_time_ms: opts.block_time_ms,
            alerts_at_start: safety_alerts(),
            root,
            _lock: lock,
        };
        let all_p2p: Vec<String> = p2p.iter().map(|p| tcp_addr(*p)).collect();
        let all_gossip: Vec<String> = gossip.iter().map(|p| tcp_addr(*p)).collect();
        for k in 0..total {
            let (role, name, seed) = if k < opts.validators {
                (Role::Validator, format!("val{}", k + 1), Some(&seeds[k]))
            } else {
                (
                    Role::FullNode,
                    format!("full{}", k - opts.validators + 1),
                    None,
                )
            };
            let peers = |all: &[String]| -> Vec<String> {
                all.iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .map(|(_, a)| a.clone())
                    .collect()
            };
            let node = net.make_node(
                name,
                role,
                seed,
                (p2p[k], gossip[k]),
                peers(&all_p2p),
                peers(&all_gossip),
            )?;
            net.nodes.push(node);
        }
        for i in 0..total {
            net.start_node(i).await?;
        }
        Ok(net)
    }

    fn make_node(
        &self,
        name: String,
        role: Role,
        seed: Option<&Ed25519Seed>,
        (p2p, gossip): (u16, u16),
        peers: Vec<String>,
        bootnodes: Vec<String>,
    ) -> anyhow::Result<NetNode> {
        let home = self.root.path().join(&name);
        std::fs::create_dir_all(&home)?;
        let mut config = NodeConfig::default();
        config.chain.genesis = Some(self.genesis_path.clone());
        config.chain.data_dir = home.join("data");
        config.consensus = ConsensusTimeouts::fast_for_tests(self.block_time_ms);
        config.p2p.listen = tcp_addr(p2p);
        config.p2p.persistent_peers = peers;
        // Stable libp2p identity across restarts.
        config.p2p.node_key_file = Some(home.join("node.key"));
        config.gossip.network.listen = tcp_addr(gossip);
        config.gossip.network.bootnodes = bootnodes;
        config.rpc.http = "127.0.0.1:0".parse().expect("socket addr");
        let mut address = None;
        if let Some(seed) = seed {
            let key_file = home.join("consensus.key");
            std::fs::write(&key_file, hex::encode(seed.as_bytes()))?;
            config.validator.enabled = true;
            config.validator.key_file = Some(key_file);
            config.validator.fee_recipient =
                Address::repeat_byte(0xf0u8.wrapping_add(self.nodes.len() as u8));
            address = Some(ValidatorKey(seed.public_key()).address());
        }
        Ok(NetNode {
            name,
            role,
            home,
            config,
            address,
            running: None,
            starts: 0,
        })
    }

    async fn start_node(&mut self, i: usize) -> anyhow::Result<()> {
        let node = &mut self.nodes[i];
        ensure!(node.running.is_none(), "{} is already running", node.name);
        let n = Node::new(node.config.clone()).with_context(|| format!("open {}", node.name))?;
        let expected = match node.role {
            Role::Validator => NodeMode::Validator,
            Role::FullNode => NodeMode::FullNode,
        };
        ensure!(n.mode() == expected, "{}: mode {:?}", node.name, n.mode());
        let handle = n
            .start()
            .await
            .with_context(|| format!("start {}", node.name))?;
        let rpc = HttpClientBuilder::default()
            .request_timeout(RPC_TIMEOUT)
            .build(handle.http_url())?;
        node.running = Some(Running { handle, rpc });
        node.starts += 1;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Controls
    // -----------------------------------------------------------------------

    /// Stop node `i` (graceful shutdown: engine, RPC, gossip; the database is released).
    /// Its directory is kept for [`LocalNet::restart`].
    pub async fn stop(&mut self, i: usize) -> anyhow::Result<()> {
        let node = &mut self.nodes[i];
        let running = node
            .running
            .take()
            .with_context(|| format!("{} is not running", node.name))?;
        drop(running.rpc);
        running.handle.shutdown().await;
        Ok(())
    }

    /// Start a stopped node again from its directory, on the same addresses.
    pub async fn restart(&mut self, i: usize) -> anyhow::Result<()> {
        self.start_node(i).await
    }

    /// Add and start a new full node that peers with every node created so far (the
    /// late joiner of §6.8: it starts at genesis and catches up through block sync).
    /// Returns its index.
    pub async fn add_full_node(&mut self) -> anyhow::Result<usize> {
        let ports = free_ports(2);
        let full = self
            .nodes
            .iter()
            .filter(|n| n.role == Role::FullNode)
            .count();
        let peers = self
            .nodes
            .iter()
            .map(|n| n.config.p2p.listen.clone())
            .collect();
        let bootnodes = self
            .nodes
            .iter()
            .map(|n| n.config.gossip.network.listen.clone())
            .collect();
        let node = self.make_node(
            format!("full{}", full + 1),
            Role::FullNode,
            None,
            (ports[0], ports[1]),
            peers,
            bootnodes,
        )?;
        self.nodes.push(node);
        let i = self.nodes.len() - 1;
        self.start_node(i).await?;
        Ok(i)
    }

    /// Stop every running node.
    pub async fn shutdown(mut self) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].is_running() {
                let _ = self.stop(i).await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Every node (running or not), validators first.
    pub fn nodes(&self) -> &[NetNode] {
        &self.nodes
    }

    /// Node `i`.
    pub fn node(&self, i: usize) -> &NetNode {
        &self.nodes[i]
    }

    /// Indices of the running nodes.
    pub fn running(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|i| self.nodes[*i].is_running())
            .collect()
    }

    /// Chain id of the genesis.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// The (static) validator set.
    pub fn validator_set(&self) -> &ValidatorSet {
        &self.set
    }

    /// Target block time.
    pub fn block_time(&self) -> Duration {
        Duration::from_millis(self.block_time_ms)
    }

    /// Safety alerts logged since this net started.
    pub fn safety_alerts(&self) -> SafetyAlerts {
        safety_alerts().since(&self.alerts_at_start)
    }

    fn rpc(&self, i: usize) -> anyhow::Result<&HttpClient> {
        let node = &self.nodes[i];
        node.running
            .as_ref()
            .map(|r| &r.rpc)
            .with_context(|| format!("{} is not running", node.name))
    }

    // -----------------------------------------------------------------------
    // JSON-RPC
    // -----------------------------------------------------------------------

    /// Raw JSON-RPC call to node `i`.
    pub async fn call(&self, i: usize, method: &str, params: Vec<Value>) -> anyhow::Result<Value> {
        self.rpc(i)?
            .request::<Value, _>(method, params)
            .await
            .map_err(|e| anyhow!("{} {method}: {e}", self.nodes[i].name))
    }

    /// `eth_blockNumber` of node `i`.
    pub async fn head(&self, i: usize) -> anyhow::Result<u64> {
        quantity(&self.call(i, "eth_blockNumber", vec![]).await?)
    }

    /// Head of every node (`None` = stopped).
    pub async fn heads(&self) -> Vec<Option<u64>> {
        let mut out = Vec::new();
        for i in 0..self.nodes.len() {
            out.push(if self.nodes[i].is_running() {
                self.head(i).await.ok()
            } else {
                None
            });
        }
        out
    }

    /// Highest head among the running nodes.
    pub async fn max_head(&self) -> u64 {
        self.heads().await.into_iter().flatten().max().unwrap_or(0)
    }

    /// `eth_getBlockByNumber(h, false)` on node `i` (`Value::Null` if unknown).
    pub async fn block(&self, i: usize, h: u64) -> anyhow::Result<Value> {
        self.call(
            i,
            "eth_getBlockByNumber",
            vec![json!(format!("{h:#x}")), json!(false)],
        )
        .await
    }

    /// The commit certificate node `i` stores for block `h` (decoded from `raw`).
    pub async fn certificate(&self, i: usize, h: u64) -> anyhow::Result<Option<CommitCertificate>> {
        let v = self
            .call(
                i,
                "xdevs_getCommitCertificate",
                vec![json!(format!("{h:#x}"))],
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        let raw = hex::decode(v["raw"].as_str().context("certificate without raw")?)?;
        let cert = CommitCertificate::from_bytes(&raw)
            .map_err(|e| anyhow!("certificate of block {h}: {e:?}"))?;
        Ok(Some(cert))
    }

    /// Send a signed raw transaction (`0x…` 2718 bytes) to node `i`.
    pub async fn send_raw(&self, i: usize, raw: &[u8]) -> anyhow::Result<B256> {
        let v = self
            .call(
                i,
                "eth_sendRawTransaction",
                vec![json!(format!("0x{}", hex::encode(raw)))],
            )
            .await?;
        b256(&v)
    }

    /// Sign an EIP-1559 transfer of `value` wei from dev account `account` (nonce
    /// `nonce`) to `to` and send it to node `i`'s RPC. Returns the tx hash.
    pub async fn send_transfer(
        &self,
        i: usize,
        account: usize,
        nonce: u64,
        to: Address,
        value: U256,
    ) -> anyhow::Result<B256> {
        let signer = signer_from_hex(DEV_PRIVATE_KEYS[account])?;
        let tx = sign_tx_envelope(
            &signer,
            TxEip1559 {
                chain_id: self.chain_id,
                nonce,
                gas_limit: 21_000,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: u128::from(GWEI),
                to: TxKind::Call(to),
                value,
                input: Bytes::new(),
                access_list: Default::default(),
            },
        )?;
        let hash = self.send_raw(i, &tx.encoded_2718()).await?;
        ensure!(hash == *tx.tx_hash(), "RPC returned another tx hash");
        Ok(hash)
    }

    /// Wait until node `i` has a receipt for `hash`; returns it.
    pub async fn wait_receipt(
        &self,
        i: usize,
        hash: B256,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let r = self
                .call(i, "eth_getTransactionReceipt", vec![json!(hash)])
                .await?;
            if !r.is_null() {
                return Ok(r);
            }
            if Instant::now() >= deadline {
                bail!(
                    "{}: no receipt for {hash} within {timeout:?}",
                    self.nodes[i].name
                );
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// `eth_getBalance(addr, latest)` on node `i`.
    pub async fn balance(&self, i: usize, addr: Address) -> anyhow::Result<U256> {
        let v = self
            .call(i, "eth_getBalance", vec![json!(addr), json!("latest")])
            .await?;
        v.as_str()
            .context("balance")?
            .parse()
            .context("balance quantity")
    }

    // -----------------------------------------------------------------------
    // Waits and assertions
    // -----------------------------------------------------------------------

    /// Fail if a running node's consensus engine stopped on its own.
    pub fn check_engines(&self) -> anyhow::Result<()> {
        for n in &self.nodes {
            if let Some(h) = n.handle() {
                ensure!(h.consensus_running(), "{}: consensus engine halted", n.name);
            }
        }
        Ok(())
    }

    /// Wait until every running node has a head ≥ `height`.
    pub async fn wait_for_height(&self, height: u64, timeout: Duration) -> anyhow::Result<()> {
        let nodes = self.running();
        self.wait_nodes_height(&nodes, height, timeout).await
    }

    /// Wait until node `i` has a head ≥ `height`.
    pub async fn wait_node_height(
        &self,
        i: usize,
        height: u64,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.wait_nodes_height(&[i], height, timeout).await
    }

    async fn wait_nodes_height(
        &self,
        nodes: &[usize],
        height: u64,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.check_engines()?;
            let mut heads = Vec::new();
            for &i in nodes {
                heads.push(self.head(i).await?);
            }
            if heads.iter().all(|h| *h >= height) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let names: Vec<_> = nodes.iter().map(|i| self.nodes[*i].name.as_str()).collect();
                bail!("{names:?} did not reach height {height} within {timeout:?}: {heads:?}");
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Wait until every running node has at least `peers` tx-gossip topic peers.
    pub async fn wait_gossip_mesh(&self, peers: usize, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let counts: Vec<usize> = self
                .nodes
                .iter()
                .filter_map(|n| n.handle().map(NodeHandle::gossip_topic_peers))
                .collect();
            if counts.iter().all(|c| *c >= peers) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("tx-gossip mesh incomplete: {counts:?}");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Never a fork: on every running node, every block from genesis to its head has the
    /// same hash and state root as on every other node, links to its parent, and has a
    /// stored commit certificate for exactly that hash that verifies against the
    /// validator set (> 2/3 of the power, valid signatures). Also fails on any safety
    /// alert logged since the net started. Returns the lowest head checked.
    pub async fn assert_no_fork(&self) -> anyhow::Result<u64> {
        let running = self.running();
        ensure!(!running.is_empty(), "no running node");
        // height -> (hash, state root, node that reported it first)
        let mut seen: BTreeMap<u64, (B256, B256, usize)> = BTreeMap::new();
        let mut lowest = u64::MAX;
        for &i in &running {
            let name = &self.nodes[i].name;
            let head = self.head(i).await?;
            lowest = lowest.min(head);
            let mut parent = None;
            for h in 0..=head {
                let block = self.block(i, h).await?;
                ensure!(
                    !block.is_null(),
                    "{name}: block {h} missing below head {head}"
                );
                let hash = b256(&block["hash"])?;
                let root = b256(&block["stateRoot"])?;
                if let Some(p) = parent {
                    ensure!(
                        b256(&block["parentHash"])? == p,
                        "{name}: block {h} does not link to block {}",
                        h - 1
                    );
                }
                parent = Some(hash);
                match seen.get(&h) {
                    Some((ref_hash, ref_root, by)) => {
                        let other = &self.nodes[*by].name;
                        ensure!(
                            *ref_hash == hash,
                            "FORK at height {h}: {other} has {ref_hash}, {name} has {hash}"
                        );
                        ensure!(
                            *ref_root == root,
                            "state root mismatch at height {h}: {other} has {ref_root}, \
                             {name} has {root}"
                        );
                    }
                    None => {
                        seen.insert(h, (hash, root, i));
                    }
                }
                if h == 0 {
                    continue;
                }
                let cert = self
                    .certificate(i, h)
                    .await?
                    .with_context(|| format!("{name}: no commit certificate for block {h}"))?;
                ensure!(
                    cert.block_hash == hash,
                    "{name}: certificate of block {h} is for {}, block is {hash}",
                    cert.block_hash
                );
                cert.verify_at(self.chain_id, h, &self.set).map_err(|e| {
                    anyhow!("{name}: commit certificate of block {h} does not verify: {e:?}")
                })?;
            }
        }
        let alerts = self.safety_alerts();
        ensure!(
            alerts.is_clean(),
            "safety alerts in the node logs: {alerts:?}"
        );
        Ok(lowest)
    }

    /// Heights in `from..=to` whose certificate (on node `i`) carries a signature of
    /// validator `v`.
    pub async fn signed_heights(
        &self,
        i: usize,
        v: usize,
        from: u64,
        to: u64,
    ) -> anyhow::Result<Vec<u64>> {
        let address = self.nodes[v]
            .address
            .with_context(|| format!("{} is not a validator", self.nodes[v].name))?;
        let mut out = Vec::new();
        for h in from..=to {
            if let Some(cert) = self.certificate(i, h).await? {
                if cert.signatures.iter().any(|s| s.validator == address) {
                    out.push(h);
                }
            }
        }
        Ok(out)
    }
}
