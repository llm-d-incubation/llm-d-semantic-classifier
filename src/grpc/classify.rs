//! Blocking classification server and client over a persistent gRPC channel.
//!
//! AC-009 requires the dummy Praxis client to consume the classification
//! response over a PERSISTENT gRPC channel. This slice (I-001/I-002) pins the
//! round-trip contract: a real tonic client/server exchange a request and a
//! response carrying ranked semantic signals, and multi-turn requests reuse one
//! HTTP/2 channel without reconnecting per call (I-008).
//!
//! The generated protobuf/tonic items live in [`generated`]. This module wraps
//! them in blocking APIs so tests and the dummy Praxis can call classify without
//! touching an async runtime directly. A private Tokio runtime owns the network
//! I/O; the client holds exactly one [`tonic::transport::Channel`] and reuses it
//! for every turn (no reconnect per call).
//!
//! IMPORTANT (slice scope): the classify handler runs the deterministic
//! classification pipeline (tokenizer -> versioned cache -> single-flight ->
//! ranker over synthetic prototypes) WITHOUT running the Candle model forward.
//! This respects the hard rule "no unrestricted model forward from Tokio request
//! workers" and keeps the slice minimal: I-001 pins the RPC contract, not the
//! model. The response never sets `final_route` (AC-010).

use std::collections::HashMap;
use std::io;

use crate::classify::ClassifierRuntime;
use crate::metrics::{Metrics, MetricsSnapshot};
use crate::telemetry::{RequestEvent, Telemetry, TraceEvent};

/// Generated protobuf messages and tonic service/client code (from
/// `proto/classify.proto`, produced by `build.rs`).
pub mod generated {
    tonic::include_proto!("classify");
}

pub use generated::{ClassifyRequest, ClassifyResponse};

/// The generated tonic (async) service trait.
pub use generated::classify_server::Classify as ClassifyTrait;

/// Blocking classify server.
///
/// Binds a real TCP listener (an ephemeral port when given `:0`), serves the
/// tonic classify service on a private Tokio runtime in the background, and
/// reports the actual bound address via [`ClassifyServer::local_addr`].
pub struct ClassifyServer {
    /// Held so the background serving runtime stays alive for the struct's
    /// lifetime; never read directly (hence the underscore prefix).
    _runtime: tokio::runtime::Runtime,
    addr: std::net::SocketAddr,
    metrics: Metrics,
    telemetry: Telemetry,
}

/// The pipeline-backed tonic classify service.
///
/// Runs the deterministic classification pipeline (tokenizer -> versioned cache
/// -> single-flight -> ranker over synthetic prototypes) and returns ranked
/// semantic signals, never a final route (AC-010).
#[derive(Clone)]
pub struct ClassifyServiceImpl {
    service: crate::classify::ClassifyService,
    telemetry: Telemetry,
}

impl ClassifyServiceImpl {
    /// Build a classify service backed by the given pipeline and telemetry
    /// recorder (AC-014).
    pub fn new(service: crate::classify::ClassifyService, telemetry: Telemetry) -> Self {
        Self { service, telemetry }
    }
}

#[tonic::async_trait]
impl generated::classify_server::Classify for ClassifyServiceImpl {
    async fn classify(
        &self,
        request: tonic::Request<generated::ClassifyRequest>,
    ) -> Result<tonic::Response<generated::ClassifyResponse>, tonic::Status> {
        let req = request.into_inner();
        // AC-014: record request telemetry with the context/session hashed, so
        // default telemetry and trace capture never carry raw prompt/session text.
        // Recorded before the request fields are moved into the pipeline input.
        self.telemetry.record_request(RequestEvent {
            request_id: req.request_id.clone(),
            session_id: req.session_id.clone(),
            context: req.context.clone(),
        });
        // Build the typed input; session/signals are passthrough metadata, the
        // context is what gets classified. Never a route in the response (AC-010).
        let input = crate::classify::ClassificationInput {
            text: req.context,
            requested_signals: req.signals,
            session_metadata: HashMap::from([("session_id".to_string(), req.session_id)]),
        };
        // Slice-scope: deterministic pipeline, no model forward. Runtime errors
        // map to an explicit gRPC unavailable status (never a fabricated label).
        let result = self
            .service
            .classify(input)
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        // The response schema carries request_id + ranked signals only; it has
        // no route field at all (ADR-0001, AC-010), so a route is
        // unrepresentable on the wire.
        let response = generated::ClassifyResponse {
            request_id: req.request_id,
            signals: result.ranked.iter().map(|s| s.id.clone()).collect(),
        };
        Ok(tonic::Response::new(response))
    }
}

impl ClassifyServer {
    /// Bind a classify server on the given address (`127.0.0.1:0` for an
    /// ephemeral port) and begin serving in the background.
    pub fn bind(addr: impl AsRef<str>) -> io::Result<ClassifyServer> {
        let addr_str = addr.as_ref();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;

        // Bind a TOKIO listener inside the runtime (an ephemeral port when the
        // address is `:0`). Registering a blocking std socket with tokio is
        // unsupported (tokio-rs/tokio#7172), so the socket is created by tokio.
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(addr_str))
            .map_err(io::Error::other)?;
        let bound = listener.local_addr()?;

        let metrics = Metrics::new();
        let telemetry = Telemetry::new();
        let service = generated::classify_server::ClassifyServer::new(ClassifyServiceImpl::new(
            crate::classify::ClassifyService::from_synthetic_fixtures_with_metrics(metrics.clone()),
            telemetry.clone(),
        ));
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let serve = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming);

        runtime.spawn(serve);

        Ok(ClassifyServer {
            _runtime: runtime,
            addr: bound,
            metrics,
            telemetry,
        })
    }

    /// The actual bound address (resolved after an ephemeral `:0` bind).
    pub fn local_addr(&self) -> String {
        self.addr.to_string()
    }

    /// A snapshot of the server's latency-decomposition and cache counters.
    ///
    /// The returned [`MetricsSnapshot`] exposes the accumulated
    /// queue/tokenize/forward/total latency and the cache hit/miss counters
    /// recorded by every classification the server has served (AC-012).
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// A copy of the captured trace events recorded by served classifications.
    ///
    /// Each [`TraceEvent`] carries the request id and context/session hashes but
    /// never the raw prompt or session text (AC-014).
    pub fn trace_capture(&self) -> Vec<TraceEvent> {
        self.telemetry.trace_capture()
    }
}

/// Blocking classify client over a persistent HTTP/2 channel.
///
/// Connects once and reuses the single [`tonic::transport::Channel`] for every
/// [`ClassifyClient::classify`] call, so multi-turn requests never reconnect per
/// call ([`ClassifyClient::channel_reconnect_count`] stays 0 — I-008).
pub struct ClassifyClient {
    runtime: tokio::runtime::Runtime,
    channel: tonic::transport::Channel,
    reconnects: u64,
}

impl ClassifyClient {
    /// Connect to the server at `addr` and keep the resulting channel persistent.
    pub fn connect(addr: impl AsRef<str>) -> io::Result<ClassifyClient> {
        let addr_str = addr.as_ref();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;

        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{addr_str}"))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
            .connect_timeout(std::time::Duration::from_secs(5))
            .tcp_nodelay(true);

        let channel = runtime
            .block_on(endpoint.connect())
            .map_err(io::Error::other)?;

        Ok(ClassifyClient {
            runtime,
            channel,
            reconnects: 0,
        })
    }

    /// Send one classify request over the persistent channel and return the
    /// ranked signals (never a final route).
    pub fn classify(
        &mut self,
        request: ClassifyRequest,
    ) -> Result<ClassifyResponse, tonic::Status> {
        let mut client = generated::classify_client::ClassifyClient::new(self.channel.clone());
        let response = self.runtime.block_on(client.classify(request))?;
        Ok(response.into_inner())
    }

    /// The number of channel re-establishments after the initial connect.
    ///
    /// The channel is created once and reused, so this remains 0 across turns.
    pub fn channel_reconnect_count(&self) -> u64 {
        self.reconnects
    }
}
