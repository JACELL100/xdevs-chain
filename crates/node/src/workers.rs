//! [`WorkerPool`]: a small fixed set of named OS threads for blocking work that must
//! never queue behind other work (security review: RPC load starving consensus).
//!
//! The consensus [`crate::ChainApp`] runs every storage / EVM step here instead of on
//! tokio's shared blocking pool, which also serves the RPC backend. However many heavy
//! `eth_call`s are in flight, validate / propose / commit start at once.

use anyhow::anyhow;
use parking_lot::Mutex;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{mpsc, Arc},
};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed pool of OS threads fed by a FIFO queue. Threads exit once the pool is
/// dropped and the queue is drained.
pub(crate) struct WorkerPool {
    name: &'static str,
    jobs: mpsc::Sender<Job>,
}

impl std::fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerPool")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl WorkerPool {
    /// Spawn `threads` (at least 1) threads named `<name>-<i>`.
    pub(crate) fn new(name: &'static str, threads: usize) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..threads.max(1) {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || loop {
                    // The lock is only held while waiting for the next job.
                    let job = rx.lock().recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => break, // pool dropped
                    }
                })?;
        }
        Ok(Self { name, jobs: tx })
    }

    /// Run `f` on a pool thread and wait for its result (a panic becomes an error).
    pub(crate) async fn run<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(catch_unwind(AssertUnwindSafe(f)));
        });
        self.jobs
            .send(job)
            .map_err(|_| anyhow!("{} worker pool stopped", self.name))?;
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(anyhow!("{} task panicked", self.name)),
            Err(_) => Err(anyhow!("{} worker pool stopped", self.name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Regression (RPC DoS): work on the pool starts at once even while tokio's
    /// blocking pool is saturated by other (RPC) work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runs_while_the_blocking_pool_is_saturated() {
        let pool = WorkerPool::new("test-pool", 2).unwrap();
        assert_eq!(pool.run(|| 7).await.unwrap(), 7);
        assert!(pool
            .run(|| panic!("boom"))
            .await
            .unwrap_err()
            .to_string()
            .contains("panicked"));

        // Occupy far more blocking threads than the runtime will start: further
        // spawn_blocking calls queue, the pool does not.
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(2)
            .enable_all()
            .build()
            .unwrap();
        for _ in 0..8 {
            let stop = stop.clone();
            rt.spawn_blocking(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
        }
        let queued = rt.spawn_blocking(|| 1);
        let started = Instant::now();
        assert_eq!(pool.run(|| 2).await.unwrap(), 2);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !queued.is_finished(),
            "the shared blocking pool is saturated"
        );
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(queued.await.unwrap(), 1);
        rt.shutdown_background();
    }
}
