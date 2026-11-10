//! Test utilities for xDEVS Chain (research doc §6.12, WP5). **Dev-only**: depend on it
//! from `[dev-dependencies]` only.
//!
//! Planned contents (WP5):
//! * `LocalNet::start(n, opts)`: `n` in-process nodes with fresh Ed25519 consensus and
//!   node keys, a genesis with `n` equal-power validators plus the dev alloc, 127.0.0.1
//!   ports, `persistent_peers` = all others, one tempdir per node.
//! * Controls: `stop(i)`, `crash(i)`, `restart(i)`, `add_node()`.
//! * Assertions: `wait_height(h, timeout)`, `assert_no_forks()`, `assert_roots()`.
//!
//! **Owner: WP5.** Day-0 skeleton only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{
    future::Future,
    time::{Duration, Instant},
};

/// Poll `check` every `every` until it returns `true` or `timeout` passes. Returns whether
/// the condition was met. The basic building block of the `wait_*` assertions.
pub async fn wait_until<F, Fut>(timeout: Duration, every: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if check().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn wait_until_succeeds_and_times_out() {
        let n = AtomicU32::new(0);
        let ok = wait_until(Duration::from_secs(5), Duration::from_millis(1), || async {
            n.fetch_add(1, Ordering::Relaxed) >= 3
        })
        .await;
        assert!(ok);
        let never = wait_until(
            Duration::from_millis(20),
            Duration::from_millis(5),
            || async { false },
        )
        .await;
        assert!(!never);
    }
}
