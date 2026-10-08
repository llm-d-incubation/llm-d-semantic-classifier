//! HTTP server for health and metrics endpoints.
//!
//! Runs alongside the gRPC service on a separate port, exposing `/healthz`
//! (liveness), `/readyz` (readiness), and `/metrics` (classification counters
//! and per-stage latency). Kubernetes probes can target `/healthz` and `/readyz`
//! instead of the current `tcpSocket` workaround on the gRPC port.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::metrics::{LatencyStage, Metrics};

/// Default HTTP listen address for the health/metrics server.
pub const DEFAULT_HTTP_LISTEN: &str = "0.0.0.0:8080";

/// Shared readiness flag, observable from both the HTTP server and the runtime.
///
/// Starts `false` and flips to `true` after the classifier is loaded and
/// warmed. Future work (graceful drain) will flip it back to `false` on
/// shutdown.
#[derive(Clone)]
pub struct SharedReadiness(Arc<AtomicBool>);

impl SharedReadiness {
    pub fn new(ready: bool) -> Self {
        Self(Arc::new(AtomicBool::new(ready)))
    }

    pub fn set(&self, ready: bool) {
        self.0.store(ready, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct AppState {
    metrics: Metrics,
    readiness: SharedReadiness,
}

/// An HTTP server exposing health and metrics endpoints.
pub struct HttpServer {
    addr: SocketAddr,
    _thread: std::thread::JoinHandle<()>,
}

impl HttpServer {
    /// Bind and serve in the background on a dedicated thread.
    ///
    /// The server runs its own single-threaded Tokio runtime — the endpoints
    /// are trivially cheap (a snapshot read and a format), so a multi-threaded
    /// executor would waste more on scheduling than it saves.
    pub fn spawn(listen: &str, metrics: Metrics, readiness: SharedReadiness) -> io::Result<Self> {
        let state = AppState { metrics, readiness };
        let app = Router::new()
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics_handler))
            .with_state(state);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;
        let listener = rt.block_on(tokio::net::TcpListener::bind(listen))?;
        let addr = listener.local_addr()?;

        let thread = std::thread::Builder::new()
            .name("http-server".into())
            .spawn(move || {
                rt.block_on(async {
                    axum::serve(listener, app).await.ok();
                });
            })
            .expect("http server thread must spawn");

        Ok(Self {
            addr,
            _thread: thread,
        })
    }

    /// The actual bound address (resolved after an ephemeral `:0` bind).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Liveness: always 200 if the process can serve HTTP.
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

/// Readiness: 200 when the classifier is loaded and warmed, 503 otherwise.
async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    if state.readiness.is_ready() {
        (StatusCode::OK, "ok\n")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready\n")
    }
}

/// Metrics snapshot in a human-readable format.
///
/// This is a placeholder; the next step replaces it with Prometheus exposition
/// format so the endpoint is scrapable.
async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let snap = state.metrics.snapshot();
    let served = snap.cache_hits + snap.cache_misses + snap.cache_coalesced;

    let q = state.metrics.stage_percentiles(LatencyStage::Queue);
    let t = state.metrics.stage_percentiles(LatencyStage::Tokenize);
    let f = state.metrics.stage_percentiles(LatencyStage::Forward);
    let tot = state.metrics.stage_percentiles(LatencyStage::Total);

    let body = format!(
        "# llm-d-sc metrics snapshot\n\
         # Placeholder format — Prometheus exposition format is next.\n\n\
         served {served}\n\
         cache_hits {}\n\
         cache_misses {}\n\
         cache_coalesced {}\n\
         l2_hits {}\n\
         l2_misses {}\n\
         l2_degraded {}\n\
         queued_expired {}\n\
         queued_cancelled {}\n\n\
         # Per-stage percentiles (microseconds)\n\
         queue_p50_us {}\n\
         queue_p99_us {}\n\
         tokenize_p50_us {}\n\
         tokenize_p99_us {}\n\
         forward_p50_us {}\n\
         forward_p99_us {}\n\
         total_p50_us {}\n\
         total_p99_us {}\n",
        snap.cache_hits,
        snap.cache_misses,
        snap.cache_coalesced,
        snap.l2_hits,
        snap.l2_misses,
        snap.l2_degraded,
        snap.queued_expired,
        snap.queued_cancelled,
        q.p50.as_micros(),
        q.p99.as_micros(),
        t.p50.as_micros(),
        t.p99.as_micros(),
        f.p50.as_micros(),
        f.p99.as_micros(),
        tot.p50.as_micros(),
        tot.p99.as_micros(),
    );
    (StatusCode::OK, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_readiness_starts_at_given_value() {
        let r = SharedReadiness::new(false);
        assert!(!r.is_ready());
        r.set(true);
        assert!(r.is_ready());
    }

    #[test]
    fn http_server_binds_ephemeral_port() {
        let metrics = Metrics::new();
        let readiness = SharedReadiness::new(true);
        let server =
            HttpServer::spawn("127.0.0.1:0", metrics, readiness).expect("must bind ephemeral");
        assert_ne!(server.local_addr().port(), 0);
    }

    #[test]
    fn healthz_returns_200() {
        let metrics = Metrics::new();
        let readiness = SharedReadiness::new(true);
        let server = HttpServer::spawn("127.0.0.1:0", metrics, readiness).unwrap();
        let addr = server.local_addr();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream =
            std::net::TcpStream::connect(addr).expect("must connect to http server");
        std::io::Write::write_all(&mut stream, b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut buf = [0u8; 1024];
        let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
        let response = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "expected 200, got: {response}"
        );
    }

    #[test]
    fn readyz_reflects_readiness() {
        let metrics = Metrics::new();
        let readiness = SharedReadiness::new(false);
        let server =
            HttpServer::spawn("127.0.0.1:0", metrics, readiness.clone()).unwrap();
        let addr = server.local_addr();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let get_readyz = || {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            std::io::Write::write_all(
                &mut stream,
                b"GET /readyz HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
            let mut buf = [0u8; 1024];
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
            std::str::from_utf8(&buf[..n]).unwrap().to_string()
        };

        let resp = get_readyz();
        assert!(
            resp.starts_with("HTTP/1.1 503"),
            "expected 503 when not ready, got: {resp}"
        );

        readiness.set(true);
        let resp = get_readyz();
        assert!(
            resp.starts_with("HTTP/1.1 200"),
            "expected 200 when ready, got: {resp}"
        );
    }

    #[test]
    fn metrics_endpoint_includes_counters() {
        let metrics = Metrics::new();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        let readiness = SharedReadiness::new(true);
        let server = HttpServer::spawn("127.0.0.1:0", metrics, readiness).unwrap();
        let addr = server.local_addr();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        std::io::Write::write_all(
            &mut stream,
            b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .unwrap();
        let mut buf = [0u8; 4096];
        let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
        let response = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(response.contains("cache_hits 1"), "expected cache_hits 1");
        assert!(
            response.contains("cache_misses 1"),
            "expected cache_misses 1"
        );
    }
}
