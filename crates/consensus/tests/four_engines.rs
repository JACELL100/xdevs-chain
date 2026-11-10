//! Four in-process `MalachiteEngine`s driving a mock counter `ConsensusApp` (WP1 exit
//! test): they reach height 20+, survive one engine being torn down mid-run and
//! restarted on the same data dir (WAL + consensus.redb recovery, then catch-up through
//! ValueSync), and never decide two different blocks at the same height.
//!
//! Ephemeral 127.0.0.1 ports and temp dirs; each "block" is a counter value (its
//! height) tagged with the proposer and round, so different proposers produce
//! different blocks for the same height.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinHandle;
use xdevs_consensus::{
    BlockOrigin, CommitCertificate, ConsensusApp, ConsensusEngine, ConsensusTimeouts, EngineConfig,
    EngineKeys, MalachiteEngine, Validator, ValidatorKey, ValidatorSet, Validity,
};
use xdevs_primitives::{Block, Bytes, Header, B256};

const CHAIN_ID: u64 = 7117;
const GENESIS: B256 = B256::repeat_byte(0x11);

/// Decisions seen by any node: height -> hash. A second, different hash is a fork.
#[derive(Default)]
struct Ledger {
    decided: Mutex<BTreeMap<u64, B256>>,
    conflicts: AtomicUsize,
}

/// The mock app: an in-memory chain of counter blocks (it outlives engine restarts,
/// like a node's chain DB).
struct CounterApp {
    index: u8,
    set: ValidatorSet,
    chain: Mutex<Vec<(Block, CommitCertificate)>>,
    ledger: Arc<Ledger>,
}

impl CounterApp {
    fn head_of(chain: &[(Block, CommitCertificate)]) -> (u64, B256) {
        chain
            .last()
            .map_or((0, GENESIS), |(b, _)| (b.number(), b.hash()))
    }

    fn height(&self) -> u64 {
        self.chain.lock().unwrap().len() as u64
    }
}

#[async_trait::async_trait]
impl ConsensusApp for CounterApp {
    async fn head(&self) -> anyhow::Result<(u64, B256)> {
        Ok(Self::head_of(&self.chain.lock().unwrap()))
    }

    async fn validator_set(&self, _height: u64) -> anyhow::Result<ValidatorSet> {
        Ok(self.set.clone())
    }

    async fn propose(
        &self,
        height: u64,
        round: u32,
        _deadline: std::time::Instant,
    ) -> anyhow::Result<Block> {
        let (h, parent) = Self::head_of(&self.chain.lock().unwrap());
        anyhow::ensure!(height == h + 1, "propose {height} on head {h}");
        let mut extra = vec![self.index];
        extra.extend_from_slice(&round.to_be_bytes());
        Ok(Block::new(
            Header {
                number: height,
                parent_hash: parent,
                timestamp: height,
                extra_data: Bytes::from(extra),
                ..Default::default()
            },
            vec![],
        ))
    }

    async fn validate(
        &self,
        height: u64,
        block: &Block,
        _origin: BlockOrigin,
    ) -> anyhow::Result<Validity> {
        let (h, parent) = Self::head_of(&self.chain.lock().unwrap());
        Ok(Validity::from_bool(
            height == h + 1
                && block.number() == height
                && block.header.parent_hash == parent
                && block.header.timestamp == height
                && block.header.extra_data.len() == 5,
        ))
    }

    async fn commit(&self, block: Block, cert: CommitCertificate) -> anyhow::Result<()> {
        let height = block.number();
        let hash = block.hash();
        anyhow::ensure!(cert.block_hash == hash, "certificate is for another block");
        cert.verify_at(CHAIN_ID, height, &self.set)?;
        {
            let mut decided = self.ledger.decided.lock().unwrap();
            match decided.get(&height) {
                Some(h) if *h != hash => {
                    self.ledger.conflicts.fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("FORK at height {height}: {h} vs {hash}");
                }
                _ => {
                    decided.insert(height, hash);
                }
            }
        }
        let mut chain = self.chain.lock().unwrap();
        let (h, parent) = Self::head_of(&chain);
        if height == h + 1 {
            anyhow::ensure!(block.header.parent_hash == parent, "wrong parent");
            chain.push((block, cert));
        } else {
            // Idempotent re-commit: keep the certificate with more signatures.
            let (b, c) = chain
                .get_mut((height - 1) as usize)
                .ok_or_else(|| anyhow::anyhow!("commit {height} on head {h}"))?;
            anyhow::ensure!(b.hash() == hash, "re-commit of a different block");
            if cert.signatures.len() > c.signatures.len() {
                *c = cert;
            }
        }
        Ok(())
    }

    async fn decided(&self, height: u64) -> anyhow::Result<Option<(Block, CommitCertificate)>> {
        Ok(height
            .checked_sub(1)
            .and_then(|i| self.chain.lock().unwrap().get(i as usize).cloned()))
    }

    async fn earliest_height(&self) -> anyhow::Result<u64> {
        Ok(1)
    }
}

struct Node {
    config: EngineConfig,
    consensus_seed: [u8; 32],
    node_seed: [u8; 32],
    app: Arc<CounterApp>,
    stop: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Node {
    fn start(&mut self) {
        let (tx, rx) = watch::channel(false);
        let engine = MalachiteEngine::new(
            self.config.clone(),
            EngineKeys::validator(self.consensus_seed, self.node_seed),
        );
        let app: Arc<dyn ConsensusApp> = self.app.clone();
        self.task = Some(tokio::spawn(engine.run(app, rx)));
        self.stop = Some(tx);
    }

    async fn shutdown(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(true);
        }
        if let Some(task) = self.task.take() {
            let res = tokio::time::timeout(Duration::from_secs(30), task)
                .await
                .expect("engine stops within 30 s")
                .expect("engine task did not panic");
            res.expect("engine shut down cleanly");
        }
    }

    fn assert_running(&self, i: usize) {
        if let Some(t) = &self.task {
            assert!(!t.is_finished(), "engine {i} exited unexpectedly");
        }
    }
}

fn free_ports(n: usize) -> Vec<u16> {
    let listeners: Vec<_> = (0..n)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    listeners
        .iter()
        .map(|l| l.local_addr().unwrap().port())
        .collect()
}

async fn wait_until(nodes: &[Node], which: &[usize], height: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        for &i in which {
            nodes[i].assert_running(i);
        }
        if which.iter().all(|&i| nodes[i].app.height() >= height) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn heights(nodes: &[Node]) -> Vec<u64> {
    nodes.iter().map(|n| n.app.height()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_engines_reach_height_20_and_survive_a_restart() {
    const N: usize = 4;
    let dirs: Vec<_> = (0..N).map(|_| tempfile::tempdir().unwrap()).collect();
    let ports = free_ports(N);
    let seeds: Vec<[u8; 32]> = (0..N).map(|i| [i as u8 + 1; 32]).collect();
    let set = ValidatorSet::new(
        seeds
            .iter()
            .map(|s| {
                let pk = ed25519_consensus::SigningKey::from(*s)
                    .verification_key()
                    .to_bytes();
                Validator::new(ValidatorKey::from_bytes(pk).unwrap(), 1)
            })
            .collect(),
    )
    .unwrap();
    let ledger = Arc::new(Ledger::default());

    let mut nodes: Vec<Node> = (0..N)
        .map(|i| {
            let mut config = EngineConfig::new(CHAIN_ID, dirs[i].path());
            config.moniker = format!("wp1-node-{i}");
            config.timeouts = ConsensusTimeouts::fast_for_tests(250);
            config.p2p.listen = format!("/ip4/127.0.0.1/tcp/{}", ports[i]);
            config.p2p.persistent_peers = (0..N)
                .filter(|j| *j != i)
                .map(|j| format!("/ip4/127.0.0.1/tcp/{}", ports[j]))
                .collect();
            Node {
                config,
                consensus_seed: seeds[i],
                node_seed: [100 + i as u8; 32],
                app: Arc::new(CounterApp {
                    index: i as u8,
                    set: set.clone(),
                    chain: Mutex::new(Vec::new()),
                    ledger: ledger.clone(),
                }),
                stop: None,
                task: None,
            }
        })
        .collect();

    for n in &mut nodes {
        n.start();
    }
    let all: Vec<usize> = (0..N).collect();
    assert!(
        wait_until(&nodes, &all, 6, Duration::from_secs(120)).await,
        "4 engines reach height 6: {:?}",
        heights(&nodes)
    );

    // Tear engine 3 down mid-run (no coordination with the app), keep its data dir.
    nodes[3].shutdown().await;
    let at_stop = nodes[3].app.height();
    let others = [0usize, 1, 2];
    assert!(
        wait_until(&nodes, &others, at_stop + 5, Duration::from_secs(120)).await,
        "3 of 4 keep deciding: {:?}",
        heights(&nodes)
    );

    // Restart on the same data dir: WAL + consensus.redb recovery, then ValueSync.
    let behind = heights(&nodes);
    eprintln!("engine 3 stopped at {at_stop}; restarting with heights {behind:?}");
    assert!(
        behind[3] + 5 <= behind[0],
        "engine 3 must restart behind: {behind:?}"
    );
    nodes[3].start();
    assert!(
        wait_until(&nodes, &all, 20, Duration::from_secs(180)).await,
        "all 4 reach height 20 after the restart: {:?}",
        heights(&nodes)
    );

    eprintln!("after restart: {:?}", heights(&nodes));
    for n in &mut nodes {
        n.shutdown().await;
    }

    // Safety: no two different values were ever decided at one height ...
    assert_eq!(ledger.conflicts.load(Ordering::SeqCst), 0, "fork detected");
    // ... and every node holds the same chain prefix with valid certificates.
    let min = nodes.iter().map(|n| n.app.height()).min().unwrap();
    assert!(min >= 20);
    let reference = nodes[0].app.chain.lock().unwrap().clone();
    for n in &nodes {
        let chain = n.app.chain.lock().unwrap();
        let mut parent = GENESIS;
        for (i, (block, cert)) in chain.iter().enumerate() {
            assert_eq!(block.number(), i as u64 + 1);
            assert_eq!(block.header.parent_hash, parent);
            assert_eq!(cert.block_hash, block.hash());
            cert.verify_at(CHAIN_ID, block.number(), &set).unwrap();
            parent = block.hash();
        }
        for h in 0..min as usize {
            assert_eq!(chain[h].0.hash(), reference[h].0.hash(), "height {}", h + 1);
        }
    }
    // Proposers rotated: blocks were built by more than one validator.
    let builders: std::collections::BTreeSet<u8> = reference
        .iter()
        .map(|(b, _)| b.header.extra_data[0])
        .collect();
    assert!(builders.len() > 1, "{builders:?}");
}
