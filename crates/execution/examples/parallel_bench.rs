//! Block-STM benchmark: sequential vs parallel execution of generated blocks.
//!
//! ```text
//! cargo run --release -p xdevs-execution --example parallel_bench [txs] [threads...]
//! ```
//!
//! For each workload (independent transfers, ERC-20 with a hot receiver, one hot
//! counter slot, the mixed differential workload) it builds one block of `txs`
//! transactions (default 2,000), executes it sequentially and with Block-STM on each
//! thread count (default 1, 2, 4, 8), checks that the outcomes are identical, and prints
//! the median of 5 runs. Numbers in `docs/research/parallel-execution.md`.

#[path = "../tests/workload/mod.rs"]
mod workload;

use std::time::{Duration, Instant};
use workload::{Mix, Workload};
use xdevs_execution::StakingInputs;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let txs = args.first().copied().unwrap_or(2_000);
    let threads: Vec<usize> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![1, 2, 4, 8]
    };
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("Block-STM benchmark: {txs} txs per block, {cores} cores available");
    println!(
        "{:<12} {:>10} {:>8} {:>10} {:>8} {:>10}",
        "workload", "threads", "ms", "speedup", "execs", "aborts"
    );
    let staking = StakingInputs::default();
    for mix in [Mix::Independent, Mix::Erc20, Mix::HotSpot, Mix::Mixed] {
        let mut w = Workload::new(1, 256.min(txs.max(1)));
        w.gas_limit = 2_000_000_000;
        let list = w.random_txs(mix, txs);
        let block = w.build(list);
        let seq_out = w
            .exec
            .execute_sequential(&w.parent, &block, &w.state, &staking)
            .expect("sequential");
        let seq = median(
            (0..5)
                .map(|_| {
                    let t = Instant::now();
                    w.exec
                        .execute_sequential(&w.parent, &block, &w.state, &staking)
                        .expect("sequential");
                    t.elapsed()
                })
                .collect(),
        );
        println!(
            "{:<12} {:>10} {:>8.1} {:>10} {:>8} {:>10}",
            format!("{mix:?}"),
            "sequential",
            seq.as_secs_f64() * 1e3,
            "1.00x",
            txs,
            0
        );
        for &t in &threads {
            let mut stats = None;
            let par = median(
                (0..5)
                    .map(|_| {
                        let start = Instant::now();
                        let (out, s) = w
                            .exec
                            .execute_parallel(&w.parent, &block, &w.state, &staking, t)
                            .expect("parallel");
                        let took = start.elapsed();
                        assert_eq!(out, seq_out, "parallel outcome differs");
                        stats = Some(s);
                        took
                    })
                    .collect(),
            );
            let s = stats.expect("ran");
            println!(
                "{:<12} {:>10} {:>8.1} {:>9.2}x {:>8} {:>10}",
                "",
                t,
                par.as_secs_f64() * 1e3,
                seq.as_secs_f64() / par.as_secs_f64(),
                s.executions,
                s.validation_aborts
            );
        }
    }
}
