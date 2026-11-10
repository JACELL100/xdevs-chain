//! Four in-process engines, one of them Byzantine (`Host::publish_junk`): on every
//! round it floods proposal-part streams — two different signed blocks per round
//! claiming itself as proposer (equivocation in its own rounds, wrong proposer in the
//! others), blocks claiming the real proposer under its own signature, bodies that do
//! not match their header, next / far-future / `u64::MAX` heights and oversized
//! streams — while voting honestly. The three correct engines must keep deciding, never
//! fork, and report the Byzantine proposer's equivocation.
//!
//! A mock counter app as in `tests/four_engines.rs`; ephemeral ports and temp dirs.

use super::engine::run_inner;
use super::private_key;
use crate::{
    BlockOrigin, CommitCertificate, ConsensusApp, ConsensusTimeouts, EngineConfig, EngineKeys,
    EngineStatus, Evidence, Validator, ValidatorKey, ValidatorSet, Validity,
};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use xdevs_primitives::{Block, Bytes, Header, B256};

const CHAIN_ID: u64 = 7117;
const GENESIS: B256 = B256::repeat_byte(0x11);

#[derive(Default)]
struct Ledger {
    decided: Mutex<BTreeMap<u64, B256>>,
    conflicts: AtomicUsize,
}

struct CounterApp {
    index: u8,
    set: ValidatorSet,
    chain: Mutex<Vec<(Block, CommitCertificate)>>,
    ledger: Arc<Ledger>,
    evidence: Mutex<Vec<Evidence>>,
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

    async fn propose(&self, height: u64, round: u32, _deadline: Instant) -> anyhow::Result<Block> {
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

    async fn report_evidence(&self, e: Evidence) -> anyhow::Result<()> {
        self.evidence.lock().unwrap().push(e);
        Ok(())
    }
}

struct Node {
    app: Arc<CounterApp>,
    stop: watch::Sender<bool>,
    task: JoinHandle<anyhow::Result<()>>,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_byzantine_junk_streamer_cannot_halt_or_fork_the_chain() {
    const N: usize = 4;
    const BYZANTINE: usize = 3;
    const TARGET: u64 = 12;
    let dirs: Vec<_> = (0..N).map(|_| tempfile::tempdir().unwrap()).collect();
    let ports = free_ports(N);
    let seeds: Vec<[u8; 32]> = (0..N).map(|i| [i as u8 + 41; 32]).collect();
    let set = ValidatorSet::new(
        seeds
            .iter()
            .map(|s| {
                let pk = *private_key(s).public_key().as_bytes();
                Validator::new(ValidatorKey::from_bytes(pk).unwrap(), 1)
            })
            .collect(),
    )
    .unwrap();
    let ledger = Arc::new(Ledger::default());

    let nodes: Vec<Node> = (0..N)
        .map(|i| {
            let mut config = EngineConfig::new(CHAIN_ID, dirs[i].path());
            config.moniker = format!("byz-node-{i}");
            config.timeouts = ConsensusTimeouts::fast_for_tests(250);
            config.p2p.listen = format!("/ip4/127.0.0.1/tcp/{}", ports[i]);
            config.p2p.persistent_peers = (0..N)
                .filter(|j| *j != i)
                .map(|j| format!("/ip4/127.0.0.1/tcp/{}", ports[j]))
                .collect();
            let app = Arc::new(CounterApp {
                index: i as u8,
                set: set.clone(),
                chain: Mutex::new(Vec::new()),
                ledger: ledger.clone(),
                evidence: Mutex::new(Vec::new()),
            });
            let (stop, rx) = watch::channel(false);
            let dyn_app: Arc<dyn ConsensusApp> = app.clone();
            let task = tokio::spawn(run_inner(
                config,
                EngineKeys::validator(seeds[i], [140 + i as u8; 32]),
                dyn_app,
                rx,
                EngineStatus::default(),
                i == BYZANTINE,
            ));
            Node { app, stop, task }
        })
        .collect();

    let honest = [0usize, 1, 2];
    let heights = |nodes: &[Node]| nodes.iter().map(|n| n.app.height()).collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        for &i in &honest {
            assert!(!nodes[i].task.is_finished(), "engine {i} exited");
        }
        if honest.iter().all(|&i| nodes[i].app.height() >= TARGET) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "correct engines stalled under junk streams: {:?}",
            heights(&nodes)
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!("heights with a Byzantine streamer: {:?}", heights(&nodes));

    for n in &nodes {
        let _ = n.stop.send(true);
    }
    for (i, n) in nodes.iter().enumerate() {
        if honest.contains(&i) {
            let res = tokio::time::timeout(Duration::from_secs(30), async {
                while !n.task.is_finished() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await;
            assert!(res.is_ok(), "engine {i} stops");
        }
    }
    for n in nodes.iter() {
        n.task.abort();
    }

    assert_eq!(ledger.conflicts.load(Ordering::SeqCst), 0, "fork detected");
    let reference = nodes[0].app.chain.lock().unwrap().clone();
    for &i in &honest {
        let chain = nodes[i].app.chain.lock().unwrap();
        let mut parent = GENESIS;
        for (k, (block, cert)) in chain.iter().enumerate() {
            assert_eq!(block.number(), k as u64 + 1);
            assert_eq!(block.header.parent_hash, parent);
            cert.verify_at(CHAIN_ID, block.number(), &set).unwrap();
            // Only real (counter) blocks are ever decided, never junk.
            assert_eq!(block.header.extra_data.len(), 5);
            parent = block.hash();
        }
        for k in 0..TARGET as usize {
            assert_eq!(chain[k].0.hash(), reference[k].0.hash(), "height {}", k + 1);
        }
    }
    // Its two blocks per own round were detected as equivocation.
    let byz_addr =
        ValidatorKey::from_bytes(*private_key(&seeds[BYZANTINE]).public_key().as_bytes())
            .unwrap()
            .address();
    let reported: usize = honest
        .iter()
        .map(|&i| {
            nodes[i]
                .app
                .evidence
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.validator() == byz_addr)
                .count()
        })
        .sum();
    assert!(reported > 0, "no equivocation evidence reported");
    let others: usize = honest
        .iter()
        .map(|&i| {
            nodes[i]
                .app
                .evidence
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.validator() != byz_addr)
                .count()
        })
        .sum();
    assert_eq!(others, 0, "correct validators were never accused");
}
