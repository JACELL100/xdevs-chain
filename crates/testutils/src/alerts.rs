//! Safety alerts from the node logs: a process-wide `tracing` layer that counts the
//! WARN / ERROR events a safe chain never emits (SignGuard refusals, equivocation
//! evidence, a commit that would fork). The [`crate::LocalNet`] harness snapshots the
//! counters when it starts and asserts they did not move.
//!
//! [`init_tracing`] also prints the node logs to the test output when `RUST_LOG` is set
//! (e.g. `RUST_LOG=xdevs=debug`).

use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use tracing::{field::Field, Event, Level, Subscriber};
use tracing_subscriber::{
    filter::LevelFilter, fmt, layer::Context, prelude::*, EnvFilter, Layer, Registry,
};

/// Counters of safety-relevant log events since the process started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SafetyAlerts {
    /// The SignGuard refused to sign (regression or double-sign attempt).
    pub sign_guard_refusals: u64,
    /// Equivocation evidence (double vote / double proposal) reported by consensus.
    pub equivocations: u64,
    /// A decided block conflicting with a committed one (`FORK:` commit error).
    pub forks: u64,
}

impl SafetyAlerts {
    /// Counters accumulated since `earlier`.
    pub fn since(&self, earlier: &SafetyAlerts) -> SafetyAlerts {
        SafetyAlerts {
            sign_guard_refusals: self.sign_guard_refusals - earlier.sign_guard_refusals,
            equivocations: self.equivocations - earlier.equivocations,
            forks: self.forks - earlier.forks,
        }
    }

    /// No alert at all.
    pub fn is_clean(&self) -> bool {
        *self == SafetyAlerts::default()
    }
}

static SIGN_GUARD: AtomicU64 = AtomicU64::new(0);
static EQUIVOCATION: AtomicU64 = AtomicU64::new(0);
static FORK: AtomicU64 = AtomicU64::new(0);

/// Current process-wide counters (call [`init_tracing`] first, or they stay zero).
pub fn safety_alerts() -> SafetyAlerts {
    SafetyAlerts {
        sign_guard_refusals: SIGN_GUARD.load(Ordering::SeqCst),
        equivocations: EQUIVOCATION.load(Ordering::SeqCst),
        forks: FORK.load(Ordering::SeqCst),
    }
}

/// Classify one log message (exposed for the unit test).
fn classify(message: &str) {
    if message.contains("sign guard:") {
        SIGN_GUARD.fetch_add(1, Ordering::SeqCst);
    }
    if message.contains("equivocation evidence") {
        EQUIVOCATION.fetch_add(1, Ordering::SeqCst);
    }
    if message.contains("FORK:") {
        FORK.fetch_add(1, Ordering::SeqCst);
    }
}

/// Collects the formatted `message` and every other field of an event.
#[derive(Default)]
struct Text(String);

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, " {}={value:?}", field.name());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(' ');
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(value);
    }
}

/// The counting layer (WARN and above).
struct AlertLayer;

impl<S: Subscriber> Layer<S> for AlertLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() > Level::WARN {
            return;
        }
        let mut text = Text::default();
        event.record(&mut text);
        classify(&text.0);
    }
}

/// Install the process-wide subscriber once: the alert counters, plus the formatted
/// logs on the test output when `RUST_LOG` is set. A no-op if another global
/// subscriber is already installed (the counters then stay zero).
pub fn init_tracing() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let logs = std::env::var("RUST_LOG").ok().map(|_| {
            fmt::layer()
                .with_test_writer()
                .with_filter(EnvFilter::from_default_env())
        });
        let _ = Registry::default()
            .with(AlertLayer.with_filter(LevelFilter::WARN))
            .with(logs)
            .try_init();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_safety_messages() {
        init_tracing();
        let before = safety_alerts();
        tracing::info!(target: "xdevs::consensus", "sign guard: ignored below WARN");
        tracing::error!(target: "xdevs::consensus", "sign guard: DOUBLE-SIGN attempt at ...");
        tracing::warn!(target: "xdevs::consensus", validator = "v", "equivocation evidence");
        let reason = "FORK: block 3 is committed as 0x01, consensus decided 0x02";
        tracing::error!(target: "xdevs::consensus", height = 3, "commit failed: {reason}");
        tracing::warn!(target: "xdevs::consensus", "nothing to see");
        let d = safety_alerts().since(&before);
        assert_eq!(
            d,
            SafetyAlerts {
                sign_guard_refusals: 1,
                equivocations: 1,
                forks: 1
            }
        );
        assert!(!d.is_clean());
        assert!(safety_alerts().since(&safety_alerts()).is_clean());
    }
}
