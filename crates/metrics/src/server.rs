//! Minimal HTTP/1.1 `/metrics` endpoint for Prometheus scrapes.
//!
//! Deliberately tiny: one request per connection, a bounded request size and a read
//! timeout, `GET /metrics` only. It should listen on a private address (plan §24.3: port
//! 9100/9615 on the private subnet), never on the public internet.

use crate::{MetricsRegistry, CONTENT_TYPE};
use std::{io, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};
use tracing::debug;

/// Largest request head accepted (Prometheus sends a few hundred bytes).
const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Time allowed to read the request head.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A running `/metrics` server.
#[derive(Debug)]
pub struct MetricsServer {
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl MetricsServer {
    /// Bind `addr` and serve `registry` until [`MetricsServer::shutdown`].
    pub async fn start(addr: SocketAddr, registry: MetricsRegistry) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (shutdown, mut stop) = watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let registry = registry.clone();
                            tokio::spawn(async move {
                                if let Err(e) = serve(stream, &registry).await {
                                    debug!(target: "xdevs::metrics", "metrics request failed: {e}");
                                }
                            });
                        }
                        Err(e) => debug!(target: "xdevs::metrics", "accept failed: {e}"),
                    },
                }
            }
        });
        Ok(Self {
            local_addr,
            shutdown,
            task,
        })
    }

    /// The bound address (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop accepting connections and wait for the accept loop to end.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
}

async fn serve(mut stream: TcpStream, registry: &MetricsRegistry) -> io::Result<()> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    let head_complete = tokio::time::timeout(READ_TIMEOUT, async {
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Ok::<bool, io::Error>(false);
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                return Ok(true);
            }
            if buf.len() > MAX_REQUEST_BYTES {
                return Ok(false);
            }
        }
    })
    .await
    .unwrap_or(Ok(false))?;

    let (status, content_type, body) = if !head_complete {
        ("400 Bad Request", "text/plain", "bad request\n".to_owned())
    } else if is_get_metrics(&buf) {
        match registry.render() {
            Ok(text) => ("200 OK", CONTENT_TYPE, text),
            Err(_) => (
                "500 Internal Server Error",
                "text/plain",
                "encoding failed\n".to_owned(),
            ),
        }
    } else {
        ("404 Not Found", "text/plain", "not found\n".to_owned())
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// `GET /metrics` (optionally with a query string).
fn is_get_metrics(head: &[u8]) -> bool {
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(head.len());
    let mut parts = head[..line_end].split(|b| *b == b' ');
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return false;
    };
    let path = target.split(|b| *b == b'?').next().unwrap_or_default();
    method == b"GET" && path == b"/metrics"
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::metrics::gauge::Gauge;

    async fn request(addr: SocketAddr, raw: &str) -> String {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(raw.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn serves_metrics_and_404s_everything_else() {
        let reg = MetricsRegistry::new();
        let g: Gauge = Gauge::default();
        reg.with_registry(|r| r.register("block_height", "Committed height", g.clone()));
        g.set(12);
        let server = MetricsServer::start("127.0.0.1:0".parse().unwrap(), reg)
            .await
            .unwrap();
        let addr = server.local_addr();

        let ok = request(addr, "GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(ok.starts_with("HTTP/1.1 200 OK"), "{ok}");
        assert!(ok.contains("xdevs_block_height 12"), "{ok}");
        assert!(ok.trim_end().ends_with("# EOF"), "{ok}");

        let q = request(addr, "GET /metrics?x=1 HTTP/1.1\r\n\r\n").await;
        assert!(q.starts_with("HTTP/1.1 200 OK"), "{q}");
        let nf = request(addr, "GET / HTTP/1.1\r\n\r\n").await;
        assert!(nf.starts_with("HTTP/1.1 404"), "{nf}");
        let post = request(addr, "POST /metrics HTTP/1.1\r\n\r\n").await;
        assert!(post.starts_with("HTTP/1.1 404"), "{post}");

        server.shutdown().await;
    }

    #[test]
    fn request_line_parsing() {
        assert!(is_get_metrics(b"GET /metrics HTTP/1.1\r\n\r\n"));
        assert!(!is_get_metrics(b"GET /metricsX HTTP/1.1\r\n\r\n"));
        assert!(!is_get_metrics(b""));
    }
}
