//! Minimal CORS middleware for the HTTP transport (plan §19.3).
//!
//! Browser wallets and dapps (MetaMask, viem in a page) send `Origin` headers and
//! preflight `OPTIONS` requests. This layer answers preflights itself and adds
//! `Access-Control-Allow-Origin` to every response whose origin is allowed. It is a
//! few dozen lines on top of `tower` + `http` (already in the graph via jsonrpsee),
//! which avoids an extra `tower-http` dependency.

use http::{
    header::{
        ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
        ORIGIN, VARY,
    },
    HeaderMap, HeaderValue, Method, Request, StatusCode,
};
use jsonrpsee::{
    core::BoxError,
    server::{HttpBody, HttpResponse},
};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

/// Which origins may call the server from a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CorsPolicy {
    /// `*`: every origin (local dev).
    Any,
    /// Only these origins (compared case-insensitively, no trailing slash).
    List(Vec<String>),
}

impl CorsPolicy {
    /// Build from the `[rpc] cors` setting; `None` = CORS disabled (no headers).
    pub(crate) fn from_config(origins: &[String]) -> Option<Self> {
        if origins.is_empty() {
            None
        } else if origins.iter().any(|o| o.trim() == "*") {
            Some(Self::Any)
        } else {
            Some(Self::List(
                origins
                    .iter()
                    .map(|o| o.trim().trim_end_matches('/').to_ascii_lowercase())
                    .collect(),
            ))
        }
    }

    /// Value for `Access-Control-Allow-Origin`, if the request's origin is allowed.
    fn allow_origin(&self, headers: &HeaderMap) -> Option<HeaderValue> {
        match self {
            Self::Any => Some(HeaderValue::from_static("*")),
            Self::List(list) => {
                let origin = headers.get(ORIGIN)?;
                let lower = origin
                    .to_str()
                    .ok()?
                    .trim_end_matches('/')
                    .to_ascii_lowercase();
                list.contains(&lower).then(|| origin.clone())
            }
        }
    }
}

/// Tower layer installing [`Cors`].
#[derive(Debug, Clone)]
pub(crate) struct CorsLayer {
    policy: Arc<Option<CorsPolicy>>,
}

impl CorsLayer {
    /// `None` disables CORS (requests pass through untouched).
    pub(crate) fn new(policy: Option<CorsPolicy>) -> Self {
        Self {
            policy: Arc::new(policy),
        }
    }
}

impl<S> Layer<S> for CorsLayer {
    type Service = Cors<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Cors {
            inner,
            policy: self.policy.clone(),
        }
    }
}

/// CORS service wrapping the jsonrpsee HTTP service.
#[derive(Debug, Clone)]
pub(crate) struct Cors<S> {
    inner: S,
    policy: Arc<Option<CorsPolicy>>,
}

impl<S, B> Service<Request<B>> for Cors<S>
where
    S: Service<Request<B>, Response = HttpResponse, Error = BoxError>,
    S::Future: Send + 'static,
{
    type Response = HttpResponse;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let Some(policy) = self.policy.as_ref() else {
            return Box::pin(self.inner.call(req));
        };
        let allow = policy.allow_origin(req.headers());
        let vary = matches!(policy, CorsPolicy::List(_));

        if req.method() == Method::OPTIONS
            && req.headers().contains_key(ACCESS_CONTROL_REQUEST_METHOD)
        {
            let response = preflight(
                allow,
                vary,
                req.headers().get(ACCESS_CONTROL_REQUEST_HEADERS),
            );
            return Box::pin(async move { Ok(response) });
        }

        let fut = self.inner.call(req);
        Box::pin(async move {
            let mut response = fut.await?;
            if let Some(origin) = allow {
                response
                    .headers_mut()
                    .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
            }
            if vary {
                response
                    .headers_mut()
                    .append(VARY, HeaderValue::from_static("origin"));
            }
            Ok(response)
        })
    }
}

/// Response to a CORS preflight (`OPTIONS` with `Access-Control-Request-Method`).
fn preflight(
    allow: Option<HeaderValue>,
    vary: bool,
    requested_headers: Option<&HeaderValue>,
) -> HttpResponse {
    let mut response = HttpResponse::new(HttpBody::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    if vary {
        headers.insert(VARY, HeaderValue::from_static("origin"));
    }
    if let Some(origin) = allow {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("POST, GET, OPTIONS"),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            requested_headers
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("content-type")),
        );
        headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_origin(origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
        h
    }

    #[test]
    fn policy_from_config() {
        assert_eq!(CorsPolicy::from_config(&[]), None);
        assert_eq!(
            CorsPolicy::from_config(&["https://a.io".into(), "*".into()]),
            Some(CorsPolicy::Any)
        );
        assert_eq!(
            CorsPolicy::from_config(&["https://App.xdevs.io/".into()]),
            Some(CorsPolicy::List(vec!["https://app.xdevs.io".into()]))
        );
    }

    #[test]
    fn allow_origin_matching() {
        let any = CorsPolicy::Any;
        assert_eq!(
            any.allow_origin(&HeaderMap::new()),
            Some(HeaderValue::from_static("*"))
        );

        let list = CorsPolicy::from_config(&["https://app.xdevs.io".into()]).unwrap();
        let ok = headers_with_origin("https://APP.xdevs.io");
        assert_eq!(
            list.allow_origin(&ok),
            Some(HeaderValue::from_static("https://APP.xdevs.io"))
        );
        assert_eq!(
            list.allow_origin(&headers_with_origin("https://evil.io")),
            None
        );
        assert_eq!(list.allow_origin(&HeaderMap::new()), None);
    }

    #[test]
    fn preflight_headers() {
        let r = preflight(
            Some(HeaderValue::from_static("*")),
            false,
            Some(&HeaderValue::from_static("content-type, x-foo")),
        );
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(r.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            r.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
            "content-type, x-foo"
        );

        let denied = preflight(None, true, None);
        assert!(denied.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }
}
