//! JSON-RPC metrics (plan §24.7): `xdevs_rpc_requests_total{method,result}` and
//! `xdevs_rpc_latency_seconds{method}`, fed by `xdevs-rpc`.

use crate::{MetricsRegistry, DURATION_BUCKETS};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, histogram::Histogram},
};
use std::time::Duration;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RequestLabels {
    method: String,
    result: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct MethodLabels {
    method: String,
}

/// Longest method name kept as a label; longer (or non-ASCII) names are reported as
/// `"other"` so a client cannot create unbounded label values.
const MAX_METHOD_LEN: usize = 64;

/// RPC request metrics. Clones share the same underlying metrics.
#[derive(Clone, Debug)]
pub struct RpcMetrics {
    requests: Family<RequestLabels, Counter>,
    latency: Family<MethodLabels, Histogram>,
}

impl RpcMetrics {
    /// Create and register the RPC metrics in `registry`.
    pub fn register(registry: &MetricsRegistry) -> Self {
        let requests = Family::<RequestLabels, Counter>::default();
        let latency = Family::<MethodLabels, Histogram>::new_with_constructor(
            (|| Histogram::new(DURATION_BUCKETS)) as fn() -> Histogram,
        );
        registry.with_registry(|r| {
            r.register(
                "rpc_requests",
                "JSON-RPC requests by method and result (ok or error)",
                requests.clone(),
            );
            r.register(
                "rpc_latency_seconds",
                "JSON-RPC request latency by method",
                latency.clone(),
            );
        });
        Self { requests, latency }
    }

    /// Record one finished request.
    pub fn observe(&self, method: &str, ok: bool, elapsed: Duration) {
        let method = label_method(method);
        self.requests
            .get_or_create(&RequestLabels {
                method: method.clone(),
                result: if ok { "ok" } else { "error" },
            })
            .inc();
        self.latency
            .get_or_create(&MethodLabels { method })
            .observe(elapsed.as_secs_f64());
    }
}

/// Bounded label value for a method name.
fn label_method(method: &str) -> String {
    let valid = !method.is_empty()
        && method.len() <= MAX_METHOD_LEN
        && method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if valid {
        method.to_owned()
    } else {
        "other".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_requests_and_latency() {
        let reg = MetricsRegistry::new();
        let m = RpcMetrics::register(&reg);
        m.observe("eth_blockNumber", true, Duration::from_millis(3));
        m.observe("eth_blockNumber", false, Duration::from_millis(5));
        m.observe("eth_call", true, Duration::from_millis(20));
        let text = reg.render().unwrap();
        assert!(
            text.contains(r#"xdevs_rpc_requests_total{method="eth_blockNumber",result="ok"} 1"#),
            "{text}"
        );
        assert!(
            text.contains(r#"xdevs_rpc_requests_total{method="eth_blockNumber",result="error"} 1"#),
            "{text}"
        );
        assert!(
            text.contains(r#"xdevs_rpc_latency_seconds_count{method="eth_call"} 1"#),
            "{text}"
        );
    }

    #[test]
    fn untrusted_method_names_are_bounded() {
        assert_eq!(label_method("eth_getLogs"), "eth_getLogs");
        assert_eq!(label_method(""), "other");
        assert_eq!(label_method("eth_\"injected\""), "other");
        assert_eq!(label_method(&"a".repeat(MAX_METHOD_LEN + 1)), "other");
    }
}
