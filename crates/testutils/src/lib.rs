//! Test utilities for xDEVS Chain (research doc §6.12, WP5). **Dev-only**: depend on it
//! from `[dev-dependencies]` only.
//!
//! * [`LocalNet`]: N validators + M full nodes in-process (ephemeral 127.0.0.1 ports,
//!   one temp dir per node), with stop / restart / late-joiner controls and the
//!   `wait_for_height` / `assert_no_fork` assertions.
//! * [`alerts`]: counters of safety-relevant log events (SignGuard refusals,
//!   equivocation evidence, forks).
//! * [`wait_until`]: the polling primitive.
//!
//! The Phase 2 exit-criteria tests live in `tests/phase2_exit.rs`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod alerts;
mod localnet;

pub use alerts::{init_tracing, safety_alerts, SafetyAlerts};
pub use localnet::{free_ports, tcp_addr, LocalNet, LocalNetOptions, NetNode, Role};

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
