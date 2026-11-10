//! Request-side DoS limits (plan §19.3, security review "RPC load can starve consensus").
//!
//! * **Per-connection in-flight cap** ([`crate::RpcServerConfig::max_in_flight_per_connection`]):
//!   jsonrpsee runs every WebSocket message as its own task, so without a cap one client
//!   could keep thousands of heavy calls running at once. Calls above the cap fail at once
//!   with -32005 (nothing is queued, so memory stays bounded). An HTTP request (a single
//!   call or a batch, whose entries run one after the other) holds one slot.
//! * **Per-IP token bucket** ([`crate::RpcServerConfig::rate_limit_per_ip`]): every call
//!   (each batch entry too) takes a token from its client IP's bucket; an empty bucket
//!   fails the call with -32005. Behind a reverse proxy or tunnel every client has the
//!   proxy's IP, so rate-limit there (Cloudflare) instead and leave this off.
//!
//! Heavy work is also bounded in the node backend (a global cap on concurrent blocking
//! work and an EVM time limit for `eth_call` / `eth_estimateGas`).

use jsonrpsee::{
    core::{
        middleware::{Batch, BatchEntry, BatchEntryErr, Notification, RpcServiceT},
        server::MethodResponse,
    },
    types::ErrorObjectOwned,
};
use std::{
    collections::HashMap,
    future::Future,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

/// Client IP of a connection, set as an HTTP request extension by the accept loop
/// (jsonrpsee copies it into every call of the connection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RemoteIp(pub(crate) IpAddr);

/// IPs tracked by the rate limiter; idle (full) buckets are pruned first.
const MAX_TRACKED_IPS: usize = 10_000;

/// Error code of every limit here (geth's "limit exceeded").
const LIMIT_EXCEEDED: i32 = -32005;

fn limit_error(msg: String) -> ErrorObjectOwned {
    ErrorObjectOwned::owned::<()>(LIMIT_EXCEEDED, msg, None)
}

/// A token bucket in milli-tokens (integer arithmetic).
#[derive(Clone, Copy, Debug)]
struct Bucket {
    milli: u64,
    last: Instant,
}

/// Per-IP token buckets shared by every connection of a server.
#[derive(Debug)]
pub(crate) struct IpRateLimiter {
    /// Tokens added per second.
    rate: u64,
    /// Bucket size in tokens.
    burst: u64,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl IpRateLimiter {
    /// `rate` calls per second per IP with bursts of `burst` (≥ 1).
    pub(crate) fn new(rate: u32, burst: u32) -> Self {
        Self {
            rate: u64::from(rate.max(1)),
            burst: u64::from(burst.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn refill(&self, b: &mut Bucket, now: Instant) {
        let ms = now.saturating_duration_since(b.last).as_millis();
        let ms = u64::try_from(ms).unwrap_or(u64::MAX);
        // `rate` tokens per second = `rate` milli-tokens per millisecond.
        b.milli = b
            .milli
            .saturating_add(ms.saturating_mul(self.rate))
            .min(self.burst * 1000);
        b.last = now;
    }

    /// Take `n` tokens from `ip`'s bucket; `false` (nothing taken) if it has fewer.
    pub(crate) fn try_take(&self, ip: IpAddr, n: u64, now: Instant) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() >= MAX_TRACKED_IPS && !buckets.contains_key(&ip) {
            // Drop buckets that refilled completely (idle clients), else an arbitrary one.
            let full_after = Duration::from_millis(self.burst * 1000 / self.rate + 1);
            buckets.retain(|_, b| now.saturating_duration_since(b.last) < full_after);
            if buckets.len() >= MAX_TRACKED_IPS {
                if let Some(evict) = buckets.keys().next().copied() {
                    buckets.remove(&evict);
                }
            }
        }
        let bucket = buckets.entry(ip).or_insert(Bucket {
            milli: self.burst * 1000,
            last: now,
        });
        self.refill(bucket, now);
        let cost = n.saturating_mul(1000);
        if bucket.milli >= cost {
            bucket.milli -= cost;
            true
        } else {
            false
        }
    }
}

/// Settings shared by every [`RequestLimits`] instance of a server.
#[derive(Clone, Debug)]
pub(crate) struct LimitsConfig {
    /// Max calls in flight per connection (0 = unlimited).
    pub(crate) max_in_flight: u32,
    /// Per-IP buckets (`None` = no rate limit).
    pub(crate) rate: Option<Arc<IpRateLimiter>>,
}

/// RPC middleware enforcing the limits; one instance per WebSocket connection (or per
/// HTTP request).
#[derive(Clone, Debug)]
pub(crate) struct RequestLimits<S> {
    inner: S,
    in_flight: Option<Arc<Semaphore>>,
    max_in_flight: u32,
    rate: Option<Arc<IpRateLimiter>>,
}

impl<S> RequestLimits<S> {
    pub(crate) fn new(inner: S, config: &LimitsConfig) -> Self {
        Self {
            inner,
            in_flight: (config.max_in_flight > 0)
                .then(|| Arc::new(Semaphore::new(config.max_in_flight as usize))),
            max_in_flight: config.max_in_flight,
            rate: config.rate.clone(),
        }
    }

    fn rate_limited(&self, ext: &http::Extensions, n: u64) -> bool {
        match (&self.rate, ext.get::<RemoteIp>()) {
            (Some(rate), Some(RemoteIp(ip))) => !rate.try_take(*ip, n, Instant::now()),
            _ => false,
        }
    }

    fn in_flight_error(&self) -> ErrorObjectOwned {
        limit_error(format!(
            "too many concurrent requests on this connection (limit {})",
            self.max_in_flight
        ))
    }
}

fn rate_error() -> ErrorObjectOwned {
    limit_error("rate limit exceeded for this IP".into())
}

impl<S> RpcServiceT for RequestLimits<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse, BatchResponse = MethodResponse>
        + Send
        + Sync
        + Clone
        + 'static,
{
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;
    type BatchResponse = S::BatchResponse;

    fn call<'a>(
        &self,
        request: jsonrpsee::types::Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let mut rejected = self.rate_limited(request.extensions(), 1).then(rate_error);
        let mut permit = None;
        if let (None, Some(sem)) = (&rejected, &self.in_flight) {
            match sem.clone().try_acquire_owned() {
                Ok(p) => permit = Some(p),
                Err(_) => rejected = Some(self.in_flight_error()),
            }
        }
        let inner = self.inner.clone();
        async move {
            if let Some(err) = rejected {
                return MethodResponse::error(request.id, err);
            }
            let _permit = permit;
            inner.call(request).await
        }
    }

    fn batch<'a>(
        &self,
        mut requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        // Every entry over the IP budget becomes an error entry; the batch as a whole
        // holds one in-flight slot (its entries run sequentially).
        if self.rate.is_some() {
            for entry in requests.iter_mut() {
                let limited = match entry {
                    Ok(BatchEntry::Call(req)) => self.rate_limited(req.extensions(), 1),
                    _ => false,
                };
                if limited {
                    if let Ok(BatchEntry::Call(req)) = entry {
                        *entry = Err(BatchEntryErr::new(req.id.clone(), rate_error()));
                    }
                }
            }
        }
        let permit = match &self.in_flight {
            Some(sem) => match sem.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => {
                    let err = self.in_flight_error();
                    for entry in requests.iter_mut() {
                        if let Ok(BatchEntry::Call(req)) = entry {
                            *entry = Err(BatchEntryErr::new(req.id.clone(), err.clone()));
                        }
                    }
                    None
                }
            },
            None => None,
        };
        let inner = self.inner.clone();
        async move {
            let _permit = permit;
            inner.batch(requests).await
        }
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(n)
    }
}
