//! The load run: setup (fund senders, deploy the token), the paced send loop, the block
//! poller that detects inclusion, and the final report.
//!
//! Concurrency: one send loop (pacing + signing), one spawned task per RPC send (bounded
//! by a semaphore), one block poller, and occasional nonce-resync tasks. They share a
//! single `Mutex<Shared>` that is never held across an `.await`.

use crate::{
    accounts::{load_senders, parse_key_spec, SenderSource},
    contract,
    mix::{LoadKind, Mix},
    nonce::NonceManager,
    pacing::TokenBucket,
    rpc::{self, classify, RpcPool, SendFailure},
    stats::{round1, round3, KindStats, LatencySummary, Report, ReportConfig},
};
use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use xdevs_crypto::{sign_tx_envelope, PrivateKeySigner};
use xdevs_primitives::{keccak256, Address, Bytes, Encodable2718, TxEip1559, TxKind, B256, U256};

/// 1 tXDS / XDS in wei.
pub const ETHER: u128 = 1_000_000_000_000_000_000;

/// Run settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// RPC endpoints; sends go round-robin, setup and polling use the first.
    pub rpc: Vec<String>,
    /// Target transactions per second.
    pub tps: f64,
    /// Send-phase length.
    pub duration: Duration,
    /// Number of sender accounts.
    pub senders: usize,
    /// Where sender keys come from.
    pub source: SenderSource,
    /// Amount sent to each fresh sender with [`SenderSource::Fund`] (wei).
    pub fund_amount: U256,
    /// Tx mix.
    pub mix: Mix,
    /// Fresh slots written per `storage` tx.
    pub storage_slots: u64,
    /// Max unconfirmed txs per sender (the node's pool allows 64).
    pub window: u64,
    /// Max unconfirmed txs over all senders.
    pub max_inflight: u64,
    /// Max concurrent `eth_sendRawTransaction` calls.
    pub max_concurrent_sends: usize,
    /// Per-request RPC timeout.
    pub request_timeout: Duration,
    /// Block poll interval.
    pub poll_interval: Duration,
    /// After the send phase, wait at most this long for accepted txs to be included.
    pub drain: Duration,
    /// Priority fee (wei); default: `eth_maxPriorityFeePerGas`.
    pub tip: Option<u128>,
    /// Max fee per gas (wei); default: `2 * base fee + tip`, following the base fee.
    pub max_fee: Option<u128>,
    /// Install a Ctrl-C handler (stop and report so far).
    pub handle_ctrl_c: bool,
    /// Print progress lines to stderr.
    pub progress: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rpc: vec!["http://127.0.0.1:8545".into()],
            tps: 100.0,
            duration: Duration::from_secs(60),
            senders: 10,
            source: SenderSource::Dev,
            fund_amount: U256::from(100 * ETHER),
            mix: Mix::default(),
            storage_slots: 8,
            window: 48,
            max_inflight: 4096,
            max_concurrent_sends: 256,
            request_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(100),
            drain: Duration::from_secs(30),
            tip: None,
            max_fee: None,
            handle_ctrl_c: false,
            progress: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// `eth_sendRawTransaction` not answered yet.
    Sending,
    /// Accepted by the node.
    Accepted,
    /// Timeout / transport error: may or may not be in the pool.
    Ambiguous,
}

#[derive(Debug, Clone)]
struct Tracked {
    sender: usize,
    nonce: u64,
    kind: LoadKind,
    sent_at: Instant,
    sent_wall_ms: u64,
    status: Status,
}

#[derive(Debug, Clone, Copy)]
struct BlockSeen {
    number: u64,
    tx_count: u64,
}

#[derive(Debug)]
struct Shared {
    nonces: NonceManager,
    tracked: HashMap<B256, Tracked>,
    sent: u64,
    accepted: u64,
    rejected: u64,
    send_errors: u64,
    errors: BTreeMap<String, u64>,
    included: u64,
    included_accepted: u64,
    receipts_ok: u64,
    receipts_failed: u64,
    receipts_unchecked: u64,
    latencies: Vec<f64>,
    block_ts_latencies: Vec<f64>,
    blocks: Vec<BlockSeen>,
    last_block: u64,
    last_inclusion_block: Option<u64>,
    last_inclusion_at: Option<Instant>,
    base_fee: u128,
    resyncs: u64,
    window_full_stalls: u64,
    by_kind: BTreeMap<LoadKind, KindStats>,
}

impl Shared {
    fn on_accepted(&mut self, hash: &B256) {
        self.accepted += 1;
        match self.tracked.get_mut(hash) {
            Some(t) => t.status = Status::Accepted,
            // The poller saw it in a block before the RPC answer was processed.
            None => self.included_accepted += 1,
        }
    }

    fn count_error(&mut self, key: String) {
        *self.errors.entry(key).or_default() += 1;
    }
}

struct Ctx {
    rpc: RpcPool,
    shared: Mutex<Shared>,
    addrs: Vec<Address>,
    interrupted: AtomicBool,
    stop_poll: AtomicBool,
}

impl Ctx {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Fee parameters for a new tx.
fn max_fee_for(cfg: &Config, base_fee: u128, tip: u128) -> u128 {
    cfg.max_fee
        .unwrap_or_else(|| base_fee.saturating_mul(2).saturating_add(tip))
        .max(tip)
}

/// Sign an EIP-1559 tx; returns `(hash, raw 2718 bytes)`.
#[allow(clippy::too_many_arguments)]
fn sign_1559(
    signer: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    max_fee: u128,
    tip: u128,
    to: TxKind,
    value: U256,
    input: Bytes,
) -> anyhow::Result<(B256, Bytes)> {
    let env = sign_tx_envelope(
        signer,
        TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: tip,
            to,
            value,
            access_list: Default::default(),
            input,
        },
    )
    .map_err(|e| anyhow!("signing failed: {e}"))?;
    Ok((*env.tx_hash(), Bytes::from(env.encoded_2718())))
}

/// Deterministic recipient pool (1024 addresses).
fn recipient(k: u64) -> Address {
    let h = keccak256(format!("xdevs-loadgen recipient {}", k % 1024));
    Address::from_slice(&h[12..])
}

/// Setup-phase helper: send signed txs to the primary endpoint and wait until all are
/// mined with status 1.
async fn send_and_confirm(
    rpc: &RpcPool,
    txs: Vec<(B256, Bytes)>,
    what: &str,
) -> anyhow::Result<Vec<Value>> {
    let c = rpc.primary();
    let mut hashes = Vec::with_capacity(txs.len());
    for (hash, raw) in txs {
        rpc::send_raw(c, &raw)
            .await
            .map_err(|e| anyhow!("{what}: eth_sendRawTransaction failed: {e}"))?;
        hashes.push(hash);
    }
    let receipts = rpc::wait_receipts(c, &hashes, Duration::from_secs(60))
        .await
        .with_context(|| what.to_string())?;
    if let Some(bad) = receipts.iter().find(|r| !rpc::receipt_ok(r)) {
        bail!("{what}: tx failed: {bad}");
    }
    Ok(receipts)
}

/// Run the load test and return its report.
pub async fn run(cfg: Config) -> anyhow::Result<Report> {
    if !cfg.tps.is_finite() || cfg.tps <= 0.0 {
        bail!("--tps must be > 0");
    }
    let rpc = RpcPool::new(&cfg.rpc, cfg.request_timeout)?;
    let c = rpc.primary();
    let chain_id = rpc::chain_id(c)
        .await
        .with_context(|| format!("cannot reach {}", cfg.rpc[0]))?;
    let (base_fee, suggested_tip) = rpc::fees(c).await?;
    let tip = cfg.tip.unwrap_or(suggested_tip);
    let log = |msg: String| {
        if cfg.progress {
            eprintln!("{msg}");
        }
    };
    log(format!(
        "chain {chain_id}, base fee {base_fee} wei, tip {tip} wei, {} endpoint(s)",
        rpc.urls().len()
    ));

    // --- senders ---------------------------------------------------------------------
    let signers = load_senders(&cfg.source, cfg.senders)?;
    let addrs: Vec<Address> = signers.iter().map(|s| s.address()).collect();
    if let SenderSource::Fund(spec) = &cfg.source {
        let funder = parse_key_spec(spec)?;
        let mut n = rpc::nonce(c, funder.address(), "pending").await?;
        let needed = cfg.fund_amount * U256::from(addrs.len());
        let have = rpc::balance(c, funder.address()).await?;
        if have < needed {
            bail!(
                "funder {} has {have} wei, needs {needed} + gas",
                funder.address()
            );
        }
        let mut txs = Vec::new();
        for a in &addrs {
            txs.push(sign_1559(
                &funder,
                chain_id,
                n,
                21_000,
                max_fee_for(&cfg, base_fee, tip),
                tip,
                TxKind::Call(*a),
                cfg.fund_amount,
                Bytes::new(),
            )?);
            n += 1;
        }
        log(format!(
            "funding {} fresh senders from {}",
            addrs.len(),
            funder.address()
        ));
        send_and_confirm(&rpc, txs, "funding senders").await?;
    }
    let mut nonces = Vec::with_capacity(addrs.len());
    for a in &addrs {
        nonces.push(rpc::nonce(c, *a, "pending").await?);
    }

    // --- token contract ----------------------------------------------------------------
    let needs_token = cfg.mix.contains(LoadKind::Erc20) || cfg.mix.contains(LoadKind::Storage);
    let mut token = Address::ZERO;
    if needs_token {
        let deployer = &signers[0];
        let (hash, raw) = sign_1559(
            deployer,
            chain_id,
            nonces[0],
            contract::DEPLOY_GAS,
            max_fee_for(&cfg, base_fee, tip),
            tip,
            TxKind::Create,
            U256::ZERO,
            contract::token_init_code(),
        )?;
        let receipts = send_and_confirm(&rpc, vec![(hash, raw)], "token deployment").await?;
        token = receipts[0]
            .get("contractAddress")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(|| addrs[0].create(nonces[0]));
        nonces[0] += 1;
        let out = rpc::eth_call(c, token, &contract::balance_of_calldata(addrs[0])).await?;
        if out.len() != 32 || U256::from_be_slice(&out) != contract::TOKEN_SUPPLY {
            bail!("token at {token} misbehaves: balanceOf(deployer) = {out}");
        }
        log(format!("token deployed at {token}"));
        if cfg.mix.contains(LoadKind::Erc20) && addrs.len() > 1 {
            let share = contract::TOKEN_SUPPLY / U256::from(2 * addrs.len());
            let mut txs = Vec::new();
            for a in &addrs[1..] {
                txs.push(sign_1559(
                    deployer,
                    chain_id,
                    nonces[0],
                    contract::ERC20_TRANSFER_GAS,
                    max_fee_for(&cfg, base_fee, tip),
                    tip,
                    TxKind::Call(token),
                    U256::ZERO,
                    contract::transfer_calldata(*a, share),
                )?);
                nonces[0] += 1;
            }
            send_and_confirm(&rpc, txs, "token distribution").await?;
        }
    }

    // --- measured phase ------------------------------------------------------------------
    let start_block = rpc::block_number(c).await?;
    let window = cfg.window.max(1);
    let ctx = Arc::new(Ctx {
        rpc,
        shared: Mutex::new(Shared {
            nonces: NonceManager::new(&nonces, window, cfg.max_inflight),
            tracked: HashMap::new(),
            sent: 0,
            accepted: 0,
            rejected: 0,
            send_errors: 0,
            errors: BTreeMap::new(),
            included: 0,
            included_accepted: 0,
            receipts_ok: 0,
            receipts_failed: 0,
            receipts_unchecked: 0,
            latencies: Vec::new(),
            block_ts_latencies: Vec::new(),
            blocks: Vec::new(),
            last_block: start_block,
            last_inclusion_block: None,
            last_inclusion_at: None,
            base_fee,
            resyncs: 0,
            window_full_stalls: 0,
            by_kind: BTreeMap::new(),
        }),
        addrs,
        interrupted: AtomicBool::new(false),
        stop_poll: AtomicBool::new(false),
    });

    if cfg.handle_ctrl_c {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if ctx.interrupted.swap(true, Ordering::SeqCst) {
                    eprintln!("second Ctrl-C: exiting without a report");
                    std::process::exit(130);
                }
                eprintln!("Ctrl-C: stopping, report follows (press again to abort)");
            }
        });
    }

    let poller = tokio::spawn(poll_blocks(ctx.clone(), start_block, cfg.poll_interval));
    log(format!(
        "sending {} tps for {:?} from {} senders (mix {}), from block {start_block}",
        cfg.tps,
        cfg.duration,
        signers.len(),
        cfg.mix
    ));

    let max_sends = cfg.max_concurrent_sends.max(1);
    let sem = Arc::new(Semaphore::new(max_sends));
    let send_start = Instant::now();
    let deadline = send_start + cfg.duration;
    let mut bucket = TokenBucket::new(cfg.tps, TokenBucket::default_capacity(cfg.tps), send_start);
    let mut picker = cfg.mix.picker();
    let mut counter: u64 = 0;
    let mut next_progress = send_start + Duration::from_secs(5);

    loop {
        let now = Instant::now();
        if now >= deadline || ctx.interrupted.load(Ordering::Relaxed) {
            break;
        }
        if cfg.progress && now >= next_progress {
            next_progress += Duration::from_secs(5);
            let s = ctx.lock();
            eprintln!(
                "[{:>5.1}s] sent {} accepted {} included {} rejected {} unconfirmed {} block {}",
                now.duration_since(send_start).as_secs_f64(),
                s.sent,
                s.accepted,
                s.included,
                s.rejected + s.send_errors,
                s.nonces.total_outstanding(),
                s.last_block
            );
        }
        let ready = ctx.lock().nonces.take_resync_ready();
        for i in ready {
            tokio::spawn(resync(ctx.clone(), i));
        }
        if !bucket.ready(now) {
            let wait = bucket.wait_time().max(Duration::from_millis(1));
            tokio::time::sleep(wait.min(deadline.saturating_duration_since(now))).await;
            continue;
        }
        let (picked, base) = {
            let mut s = ctx.lock();
            let p = s.nonces.pick();
            if p.is_none() {
                s.window_full_stalls += 1;
            }
            (p, s.base_fee)
        };
        let Some((i, nonce)) = picked else {
            tokio::time::sleep(Duration::from_millis(2)).await;
            continue;
        };
        bucket.try_take(now);
        let kind = picker.next_kind();
        counter += 1;
        let (gas, to, value, input) = match kind {
            LoadKind::Transfer => (
                21_000,
                recipient(counter),
                U256::from(1_000_000_000u64),
                Bytes::new(),
            ),
            LoadKind::Erc20 => (
                contract::ERC20_TRANSFER_GAS,
                token,
                U256::ZERO,
                contract::transfer_calldata(recipient(counter), U256::from(1u64)),
            ),
            LoadKind::Storage => (
                contract::stress_gas(cfg.storage_slots),
                token,
                U256::ZERO,
                contract::stress_calldata(cfg.storage_slots),
            ),
        };
        let signed = sign_1559(
            &signers[i],
            chain_id,
            nonce,
            gas,
            max_fee_for(&cfg, base, tip),
            tip,
            TxKind::Call(to),
            value,
            input,
        );
        let (hash, raw) = match signed {
            Ok(x) => x,
            Err(e) => {
                let mut s = ctx.lock();
                s.nonces.send_done(i);
                s.nonces.release(i, nonce);
                drop(s);
                return Err(e);
            }
        };
        {
            let mut s = ctx.lock();
            s.sent += 1;
            s.by_kind.entry(kind).or_default().sent += 1;
            s.tracked.insert(
                hash,
                Tracked {
                    sender: i,
                    nonce,
                    kind,
                    sent_at: Instant::now(),
                    sent_wall_ms: wall_ms(),
                    status: Status::Sending,
                },
            );
        }
        let permit = sem.clone().acquire_owned().await.expect("semaphore open");
        tokio::spawn(send_one(ctx.clone(), hash, raw, i, nonce, permit));
    }
    let send_duration = send_start.elapsed();
    log(format!(
        "send phase done after {:.1} s; draining",
        send_duration.as_secs_f64()
    ));

    // Wait for in-flight sends, then for accepted txs to be included.
    let _ = tokio::time::timeout(
        cfg.request_timeout + Duration::from_secs(5),
        sem.acquire_many(max_sends as u32),
    )
    .await;
    let drain_deadline = Instant::now() + cfg.drain;
    while !ctx.interrupted.load(Ordering::Relaxed) && Instant::now() < drain_deadline {
        let waiting = ctx
            .lock()
            .tracked
            .values()
            .filter(|t| t.status != Status::Ambiguous)
            .count();
        if waiting == 0 {
            break;
        }
        tokio::time::sleep(cfg.poll_interval).await;
    }
    ctx.stop_poll.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(cfg.request_timeout * 2, poller).await;

    let s = ctx.lock();
    let report_cfg = ReportConfig {
        rpc: cfg.rpc.clone(),
        target_tps: cfg.tps,
        duration_s: cfg.duration.as_secs_f64(),
        senders: signers.len(),
        mix: cfg.mix.to_string(),
        window_per_sender: window,
        chain_id,
    };
    Ok(build_report(
        &s,
        report_cfg,
        ctx.interrupted.load(Ordering::Relaxed),
        send_start,
        send_duration,
        start_block,
    ))
}

fn build_report(
    s: &Shared,
    config: ReportConfig,
    interrupted: bool,
    send_start: Instant,
    send_duration: Duration,
    start_block: u64,
) -> Report {
    let last_relevant = s.last_inclusion_block.unwrap_or(s.last_block);
    let blocks: Vec<&BlockSeen> = s
        .blocks
        .iter()
        .filter(|b| b.number <= last_relevant)
        .collect();
    let span = s
        .last_inclusion_at
        .map(|t| t.duration_since(send_start).as_secs_f64())
        .unwrap_or(0.0);
    let send_s = send_duration.as_secs_f64();
    Report {
        config,
        interrupted,
        send_duration_s: round3(send_s),
        total_duration_s: round3(send_start.elapsed().as_secs_f64()),
        sent: s.sent,
        accepted: s.accepted,
        rejected: s.rejected,
        send_errors: s.send_errors,
        errors: s.errors.clone(),
        included: s.included,
        included_accepted: s.included_accepted,
        not_included: s
            .tracked
            .values()
            .filter(|t| t.status != Status::Ambiguous)
            .count() as u64,
        inclusion_rate: if s.accepted == 0 {
            1.0
        } else {
            round3(s.included_accepted as f64 / s.accepted as f64)
        },
        receipts_ok: s.receipts_ok,
        receipts_failed: s.receipts_failed,
        receipts_unchecked: s.receipts_unchecked,
        latency_ms: LatencySummary::from_samples(&s.latencies),
        block_ts_latency_ms: LatencySummary::from_samples(&s.block_ts_latencies),
        send_tps: if send_s > 0.0 {
            round1(s.sent as f64 / send_s)
        } else {
            0.0
        },
        achieved_tps: if span > 0.0 {
            round1(s.included as f64 / span)
        } else {
            0.0
        },
        blocks_seen: blocks.len() as u64,
        empty_blocks: blocks.iter().filter(|b| b.tx_count == 0).count() as u64,
        block_txs: blocks.iter().map(|b| b.tx_count).sum(),
        first_block: start_block + 1,
        last_block: s.last_block,
        nonce_resyncs: s.resyncs,
        window_full_stalls: s.window_full_stalls,
        by_kind: s
            .by_kind
            .iter()
            .map(|(k, v)| (k.name().to_string(), v.clone()))
            .collect(),
    }
}

/// One `eth_sendRawTransaction` and its bookkeeping.
async fn send_one(
    ctx: Arc<Ctx>,
    hash: B256,
    raw: Bytes,
    sender: usize,
    nonce: u64,
    _permit: OwnedSemaphorePermit,
) {
    let res = rpc::send_raw(ctx.rpc.next_client(), &raw).await;
    let mut s = ctx.lock();
    s.nonces.send_done(sender);
    match res {
        Ok(_) => s.on_accepted(&hash),
        Err(e) => {
            let (key, failure) = classify(&e);
            match failure {
                SendFailure::AlreadyKnown => s.on_accepted(&hash),
                SendFailure::Nonce => {
                    s.rejected += 1;
                    s.count_error(key);
                    s.tracked.remove(&hash);
                    s.nonces.mark_resync(sender);
                }
                SendFailure::Rejected => {
                    s.rejected += 1;
                    s.count_error(key);
                    s.tracked.remove(&hash);
                    s.nonces.release(sender, nonce);
                }
                SendFailure::Ambiguous => {
                    s.send_errors += 1;
                    s.count_error(key);
                    if let Some(t) = s.tracked.get_mut(&hash) {
                        t.status = Status::Ambiguous;
                    }
                    s.nonces.mark_resync(sender);
                }
            }
        }
    }
}

/// Re-read sender `i`'s nonce from the node after an error.
async fn resync(ctx: Arc<Ctx>, i: usize) {
    let c = ctx.rpc.primary();
    let addr = ctx.addrs[i];
    let res = async {
        let pending = rpc::nonce(c, addr, "pending").await?;
        let latest = rpc::nonce(c, addr, "latest").await?;
        Ok::<_, jsonrpsee::core::ClientError>((pending, latest))
    }
    .await;
    match res {
        Ok((pending, latest)) => {
            let mut s = ctx.lock();
            s.nonces.resynced(i, pending, latest);
            s.resyncs += 1;
        }
        Err(_) => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            ctx.lock().nonces.resync_failed(i);
        }
    }
}

/// Follow the chain from `start_block + 1`: one `eth_getBlockByNumber` per block (tx
/// hashes) and one `eth_getBlockReceipts` per block that holds our txs.
async fn poll_blocks(ctx: Arc<Ctx>, start_block: u64, interval: Duration) {
    let mut next = start_block + 1;
    loop {
        let stopping = ctx.stop_poll.load(Ordering::SeqCst);
        if let Ok(head) = rpc::block_number(ctx.rpc.primary()).await {
            while next <= head {
                match process_block(&ctx, next).await {
                    Ok(true) => next += 1,
                    _ => break,
                }
            }
        }
        if stopping {
            break;
        }
        tokio::time::sleep(interval).await;
    }
}

/// Returns `Ok(false)` if the block is not available yet.
async fn process_block(ctx: &Ctx, number: u64) -> anyhow::Result<bool> {
    let c = ctx.rpc.primary();
    let tag = format!("0x{number:x}");
    let block = rpc::call(c, "eth_getBlockByNumber", vec![json!(tag), json!(false)])
        .await
        .map_err(|e| anyhow!("eth_getBlockByNumber: {e}"))?;
    if block.is_null() {
        return Ok(false);
    }
    let observed = Instant::now();
    let ts_ms = block
        .get("timestamp")
        .and_then(rpc::parse_u64)
        .unwrap_or(0)
        .saturating_mul(1000);
    let hashes: Vec<B256> = block
        .get("transactions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| serde_json::from_value(h.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    let mut ours: Vec<(B256, LoadKind)> = Vec::new();
    {
        let mut s = ctx.lock();
        if let Some(bf) = block.get("baseFeePerGas").and_then(rpc::parse_u128) {
            s.base_fee = bf;
        }
        s.blocks.push(BlockSeen {
            number,
            tx_count: hashes.len() as u64,
        });
        s.last_block = s.last_block.max(number);
        for h in &hashes {
            let Some(t) = s.tracked.remove(h) else {
                continue;
            };
            s.included += 1;
            s.by_kind.entry(t.kind).or_default().included += 1;
            s.latencies
                .push(observed.duration_since(t.sent_at).as_secs_f64() * 1000.0);
            s.block_ts_latencies
                .push(ts_ms.saturating_sub(t.sent_wall_ms) as f64);
            s.nonces.included(t.sender, t.nonce);
            if t.status == Status::Accepted {
                s.included_accepted += 1;
            }
            ours.push((*h, t.kind));
        }
        if !ours.is_empty() {
            s.last_inclusion_block = Some(number);
            s.last_inclusion_at = Some(observed);
        }
    }
    if ours.is_empty() {
        return Ok(true);
    }

    // Receipt statuses, one call per block (fallback: per tx).
    let mut status: HashMap<B256, bool> = HashMap::new();
    match rpc::call(c, "eth_getBlockReceipts", vec![json!(tag)]).await {
        Ok(Value::Array(receipts)) => {
            for r in receipts {
                if let Some(h) = r
                    .get("transactionHash")
                    .and_then(|v| serde_json::from_value::<B256>(v.clone()).ok())
                {
                    status.insert(h, rpc::receipt_ok(&r));
                }
            }
        }
        _ => {
            for (h, _) in &ours {
                if let Ok(r) = rpc::call(c, "eth_getTransactionReceipt", vec![json!(h)]).await {
                    if !r.is_null() {
                        status.insert(*h, rpc::receipt_ok(&r));
                    }
                }
            }
        }
    }
    let mut s = ctx.lock();
    for (h, kind) in ours {
        match status.get(&h) {
            Some(true) => s.receipts_ok += 1,
            Some(false) => {
                s.receipts_failed += 1;
                s.by_kind.entry(kind).or_default().failed += 1;
            }
            None => s.receipts_unchecked += 1,
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rules() {
        let cfg = Config::default();
        assert_eq!(max_fee_for(&cfg, 100, 7), 207);
        let cfg = Config {
            max_fee: Some(50),
            ..Config::default()
        };
        assert_eq!(max_fee_for(&cfg, 100, 7), 50);
        assert_eq!(max_fee_for(&cfg, 100, 70), 70, "never below the tip");
    }

    #[test]
    fn recipients_are_stable_and_distinct() {
        assert_eq!(recipient(1), recipient(1025));
        assert_ne!(recipient(1), recipient(2));
        assert_ne!(recipient(1), Address::ZERO);
    }
}
