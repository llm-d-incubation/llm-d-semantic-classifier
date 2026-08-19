//! Blocking classification server and client over a persistent gRPC channel.
//!
//! AC-009 requires the dummy gateway client to consume the classification
//! response over a PERSISTENT gRPC channel. This slice (I-001/I-002) pins the
//! round-trip contract: a real tonic client/server exchange a request and a
//! response carrying ranked semantic signals, and multi-turn requests reuse one
//! HTTP/2 channel without reconnecting per call (I-008).
//!
//! The generated protobuf/tonic items live in [`generated`]. This module wraps
//! them in blocking APIs so tests and the dummy gateway can call classify without
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
use std::sync::Arc;

use crate::classify::ClassifyError;
use crate::handoff::InferenceExecutor;
use crate::metrics::{Metrics, MetricsSnapshot};
use crate::telemetry::{RequestEvent, Telemetry, TraceEvent};

/// Default bound on total admitted (in-flight + queued) inference work.
///
/// AC-008 / ADR-0002: the queue is IN the request path, and in-flight + queued
/// work never exceeds this bound. Generous enough that normal local benchmark
/// concurrency (P-020 concurrency 1 / P-021 concurrency 4) is never rejected.
pub const DEFAULT_QUEUE_BOUND: usize = 256;

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
    readiness: crate::runtime::Readiness,
    accepted: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// The runtime-backed tonic classify service.
///
/// Serves ANY [`ClassifierRuntime`] — the deterministic synthetic pipeline (for
/// tests that must run without weights) or the resident Candle classifier (the
/// production served path). It returns ranked semantic signals, never a final
/// route (AC-010).
///
/// AC-008 / ADR-0002: the model forward does NOT run on a Tokio network worker.
/// Every classify request is handed over a BOUNDED handoff to a dedicated
/// inference executor thread ([`InferenceExecutor`]) that performs the forward
/// and returns the result via a oneshot. Queue-full admission is rejected
/// explicitly with tonic `resource_exhausted`; in-flight + queued work never
/// exceeds the configured bound.
pub struct ClassifyServiceImpl<R> {
    telemetry: Telemetry,
    executor: Arc<InferenceExecutor<crate::classify::ServiceCore<R>>>,
}

impl<R> Clone for ClassifyServiceImpl<R> {
    /// Clone shares the same telemetry recorder and the SAME dedicated inference
    /// executor (via [`Arc`]) — cloning never spawns another executor thread.
    fn clone(&self) -> Self {
        Self {
            telemetry: self.telemetry.clone(),
            executor: self.executor.clone(),
        }
    }
}

impl<R> ClassifyServiceImpl<R>
where
    R: crate::classify::ClassifierRuntime + Send + Sync + 'static,
{
    /// Build a classify service backed by the given raw backend and telemetry
    /// recorder (AC-014), with a dedicated inference executor and the default
    /// queue bound. The raw backend is wrapped in the generic [`ServiceCore`],
    /// so EVERY backend (synthetic or Candle) inherits the exact-result cache,
    /// single-flight coalescing, metrics, and error behaviour.
    pub fn new(service: R, telemetry: Telemetry) -> Self {
        Self::with_executor(service, telemetry, Metrics::new(), DEFAULT_QUEUE_BOUND)
    }

    /// Build a classify service whose dedicated inference executor records
    /// queue wait into `metrics` and bounds total admitted (in-flight + queued)
    /// work to `bound`.
    ///
    /// The raw backend `service` is wrapped in the shared [`ServiceCore`], which
    /// owns the exact-result cache and the hit/miss/total/queue metrics. The
    /// caller supplies the [`Metrics`] handle so the core's cache counters and
    /// the executor's queue-wait recording are the SAME registry the server
    /// snapshots (AC-008/AC-012). The raw backend must share that registry for
    /// its tokenize/forward stage recording.
    pub fn with_executor(service: R, telemetry: Telemetry, metrics: Metrics, bound: usize) -> Self {
        let core = crate::classify::ServiceCore::with_metrics(service, metrics.clone());
        let executor = InferenceExecutor::spawn(core, metrics, bound);
        Self {
            telemetry,
            executor: Arc::new(executor),
        }
    }

    /// The configured bound on total admitted (in-flight + queued) work.
    pub fn queue_bound(&self) -> usize {
        self.executor.bound()
    }

    /// The observed maximum total admitted (in-flight + queued) work. AC-008:
    /// this must never exceed [`ClassifyServiceImpl::queue_bound`].
    pub fn max_admitted(&self) -> usize {
        self.executor.max_admitted()
    }
}

#[tonic::async_trait]
impl<R> generated::classify_server::Classify for ClassifyServiceImpl<R>
where
    R: crate::classify::ClassifierRuntime + Send + Sync + 'static,
{
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
        // U-011: only the supported 'sensitivity' signal is accepted; any other
        // requested signal is rejected explicitly with invalid_argument, never
        // silently ignored.
        const SUPPORTED_SIGNAL: &str = "sensitivity";
        for signal in &req.signals {
            if signal != SUPPORTED_SIGNAL {
                return Err(tonic::Status::invalid_argument(format!(
                    "unsupported signal '{signal}'; only '{SUPPORTED_SIGNAL}' is supported"
                )));
            }
        }
        // Build the typed input; session/signals are passthrough metadata, the
        // context is what gets classified. Never a route in the response (AC-010).
        let input = crate::classify::ClassificationInput {
            text: req.context,
            requested_signals: req.signals,
            session_metadata: HashMap::from([("session_id".to_string(), req.session_id)]),
        };
        // AC-008 / ADR-0002: hand the job to the dedicated inference executor
        // over a BOUNDED handoff. The model forward does NOT run on this Tokio
        // network worker. A full queue rejects admission explicitly with
        // resource_exhausted (never unboundedly buffered).
        let respond = self
            .executor
            .try_enqueue(input)
            .map_err(|_| tonic::Status::resource_exhausted("inference queue is full"))?;
        // Await the dedicated executor's forward result (returned via oneshot).
        let result = respond
            .await
            .map_err(|_| tonic::Status::unavailable("inference executor stopped"))?;
        // Runtime errors map to an explicit gRPC status (never a fabricated
        // label). The served runtime is whichever was bound (the resident Candle
        // classifier in production, the deterministic synthetic pipeline in
        // weight-free tests). A pipeline-reported resource exhaustion is explicit
        // resource_exhausted, not a generic unavailable.
        let result = match result {
            Err(e) => {
                return Err(match e {
                    ClassifyError::ResourceExhausted => {
                        tonic::Status::resource_exhausted("inference queue is full")
                    }
                    _ => tonic::Status::unavailable(e.to_string()),
                })
            }
            Ok(result) => result,
        };
        // Map the typed result's status onto the wire ClassificationStatus.
        let status = match result.status {
            crate::classify::ClassifyStatus::Ok => generated::ClassificationStatus::Ok,
            crate::classify::ClassifyStatus::Abstain => generated::ClassificationStatus::Abstain,
            crate::classify::ClassifyStatus::Error => generated::ClassificationStatus::Unavailable,
        };
        // The response carries request_id, classifier_id, the exact revision
        // fingerprint fields, the status, and the ranked signals with scores. It
        // has no route field at all (ADR-0001, AC-010), so a route is
        // unrepresentable on the wire.
        let ranked = result
            .ranked
            .iter()
            .map(|s| generated::RankedSignal {
                label: s.id.clone(),
                score: s.score as f32,
            })
            .collect();
        let response = generated::ClassifyResponse {
            request_id: req.request_id,
            classifier_id: result.classifier_id,
            model_revision: result.model_revision,
            tokenizer_revision: result.tokenizer_revision,
            taxonomy_revision: result.taxonomy_revision,
            status: status as i32,
            ranked,
        };
        Ok(tonic::Response::new(response))
    }
}

impl ClassifyServer {
    /// Bind a classify server on the given address (`127.0.0.1:0` for an
    /// ephemeral port) and begin serving in the background.
    ///
    /// TEST-ONLY synthetic path: serves the deterministic pipeline so tests that
    /// must run without model weights can exercise the full gRPC contract. The
    /// production binary uses [`ClassifyServer::bind_with_classifier`] instead.
    pub fn bind(addr: impl AsRef<str>) -> io::Result<ClassifyServer> {
        let metrics = Metrics::new();
        let telemetry = Telemetry::new();
        let service = ClassifyServiceImpl::with_executor(
            crate::classify::ClassifyService::from_synthetic_fixtures_with_metrics(metrics.clone()),
            telemetry.clone(),
            metrics.clone(),
            DEFAULT_QUEUE_BOUND,
        );
        Self::serve(
            addr,
            service,
            metrics,
            telemetry,
            crate::runtime::Readiness::Ready,
        )
    }

    /// Bind a classify server that records its latency/cache counters into the
    /// CALLER-SUPPLIED [`Metrics`] handle.
    ///
    /// TEST-ONLY synthetic path (deterministic pipeline, no model forward). The
    /// benchmark harness shares this same [`Metrics`] clone so it can PROVE its
    /// own methodology: capturing the service's `cache_hits`/`cache_misses`
    /// deltas around a measured window and asserting they equal the measured
    /// request count (see `llm_d_sc::bench`).
    pub fn bind_with_metrics(
        addr: impl AsRef<str>,
        metrics: Metrics,
    ) -> io::Result<ClassifyServer> {
        let telemetry = Telemetry::new();
        let service = ClassifyServiceImpl::with_executor(
            crate::classify::ClassifyService::from_synthetic_fixtures_with_metrics(metrics.clone()),
            telemetry.clone(),
            metrics.clone(),
            DEFAULT_QUEUE_BOUND,
        );
        Self::serve(
            addr,
            service,
            metrics,
            telemetry,
            crate::runtime::Readiness::Ready,
        )
    }

    /// Bind a classify server serving the RESIDENT Candle classifier.
    ///
    /// Production served path (AC-002/AC-003): the classifier must already be
    /// loaded AND warmed (via [`crate::classify::load_and_warm_modelcar`]) —
    /// a directory that merely exists never reaches here because warmup fails
    /// first. The server therefore reports READY. It begins serving in the
    /// background and returns on an ephemeral port when given `:0`.
    pub fn bind_with_classifier(
        addr: impl AsRef<str>,
        classifier: crate::classify::CandleClassifier,
    ) -> io::Result<ClassifyServer> {
        // The server surface shares the classifier's own metrics handle, so the
        // cache-hit/miss counters and the tokenize/forward stages recorded by the
        // real Candle forward are visible to a benchmark harness (AC-012).
        let metrics = classifier.metrics();
        let telemetry = Telemetry::new();
        let service = ClassifyServiceImpl::with_executor(
            classifier,
            telemetry.clone(),
            metrics.clone(),
            DEFAULT_QUEUE_BOUND,
        );
        Self::serve(
            addr,
            service,
            metrics,
            telemetry,
            crate::runtime::Readiness::Ready,
        )
    }

    /// Bind and serve any tonic classify service on a private Tokio runtime.
    fn serve<R>(
        addr: impl AsRef<str>,
        service: ClassifyServiceImpl<R>,
        metrics: Metrics,
        telemetry: Telemetry,
        readiness: crate::runtime::Readiness,
    ) -> io::Result<ClassifyServer>
    where
        R: crate::classify::ClassifierRuntime + Send + Sync + 'static,
    {
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

        let service = generated::classify_server::ClassifyServer::new(service);
        // I-008 evidence: count every ACCEPTED TCP connection. A client that
        // reuses one persistent HTTP/2 channel across N calls produces exactly
        // ONE accept; a client that reconnects per call produces N. Measuring
        // this at the accept boundary observes the property from OUTSIDE the
        // client, so the client cannot assert its own good behaviour.
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let accept_counter = accepted.clone();
        let incoming = tokio_stream::StreamExt::map(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            move |conn| {
                if conn.is_ok() {
                    accept_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                conn
            },
        );
        let serve = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming);

        runtime.spawn(serve);

        Ok(ClassifyServer {
            _runtime: runtime,
            addr: bound,
            metrics,
            telemetry,
            readiness,
            accepted,
        })
    }

    /// Current readiness.
    ///
    /// A successfully bound server reports READY; a real model dir that fails
    /// load/warmup never constructs a server, so readiness is never claimed for
    /// a directory that merely exists (AC-002).
    pub fn readiness(&self) -> crate::runtime::Readiness {
        self.readiness
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

    /// A SHARED handle to the server's metrics registry.
    ///
    /// Cloning the handle shares the same counters/accumulators, so a caller
    /// (e.g. the benchmark runner) can pass it to [`BenchmarkRun::with_metrics`]
    /// to prove its own methodology and capture the stage decomposition over a
    /// measured window.
    pub fn metrics(&self) -> Metrics {
        self.metrics.clone()
    }

    /// The number of TCP connections this server has ACCEPTED.
    ///
    /// I-008 is the claim that multi-turn requests do not reconnect per call.
    /// That claim is about the transport, so it is measured at the transport:
    /// N classify calls over one persistent channel must accept exactly ONE
    /// connection.
    pub fn accepted_connection_count(&self) -> u64 {
        self.accepted.load(std::sync::atomic::Ordering::SeqCst)
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
/// [`ClassifyClient::classify`] call. The no-reconnect claim (I-008) is proven
/// SERVER-side by [`ClassifyServer::accepted_connection_count`], not by a
/// counter this client keeps about itself.
pub struct ClassifyClient {
    runtime: tokio::runtime::Runtime,
    channel: tonic::transport::Channel,
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

        Ok(ClassifyClient { runtime, channel })
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

}
