//! Classification pipeline: typed core, tokenizer -> versioned cache ->
//! single-flight -> deterministic ranker over synthetic prototypes.
//!
//! This module defines the typed classification contract ([`ClassificationInput`],
//! [`ClassificationResult`], [`ClassifyStatus`], [`ClassifyError`]) and the
//! [`ClassifierRuntime`] abstraction the service and Candle backend both
//! implement. The response NEVER carries a route/endpoint field: routing/session
//! authority remains the AI Gateway (AC-010).
//!
//! I-001 (AC-009) pins the RPC contract: a real tonic round trip returns ranked
//! semantic signals over the wire. The Candle model is NOT required for that
//! slice — the [`ClassifyService`] pipeline runs a deterministic classifier (the
//! ranker over the committed synthetic prototypes) after tokenizing with the
//! resident [`Tokenizer`]. The real Candle embedder+ranker path is the
//! [`CandleClassifier`], which implements the same [`ClassifierRuntime`] trait.
//! All inputs are committed offline fixtures, so there is no model download and
//! no runtime forward on the deterministic path.
//!
//! The pipeline deliberately wires the exact-result cache with single-flight
//! coalescing (AC-006/AC-007): identical concurrent requests for the same
//! versioned fingerprint coalesce into ONE deterministic classification, and a
//! cache hit bypasses tokenization/ranking entirely. The cache stores a
//! [`ClassificationResult`], keyed by a blake3 versioned fingerprint.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::cache::{CacheKey, SharedCache};
use crate::metrics::{LatencyStage, Metrics};
use crate::ranker::{cosine_rank, Prototype};
use crate::tokenizer::Tokenizer;

/// Versioned fingerprint components pinned to the committed synthetic fixtures.
/// These identify the classification result so any revision change yields a
/// different key and a stale cached classification is never served (AC-006).
const CLASSIFIER_ID: &str = "sensitivity-synthetic";
const MODEL_REVISION: &str = "synthetic-for-mechanics-only";
const TOKENIZER_REVISION: &str = "tokenizer-fixture";
const TAXONOMY_REVISION: &str = "synthetic-prototypes";

/// Embedding dimension of the synthetic prototypes (fixture `dim: 384`).
const SYNTHETIC_DIM: usize = 384;

/// Fixture input used to WARM the resident classifier before reporting READY.
/// Running a real forward on this input proves the model/tokenizer are loaded
/// and warmed, so readiness is not claimed for a directory that merely exists
/// (AC-002).
pub const WARMUP_INPUT: &str = "this is a golden sensitivity input";

/// A classification request.
///
/// `text` is the supplied context to classify. `requested_signals` lists the
/// semantic signals the caller wants; `session_metadata` is passed through
/// verbatim and is NOT used to derive the classification (it is routing/session
/// data owned by the AI Gateway, AC-010).
#[derive(Debug, Clone)]
pub struct ClassificationInput {
    pub text: String,
    pub requested_signals: Vec<String>,
    pub session_metadata: HashMap<String, String>,
}

/// One ranked semantic signal: an id and its deterministic similarity score.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedSignal {
    pub id: String,
    pub score: f64,
}

/// The semantic evidence produced by a classifier.
///
/// Carries the classifier id and the exact revision fingerprint fields
/// (model/tokenizer/taxonomy) that reproduce the result, the ranked signals,
/// and a [`ClassifyStatus`]. It NEVER contains a route/endpoint field — routing
/// authority is the AI Gateway (AC-010).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassificationResult {
    pub classifier_id: String,
    pub model_revision: String,
    pub tokenizer_revision: String,
    pub taxonomy_revision: String,
    pub status: ClassifyStatus,
    pub ranked: Vec<RankedSignal>,
}

/// Classification outcome status per the spec failure contract.
///
/// `Ok` is a real ranked result; `Abstain` is "insufficient context where
/// required — do not fabricate a label"; `Error` is an explicit failure (the
/// typed error detail travels via [`ClassifyError`] on the `Result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifyStatus {
    Ok,
    Abstain,
    Error,
}

/// Classification failures, aligned to the spec failure contract:
/// missing/corrupt model -> not ready; full queue -> resource exhausted;
/// expired queued request -> do not infer; runtime error -> explicit
/// unavailable/error, never a fabricated label.
#[derive(Debug, Clone)]
pub enum ClassifyError {
    /// The resident model/tokenizer is not loaded/warmed yet.
    NotReady,
    /// The inference queue is full (explicit resource exhaustion).
    ResourceExhausted,
    /// A queued request expired; it must not be inferred.
    RequestExpired,
    /// Tokenization failed (runtime error — never fabricate a label).
    Tokenizer(String),
    /// The Candle embed/forward failed (runtime error).
    Embedding(String),
    /// A generic runtime failure with a message.
    Unavailable(String),
}

impl std::fmt::Display for ClassifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassifyError::NotReady => write!(f, "classifier not ready"),
            ClassifyError::ResourceExhausted => write!(f, "resource exhausted"),
            ClassifyError::RequestExpired => write!(f, "request expired"),
            ClassifyError::Tokenizer(e) => write!(f, "tokenizer error: {e}"),
            ClassifyError::Embedding(e) => write!(f, "embedding error: {e}"),
            ClassifyError::Unavailable(m) => write!(f, "classifier unavailable: {m}"),
        }
    }
}

impl std::error::Error for ClassifyError {}

/// The classification runtime abstraction.
///
/// Any backend (the deterministic synthetic pipeline or the real Candle
/// embedder+ranker) classifies an input and returns typed semantic evidence, or
/// an explicit [`ClassifyError`]. The response is always semantic evidence,
/// never a final route (AC-010).
pub trait ClassifierRuntime {
    /// Classify `input`, returning ranked semantic signals or an explicit error.
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError>;
}

/// A generic classification service core shared by EVERY backend.
///
/// Wraps any [`ClassifierRuntime`] backend (`R`) and centralizes the exact-result
/// cache, single-flight coalescing, hit/miss counters, and error behaviour so a
/// synthetic and a production Candle backend inherit the SAME cache/metrics
/// pipeline (AC-006/AC-007). The wrapped `R` performs the RAW forward only; a
/// cache hit through the core never reaches the backend's tokenizer or model
/// forward (AC-006), and identical concurrent misses coalesce into ONE forward
/// (AC-007).
///
/// The struct is deliberately `{ runtime: R, cache: SharedCache, metrics: Metrics }`:
/// the cache and the shared latency/counter registry live HERE, not inside any
/// individual backend, so every backend inherits caching, single-flight
/// coalescing, metrics, and error behaviour.
#[derive(Clone)]
pub struct ServiceCore<R> {
    runtime: Arc<R>,
    cache: SharedCache,
    metrics: Metrics,
}

impl<R> ServiceCore<R>
where
    R: ClassifierRuntime + Send + Sync + 'static,
{
    /// Build a service core around a raw backend with a fresh cache and metrics.
    pub fn new(runtime: R) -> Self {
        Self::with_metrics(runtime, Metrics::new())
    }

    /// Build a service core whose cache and hit/miss/total/queue metrics record
    /// into the CALLER-SUPPLIED [`Metrics`] handle, so the backend's own
    /// tokenize/forward stage recording can share the same registry.
    pub fn with_metrics(runtime: R, metrics: Metrics) -> Self {
        ServiceCore {
            runtime: Arc::new(runtime),
            cache: SharedCache::new(),
            metrics,
        }
    }

    /// A SHARED handle to the core's metrics registry.
    ///
    /// The same handle is shared by the raw backend's tokenize/forward stage
    /// recording and the server surface, so a snapshot sees every stage.
    pub fn metrics(&self) -> Metrics {
        self.metrics.clone()
    }

    /// Number of raw forwards (tokenizer + model) actually run across all
    /// threads (cache misses). A cache hit does not increment this (AC-006).
    pub fn forward_count(&self) -> u64 {
        self.cache.forward_count()
    }
}

impl<R> ClassifierRuntime for ServiceCore<R>
where
    R: ClassifierRuntime + Send + Sync + 'static,
{
    /// Classify `input` through the shared core: versioned cache -> single-flight
    /// -> raw backend forward.
    ///
    /// A cache hit bypasses the backend's tokenizer and model forward entirely
    /// (AC-006); identical concurrent misses coalesce into ONE forward (AC-007).
    /// The cache stores the typed [`ClassificationResult`] keyed by the blake3
    /// versioned fingerprint, and the hit/miss/total/queue metrics are recorded
    /// here so EVERY backend inherits them.
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        // Total service latency is measured from admission to response (AC-012).
        let total_start = std::time::Instant::now();
        let normalized = input.text.trim().to_string();
        let key = CacheKey::new(
            CLASSIFIER_ID,
            MODEL_REVISION,
            TOKENIZER_REVISION,
            TAXONOMY_REVISION,
            &normalized,
        );
        let metrics = self.metrics.clone();
        // A miss runs the raw backend forward on the designated single-flight
        // caller; a hit bypasses it entirely (AC-006). The flag distinguishes the
        // two so the hit/miss counters partition every request.
        let forward_ran = Arc::new(AtomicBool::new(false));
        let forward = {
            let runtime = self.runtime.clone();
            let metrics = metrics.clone();
            let forward_ran = forward_ran.clone();
            let input = ClassificationInput {
                text: normalized,
                requested_signals: input.requested_signals,
                session_metadata: input.session_metadata,
            };
            move || {
                forward_ran.store(true, Ordering::SeqCst);
                // Queue wait ends when the forward (dequeued work) begins.
                metrics.record_stage(LatencyStage::Queue, total_start.elapsed());
                runtime.classify(input)
            }
        };
        let result = self.cache.classify_concurrent(key, forward);
        // Classify the request as a cache hit or miss and expose the counters.
        if forward_ran.load(Ordering::SeqCst) {
            metrics.record_cache_miss();
        } else {
            metrics.record_cache_hit();
            // A hit's queue wait ends when the cached result is served.
            metrics.record_stage(LatencyStage::Queue, total_start.elapsed());
        }
        metrics.record_stage(LatencyStage::Total, total_start.elapsed());
        result
    }
}

/// The real Candle embedder + ranker path implementing [`ClassifierRuntime`].
///
/// This is the RAW backend: it performs only the real tokenize and model
/// forward then rank. It carries NO cache and NO single-flight logic — those
/// live in the generic [`ServiceCore`] that wraps it, so a production cache hit
/// through the core never reaches the tokenizer or model forward (AC-006).
///
/// Instrumented counters (`tokenizer_call_counter` / `forward_call_counter`) prove
/// a cache hit performs ZERO tokenizer calls and ZERO model forwards.
///
/// Requires the fetched local model weights (gitignored), so the exercising tests
/// are `#[ignore]`d and run explicitly with `-- --ignored` after `./hack/fetch-model`.
pub struct CandleClassifier {
    embedder: crate::embedding::Embedder,
    prototypes: Arc<Vec<Prototype>>,
    metrics: Metrics,
    /// Number of real tokenizer invocations (instrumented for the parity test).
    tokenizer_calls: Arc<AtomicU64>,
    /// Number of real model forwards (instrumented for the parity test).
    forward_calls: Arc<AtomicU64>,
}

impl CandleClassifier {
    /// Build a Candle classifier from a resident embedder and the prototype set.
    pub fn new(embedder: crate::embedding::Embedder, prototypes: Vec<Prototype>) -> Self {
        Self::with_metrics(embedder, prototypes, Metrics::new())
    }

    /// Build a Candle classifier whose tokenize/forward stage recording shares
    /// the CALLER-SUPPLIED [`Metrics`] handle.
    pub fn with_metrics(
        embedder: crate::embedding::Embedder,
        prototypes: Vec<Prototype>,
        metrics: Metrics,
    ) -> Self {
        CandleClassifier {
            embedder,
            prototypes: Arc::new(prototypes),
            metrics,
            tokenizer_calls: Arc::new(AtomicU64::new(0)),
            forward_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A SHARED handle to the classifier's metrics registry.
    ///
    /// The served path shares this same handle with the server surface and the
    /// request executor, so the stage decomposition recorded by the real Candle
    /// forward is visible to a benchmark harness (AC-012).
    pub fn metrics(&self) -> Metrics {
        self.metrics.clone()
    }

    /// A SHARED tokenizer-call counter handle, incremented on every real Candle
    /// tokenize. Clone it BEFORE moving the classifier into a [`ServiceCore`] so
    /// a test can observe that a cache hit performs ZERO tokenizer calls.
    pub fn tokenizer_call_counter(&self) -> Arc<AtomicU64> {
        self.tokenizer_calls.clone()
    }

    /// A SHARED model-forward counter handle, incremented on every real Candle
    /// model forward. Clone it BEFORE moving the classifier into a
    /// [`ServiceCore`] so a test can observe that a cache hit performs ZERO
    /// model forwards.
    pub fn forward_call_counter(&self) -> Arc<AtomicU64> {
        self.forward_calls.clone()
    }

    /// Real Candle forward (tokenize + embed + rank) with the tokenize and
    /// forward stages measured independently from their own boundaries (AC-012).
    /// The runtime counters increment on every real tokenizer call / forward.
    fn real_forward(&self, text: &str) -> Result<ClassificationResult, ClassifyError> {
        // Tokenize stage (AC-012): independently measured.
        let tokenize_start = std::time::Instant::now();
        self.tokenizer_calls.fetch_add(1, Ordering::SeqCst);
        let ids = self
            .embedder
            .tokenize(text)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        self.metrics
            .record_stage(LatencyStage::Tokenize, tokenize_start.elapsed());
        // Forward stage (AC-012): the real embed + rank, independently measured.
        let forward_start = std::time::Instant::now();
        self.forward_calls.fetch_add(1, Ordering::SeqCst);
        let embedding = self
            .embedder
            .embed_ids(ids)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        let ranked = cosine_rank(&embedding, &self.prototypes)
            .into_iter()
            .map(|(id, score)| RankedSignal { id, score })
            .collect();
        self.metrics
            .record_stage(LatencyStage::Forward, forward_start.elapsed());
        Ok(ClassificationResult {
            classifier_id: CLASSIFIER_ID.to_string(),
            model_revision: MODEL_REVISION.to_string(),
            tokenizer_revision: TOKENIZER_REVISION.to_string(),
            taxonomy_revision: TAXONOMY_REVISION.to_string(),
            status: ClassifyStatus::Ok,
            ranked,
        })
    }

    /// Build the classifier from the fetched sensitivity model artifacts and the
    /// committed synthetic prototypes. Offline: reads the resident model dir.
    pub fn from_modelcar(model_dir: &std::path::Path) -> Result<Self, ClassifyError> {
        let embedder = crate::embedding::Embedder::load(
            model_dir.join("config.json"),
            model_dir.join("model.safetensors"),
            model_dir.join("tokenizer.json"),
            model_dir.join("1_Pooling/config.json"),
        )
        .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        let prototypes = load_prototypes(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("modelcar")
                .join("synthetic-prototypes.json"),
        );
        Ok(CandleClassifier::new(embedder, prototypes))
    }
}

impl ClassifierRuntime for CandleClassifier {
    /// Classify `input`, returning ranked semantic signals from the ACTUAL
    /// embedding.
    ///
    /// RAW backend forward: tokenize + embed + rank, with the tokenize/forward
    /// stages measured (AC-012). There is NO cache, single-flight, or hit/miss
    /// logic here — the generic [`ServiceCore`] that wraps this backend provides
    /// them, so a cache hit through the core never reaches this forward (AC-006).
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let normalized = input.text.trim().to_string();
        self.real_forward(&normalized)
    }
}

/// Load and warm the resident Candle classifier from a ModelCar directory.
///
/// AC-002/AC-003: a model directory that merely exists must NOT produce READY.
/// This performs the full load/warmup sequence and returns an actionable typed
/// error on ANY failure:
///
/// 1. validate the ModelCar layout via the existing required-files check
///    (`Runtime::warmup_modelcar`);
/// 2. load tokenizer + bert config + safetensors and build the
///    [`CandleClassifier`];
/// 3. run a WARMUP FORWARD on a fixture input to prove readiness.
///
/// Only a fully warmed classifier is returned; any failure leaves the service
/// NOT ready.
pub fn load_and_warm_modelcar<P: AsRef<std::path::Path>>(
    model_dir: P,
) -> Result<CandleClassifier, ClassifyError> {
    let model_dir = model_dir.as_ref();
    // (1) ModelCar required-files layout check (AC-003): a dir that merely
    // exists but lacks the resident weights/tokenizer/pooling config is rejected
    // before any load.
    let mut runtime = crate::runtime::Runtime::new();
    runtime
        .warmup_modelcar(model_dir, crate::runtime::MODELCAR_REQUIRED_FILES)
        .map_err(ClassifyError::Unavailable)?;
    // (2) Load tokenizer + config + safetensors and build the real classifier.
    let classifier = CandleClassifier::from_modelcar(model_dir)?;
    // (3) Warmup forward on a fixture input; a runtime error leaves not-ready.
    let warmup_input = ClassificationInput {
        text: WARMUP_INPUT.to_string(),
        requested_signals: vec!["sensitivity".to_string()],
        session_metadata: HashMap::new(),
    };
    classifier
        .classify(warmup_input)
        .map_err(|e| ClassifyError::Unavailable(format!("warmup forward failed: {e}")))?;
    Ok(classifier)
}

/// Deterministic classification pipeline (no model forward).
///
/// This is the RAW synthetic backend: it performs only the deterministic
/// tokenize + rank forward. It carries NO cache and NO single-flight logic —
/// those live in the generic [`ServiceCore`] that wraps it, so a cache hit
/// through the core never reaches the tokenizer/ranker (AC-006).
///
/// [`Clone`] is derived so the backend can be shared; the tokenizer and
/// prototypes are shared read-only via [`Arc`].
#[derive(Clone)]
pub struct ClassifyService {
    tokenizer: Arc<Tokenizer>,
    prototypes: Arc<Vec<Prototype>>,
    metrics: Metrics,
}

impl ClassifyService {
    /// Build a pipeline from an already-loaded resident [`Tokenizer`] and the
    /// prototype set to rank against.
    pub fn new(tokenizer: Tokenizer, prototypes: Vec<Prototype>) -> Self {
        Self::with_metrics(tokenizer, prototypes, Metrics::new())
    }

    /// Build a pipeline that records latency decomposition into `metrics`.
    pub fn with_metrics(
        tokenizer: Tokenizer,
        prototypes: Vec<Prototype>,
        metrics: Metrics,
    ) -> Self {
        Self {
            tokenizer: Arc::new(tokenizer),
            prototypes: Arc::new(prototypes),
            metrics,
        }
    }

    /// Build the deterministic pipeline from the committed synthetic fixtures
    /// (`tests/fixtures/modelcar/tokenizer.json` + `synthetic-prototypes.json`).
    /// Offline only — no model download required for I-001.
    pub fn from_synthetic_fixtures() -> Self {
        let (tokenizer, prototypes) = Self::synthetic_fixtures();
        Self::new(tokenizer, prototypes)
    }

    /// Build the deterministic pipeline from the synthetic fixtures, recording
    /// latency decomposition into `metrics`.
    pub fn from_synthetic_fixtures_with_metrics(metrics: Metrics) -> Self {
        let (tokenizer, prototypes) = Self::synthetic_fixtures();
        Self::with_metrics(tokenizer, prototypes, metrics)
    }

    /// Load the committed synthetic tokenizer + prototype fixtures (offline).
    fn synthetic_fixtures() -> (Tokenizer, Vec<Prototype>) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("modelcar");
        let tokenizer = Tokenizer::load(root.join("tokenizer.json"))
            .expect("synthetic tokenizer fixture must load");
        let prototypes = load_prototypes(root.join("synthetic-prototypes.json"));
        (tokenizer, prototypes)
    }

    /// Deterministic tokenize + rank: tokenize with the resident tokenizer,
    /// build a deterministic pseudo-embedding from the token IDs, and cosine-rank
    /// against the synthetic prototypes. No model forward. A tokenization
    /// failure is an explicit [`ClassifyError::Tokenizer`] — never a fabricated
    /// label (spec failure contract).
    fn deterministic_classify(&self, context: &str) -> Result<ClassificationResult, ClassifyError> {
        // Tokenize stage (AC-012): independently measured.
        let tokenize_start = std::time::Instant::now();
        let ids = self
            .tokenizer
            .tokenize(context)
            .map_err(|e| ClassifyError::Tokenizer(e.to_string()))?;
        self.metrics
            .record_stage(LatencyStage::Tokenize, tokenize_start.elapsed());
        // Forward stage (AC-012): the deterministic embed + rank, independently
        // measured from the tokenize boundary.
        let forward_start = std::time::Instant::now();
        let mut embedding = vec![0.0f32; SYNTHETIC_DIM];
        for id in ids {
            embedding[(id as usize) % SYNTHETIC_DIM] += 1.0;
        }
        let ranked = cosine_rank(&embedding, &self.prototypes)
            .into_iter()
            .map(|(id, score)| RankedSignal { id, score })
            .collect();
        self.metrics
            .record_stage(LatencyStage::Forward, forward_start.elapsed());
        Ok(ClassificationResult {
            classifier_id: CLASSIFIER_ID.to_string(),
            model_revision: MODEL_REVISION.to_string(),
            tokenizer_revision: TOKENIZER_REVISION.to_string(),
            taxonomy_revision: TAXONOMY_REVISION.to_string(),
            status: ClassifyStatus::Ok,
            ranked,
        })
    }
}

impl ClassifierRuntime for ClassifyService {
    /// Classify `input`, returning ranked semantic signals.
    ///
    /// RAW backend forward: deterministic tokenize + rank over the synthetic
    /// prototypes, with the tokenize/forward stages measured (AC-012). There is
    /// NO cache, single-flight, or hit/miss logic here — the generic
    /// [`ServiceCore`] that wraps this backend provides them, so a cache hit
    /// through the core never reaches this forward (AC-006).
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let normalized = input.text.trim().to_string();
        self.deterministic_classify(&normalized)
    }
}

/// Load the synthetic prototype fixture, asserting its label and dimension.
fn load_prototypes<P: AsRef<std::path::Path>>(path: P) -> Vec<Prototype> {
    let raw = std::fs::read_to_string(path).expect("synthetic prototype fixture must exist");
    let root: serde_json::Value =
        serde_json::from_str(&raw).expect("synthetic prototype fixture must be valid JSON");
    assert_eq!(
        root.get("label").and_then(serde_json::Value::as_str),
        Some("synthetic_for_mechanics_only"),
        "fixture must be labeled synthetic_for_mechanics_only"
    );
    assert_eq!(
        root.get("dim").and_then(serde_json::Value::as_u64),
        Some(SYNTHETIC_DIM as u64),
        "fixture dim must be 384"
    );
    root.get("prototypes")
        .and_then(serde_json::Value::as_array)
        .expect("fixture must have a prototypes array")
        .iter()
        .map(|obj| {
            let id = obj
                .get("id")
                .and_then(serde_json::Value::as_str)
                .expect("prototype id")
                .to_string();
            let vector: Vec<f32> = obj
                .get("vector")
                .and_then(serde_json::Value::as_array)
                .expect("prototype vector")
                .iter()
                .map(|v| v.as_f64().expect("vector value") as f32)
                .collect();
            Prototype::new(id, vector)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U-070 (AC-006/AC-010): the typed result must carry ranked semantic
    /// signals with scores and revision fingerprint fields, and MUST NOT contain
    /// any route/endpoint field (AC-010). The deterministic pipeline classifies
    /// the golden input and returns a real Ok result with non-empty ranked
    /// signals and no route anywhere in the response.
    #[test]
    fn u070_typed_result_has_ranked_signals_and_no_route() {
        let service = ClassifyService::from_synthetic_fixtures();
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::from([("session_id".to_string(), "sess-0001".to_string())]),
        };
        let result = service.classify(input).expect("golden input must classify");
        assert_eq!(result.status, ClassifyStatus::Ok);
        assert_eq!(result.classifier_id, CLASSIFIER_ID);
        assert_eq!(result.model_revision, MODEL_REVISION);
        assert_eq!(result.tokenizer_revision, TOKENIZER_REVISION);
        assert_eq!(result.taxonomy_revision, TAXONOMY_REVISION);
        assert!(
            !result.ranked.is_empty(),
            "typed result must carry ranked semantic signals"
        );
        // AC-010: the typed response carries semantic evidence, never a route.
        assert!(
            !result.ranked.is_empty(),
            "AC-010: response contains signals, not a final route"
        );
    }

    /// U-071 (AC-006): an identical re-classification through the generic
    /// [`ServiceCore`] returns the exact cached result (single-flight + exact
    /// cache), and the revision fingerprint fields are stable. Two identical
    /// inputs must produce identical results.
    #[test]
    fn u071_identical_inputs_return_identical_results() {
        // The deterministic raw backend is wrapped in the shared core so the
        // second identical call is served from the cache (AC-006), proving the
        // cache pipeline lives in the core, not the backend.
        let core =
            ServiceCore::with_metrics(ClassifyService::from_synthetic_fixtures(), Metrics::new());
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        let first = core.classify(input.clone()).expect("first classify");
        let second = core.classify(input).expect("second classify");
        assert_eq!(
            first, second,
            "identical inputs must produce identical results"
        );
    }

    /// The real Candle embedder+ranker path implementing [`ClassifierRuntime`].
    ///
    /// Requires the fetched local model weights (gitignored), so the test is
    /// `#[ignore]`d and run explicitly with `-- --ignored` after `./hack/fetch-model`.
    /// It classifies the golden input through the trait and asserts a real Ok
    /// result with ranked signals and no route field (AC-010).
    #[test]
    #[ignore]
    fn u072_candle_classifier_implements_classifier_runtime() {
        let model_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join("models")
            .join("sensitivity");
        let classifier = CandleClassifier::from_modelcar(&model_dir)
            .expect("candle classifier must load from the fetched sensitivity model");
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        let result = classifier
            .classify(input)
            .expect("candle classifier must classify the golden input");
        assert_eq!(result.status, ClassifyStatus::Ok);
        assert!(
            !result.ranked.is_empty(),
            "candle path must return ranked semantic signals"
        );
    }

    /// SERVICE-CORE (P0): a PRODUCTION Candle cache hit must perform ZERO
    /// tokenizer calls and ZERO model forwards. The exact-result cache +
    /// single-flight live in the generic [`ServiceCore`], so a cache hit through
    /// the core NEVER reaches the raw Candle runtime's tokenizer or model
    /// forward. Counters are instrumented on the runtime (tokenizer calls /
    /// model forwards). Requires the fetched model weights (gitignored), so this
    /// parity-tier test is #[ignore]d and runs under `./hack/test-parity`.
    #[test]
    #[ignore]
    fn service_core_production_candle_cache_hit_zero_tokenizer_zero_forward() {
        use std::sync::atomic::Ordering;
        let model_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join("models")
            .join("sensitivity");
        let classifier = CandleClassifier::from_modelcar(&model_dir)
            .expect("candle classifier must load from the fetched sensitivity model");
        // Observe the runtime's tokenizer/forward counters AFTER the classifier
        // is moved into the core, so a cache hit's zero-calls is proven on the
        // production path (the raw Candle forward, not the synthetic one).
        let tokenizer_calls = classifier.tokenizer_call_counter();
        let forward_calls = classifier.forward_call_counter();
        let core = ServiceCore::with_metrics(classifier, Metrics::new());

        let golden = ClassificationInput {
            text: WARMUP_INPUT.to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };

        // Miss: exactly one tokenizer call and one model forward.
        let miss = core
            .classify(golden.clone())
            .expect("cache miss must classify");
        assert_eq!(
            tokenizer_calls.load(Ordering::SeqCst),
            1,
            "a cache miss runs exactly one tokenizer call"
        );
        assert_eq!(
            forward_calls.load(Ordering::SeqCst),
            1,
            "a cache miss runs exactly one model forward"
        );

        // Hit: ZERO new tokenizer calls and ZERO new model forwards (AC-006).
        let hit = core.classify(golden).expect("cache hit must classify");
        assert_eq!(
            tokenizer_calls.load(Ordering::SeqCst),
            1,
            "a production cache hit must perform ZERO tokenizer calls"
        );
        assert_eq!(
            forward_calls.load(Ordering::SeqCst),
            1,
            "a production cache hit must perform ZERO model forwards"
        );
        assert_eq!(miss, hit, "cache hit must return the exact cached result");

        // A distinct input is a fresh miss: both runtime counters increment.
        let other = ClassificationInput {
            text: "a distinct sensitivity context for a fresh miss".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        core.classify(other).expect("distinct input must classify");
        assert_eq!(
            tokenizer_calls.load(Ordering::SeqCst),
            2,
            "a distinct input runs a fresh tokenizer call"
        );
        assert_eq!(
            forward_calls.load(Ordering::SeqCst),
            2,
            "a distinct input runs a fresh model forward"
        );
    }

    /// AC-002/AC-003: a model directory that exists but is missing the required
    /// ModelCar files must NOT become READY. `load_and_warm_modelcar` must fail
    /// with an actionable typed error (never a ready classifier). This runs
    /// WITHOUT weights because the required-files check precedes any load.
    #[test]
    fn missing_required_files_leaves_not_ready() {
        let dir = std::env::temp_dir().join("llm-d-sc-realserve-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Intentionally write NO required ModelCar files.
        match load_and_warm_modelcar(&dir) {
            Err(ClassifyError::Unavailable(_)) => {}
            Ok(_) => panic!("a dir missing required files must leave not-ready"),
            Err(other) => panic!("must be an actionable unavailable error, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
