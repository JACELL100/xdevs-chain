//! Prometheus metrics for xDEVS Chain (research doc §8 WP6, plan §24.7).
//!
//! Uses `prometheus-client` **0.23.1**, the registry type Malachite's own metrics use, so
//! consensus metrics and ours are served by one HTTP `/metrics` endpoint (`[metrics] listen`).
//!
//! - [`MetricsRegistry`]: the shared, lockable registry plus extra text sources (for example
//!   Malachite's own registry, which `xdevs-consensus` can expose as a [`MetricsSource`]).
//! - [`ChainMetrics`]: block height, block time, finality latency, consensus rounds,
//!   build / validate / commit / state-root durations, peers, mempool size, tx throughput.
//! - [`RpcMetrics`]: `xdevs_rpc_requests_total{method,result}` and
//!   `xdevs_rpc_latency_seconds{method}` (fed by `xdevs-rpc`).
//! - [`MetricsServer`]: the minimal HTTP/1.1 `/metrics` endpoint.
//!
//! Metrics are observability only: nothing here feeds back into consensus, so wall-clock
//! time and `f64` observations are fine (plan §10.6 applies to consensus paths).
//!
//! **Owner: WP6.**

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod chain;
mod rpc;
mod server;

pub use chain::{ChainMetrics, CommittedBlock};
pub use prometheus_client::registry::Registry;
pub use rpc::RpcMetrics;
pub use server::MetricsServer;

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    net::SocketAddr,
    sync::{Arc, PoisonError, RwLock},
};

/// Prefix of every xdevs metric name.
pub const METRICS_PREFIX: &str = "xdevs";

/// `Content-Type` of the `/metrics` response (OpenMetrics text, which Prometheus scrapes).
pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// `[metrics]` node config section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Serve `/metrics`.
    pub enabled: bool,
    /// HTTP listen address of the `/metrics` endpoint.
    pub listen: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:9615".into(),
        }
    }
}

impl MetricsConfig {
    /// Parsed [`MetricsConfig::listen`] (`ip:port`).
    pub fn listen_addr(&self) -> Result<SocketAddr, String> {
        self.listen
            .trim()
            .parse()
            .map_err(|e| format!("invalid [metrics] listen {:?}: {e}", self.listen))
    }
}

/// A fresh registry with the [`METRICS_PREFIX`] prefix.
pub fn new_registry() -> Registry {
    Registry::with_prefix(METRICS_PREFIX)
}

/// Extra exposition text appended to `/metrics`: writes OpenMetrics text for some other
/// registry (a trailing `# EOF` line is removed, the server adds one at the very end).
pub type MetricsSource = Arc<dyn Fn(&mut String) -> fmt::Result + Send + Sync>;

/// The node's shared metrics registry (cheap to clone; all clones share state).
///
/// Register metrics with [`MetricsRegistry::with_registry`]; render everything with
/// [`MetricsRegistry::render`].
#[derive(Clone)]
pub struct MetricsRegistry {
    registry: Arc<RwLock<Registry>>,
    sources: Arc<RwLock<Vec<MetricsSource>>>,
}

impl fmt::Debug for MetricsRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricsRegistry")
            .field("sources", &read(&self.sources).len())
            .finish_non_exhaustive()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

impl MetricsRegistry {
    /// Empty registry with the `xdevs` prefix.
    pub fn new() -> Self {
        Self::from_registry(new_registry())
    }

    /// Wrap an existing registry.
    pub fn from_registry(registry: Registry) -> Self {
        Self {
            registry: Arc::new(RwLock::new(registry)),
            sources: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Run `f` with mutable access to the registry (to register metrics).
    pub fn with_registry<R>(&self, f: impl FnOnce(&mut Registry) -> R) -> R {
        let mut guard = self
            .registry
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }

    /// Append another exposition source to `/metrics` (e.g. Malachite's registry).
    pub fn add_source(&self, source: MetricsSource) {
        self.sources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(source);
    }

    /// The full OpenMetrics text: this registry, then every source, then `# EOF`.
    pub fn render(&self) -> Result<String, fmt::Error> {
        use prometheus_client::encoding::text::{encode_eof, encode_registry};
        let mut out = String::new();
        encode_registry(&mut out, &read(&self.registry))?;
        for source in read(&self.sources).iter() {
            let mut text = String::new();
            source(&mut text)?;
            let text = text.strip_suffix("# EOF\n").unwrap_or(&text);
            out.push_str(text);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        encode_eof(&mut out)?;
        Ok(out)
    }
}

/// Default buckets (seconds) for block-level durations: 1 ms .. 10 s.
pub(crate) const DURATION_BUCKETS: [f64; 14] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0,
];

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::{encoding::text::encode, metrics::gauge::Gauge};

    #[test]
    fn registry_encodes_with_prefix() {
        let mut r = new_registry();
        let g: Gauge = Gauge::default();
        r.register("height", "Committed height", g.clone());
        g.set(42);
        let mut out = String::new();
        encode(&mut out, &r).unwrap();
        assert!(out.contains("xdevs_height 42"), "{out}");
    }

    #[test]
    fn config_defaults() {
        let c = MetricsConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.listen, "127.0.0.1:9615");
        assert_eq!(c.listen_addr().unwrap().port(), 9615);
        let bad = MetricsConfig {
            listen: "nope".into(),
            ..c
        };
        assert!(bad.listen_addr().is_err());
    }

    #[test]
    fn config_from_toml_like_json() {
        let c: MetricsConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(c.enabled);
        assert_eq!(c.listen, "127.0.0.1:9615");
    }

    #[test]
    fn render_merges_sources_with_single_eof() {
        let reg = MetricsRegistry::new();
        let g: Gauge = Gauge::default();
        reg.with_registry(|r| r.register("height", "Committed height", g.clone()));
        g.set(7);
        // A foreign registry (as Malachite's would be), encoded with its own EOF.
        let mut other = Registry::with_prefix("malachitebft_core_consensus");
        let h: Gauge = Gauge::default();
        other.register("height", "Engine height", h.clone());
        h.set(8);
        let other = Arc::new(RwLock::new(other));
        reg.add_source(Arc::new(move |out: &mut String| {
            encode(out, &other.read().unwrap())
        }));
        let text = reg.render().unwrap();
        assert!(text.contains("xdevs_height 7"), "{text}");
        assert!(
            text.contains("malachitebft_core_consensus_height 8"),
            "{text}"
        );
        assert_eq!(text.matches("# EOF").count(), 1, "{text}");
        assert!(text.ends_with("# EOF\n"));
    }
}
