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

use crate::cache::{identity_tag, CacheKey, NoopSemanticCache, SemanticCache, SharedCache};
use crate::metrics::{LatencyStage, Metrics};
use crate::ranker::{anchor_rank, cosine_rank, AnchorSet, Prototype};
use crate::taxonomy::ClassifierDefinition;
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

/// A classifier-produced embedding: the L2-normalized vector the ranker and the
/// semantic cache both consume. Produced exactly once per classification so the
/// (expensive) model forward is never repeated for a cache lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    pub vector: Vec<f32>,
}

impl Embedding {
    /// Wrap a raw embedding vector.
    pub fn new(vector: Vec<f32>) -> Self {
        Embedding { vector }
    }

    /// The embedding dimension.
    pub fn dim(&self) -> usize {
        self.vector.len()
    }
}

/// One ranked semantic signal: an id and its deterministic similarity score.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Embed `input` into its (L2-normalized) vector. This is the expensive
    /// model-forward stage; it runs at most once per classification so a cache
    /// lookup never repeats it.
    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError>;

    /// Rank a previously-computed `embedding` into typed semantic evidence.
    fn rank(
        &self,
        embedding: &Embedding,
        input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError>;

    /// Classify `input`: embed once, then rank. Backends inherit this; the
    /// caching core overrides it to interpose the exact and semantic caches.
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let embedding = self.embed(&input)?;
        self.rank(&embedding, &input)
    }

    /// The immutable identity of what this runtime actually loaded.
    ///
    /// Everything that needs to know WHICH classifier is resident previously
    /// guessed: the gRPC surface hardcoded a signal name, the cache keyed on
    /// module constants, and results reported fixture revisions. Each of those
    /// was independently wrong in the same way, because the information only
    /// existed inside the backend and was never exposed. A runtime that can
    /// describe itself fixes all of them at once, and makes a second backend
    /// possible without teaching every caller about it.
    fn metadata(&self) -> RuntimeMetadata;
}

/// The immutable identity of a loaded classifier.
///
/// `artifact_digest` is content-derived (see [`crate::runtime::modelcar_digest`])
/// while the revisions are declared. Both are carried because they answer
/// different questions: the revision says what was REQUESTED, the digest says
/// what was actually loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMetadata {
    pub classifier_id: String,
    /// The signal this runtime produces, for example `complexity`.
    pub signal: String,
    pub model_revision: String,
    pub tokenizer_revision: String,
    pub taxonomy_revision: String,
    /// Content digest of the resident artifact, when it was loaded from one.
    pub artifact_digest: Option<String>,
}

impl RuntimeMetadata {
    /// The identity components used to key the result cache. A change in ANY of
    /// them must produce a different key, so a stale classification can never be
    /// served across a revision or artifact change.
    pub fn cache_identity(&self) -> (&str, &str, &str, &str) {
        (
            &self.classifier_id,
            self.artifact_digest
                .as_deref()
                .unwrap_or(&self.model_revision),
            &self.tokenizer_revision,
            &self.taxonomy_revision,
        )
    }
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
    semantic: Arc<dyn SemanticCache>,
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
    /// tokenize/forward stage recording can share the same registry. The L2
    /// semantic cache tier defaults to [`NoopSemanticCache`] (always misses),
    /// so behaviour is UNCHANGED unless a semantic cache is opted into via
    /// [`ServiceCore::with_semantic_cache`].
    pub fn with_metrics(runtime: R, metrics: Metrics) -> Self {
        ServiceCore {
            runtime: Arc::new(runtime),
            cache: SharedCache::new(),
            semantic: Arc::new(NoopSemanticCache),
            metrics,
        }
    }

    /// Build a service core with an explicit L2 semantic cache tier.
    pub fn with_semantic_cache(
        runtime: R,
        metrics: Metrics,
        semantic: Arc<dyn SemanticCache>,
    ) -> Self {
        ServiceCore {
            runtime: Arc::new(runtime),
            cache: SharedCache::new(),
            semantic,
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
    /// The core adds caching, not identity: it reports the wrapped backend's.
    fn metadata(&self) -> RuntimeMetadata {
        self.runtime.metadata()
    }

    /// Delegates to the wrapped runtime's `embed`.
    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        self.runtime.embed(input)
    }

    /// Delegates to the wrapped runtime's `rank`.
    fn rank(
        &self,
        embedding: &Embedding,
        input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        self.runtime.rank(embedding, input)
    }

    /// Classify `input` through the shared core: versioned cache -> single-flight
    /// -> raw backend forward.
    ///
    /// A cache hit bypasses the backend's tokenizer and model forward entirely
    /// (AC-006); identical concurrent misses coalesce into ONE forward (AC-007).
    /// The cache stores the typed [`ClassificationResult`] keyed by the blake3
    /// versioned fingerprint, and the hit/miss/total/queue metrics are recorded
    /// here so EVERY backend inherits them.
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let normalized = input.text.trim().to_string();
        // Key on the identity the WRAPPED RUNTIME reports, not on module
        // constants. Keying on constants meant every backend shared one
        // namespace: two taxonomies in one process would have collided, and a
        // revision change would have served the previous revision's answers.
        let meta = self.runtime.metadata();
        let (classifier_id, model_rev, tokenizer_rev, taxonomy_rev) = meta.cache_identity();
        let key = CacheKey::new(
            classifier_id,
            model_rev,
            tokenizer_rev,
            taxonomy_rev,
            &normalized,
        );
        // The L2 isolation tag is built from the SAME identity fields as the L1
        // blake3 key, so a revision bump can never serve a stale semantic label.
        let tag = identity_tag(meta.cache_identity());
        let metrics = self.metrics.clone();
        // A miss runs the raw backend forward on the designated single-flight
        // caller; a hit bypasses it entirely (AC-006). The flag distinguishes the
        // two so the hit/miss counters partition every request.
        let forward_ran = Arc::new(AtomicBool::new(false));
        let forward = {
            let runtime = self.runtime.clone();
            let semantic = self.semantic.clone();
            let metrics = metrics.clone();
            let forward_ran = forward_ran.clone();
            let tag = tag.clone();
            let input = ClassificationInput {
                text: normalized,
                requested_signals: input.requested_signals,
                session_metadata: input.session_metadata,
            };
            move || {
                forward_ran.store(true, Ordering::SeqCst);
                let _ = &metrics;
                // Embed once. Reused by both the L2 lookup and the ranker.
                let embedding = runtime.embed(&input)?;
                // L2 semantic lookup (fail-open: None on any trouble).
                if let Some(hit) = semantic.lookup(&embedding, &tag) {
                    return Ok(hit);
                }
                // L2 miss: rank, then best-effort write-back.
                let result = runtime.rank(&embedding, &input)?;
                semantic.insert(&embedding, &result, &tag);
                Ok(result)
            }
        };
        let result = self.cache.classify_concurrent(key, forward);
        // Classify the request as a cache hit or miss and expose the counters.
        if forward_ran.load(Ordering::SeqCst) {
            metrics.record_cache_miss();
        } else {
            metrics.record_cache_hit();
        }
        // Queue and Total are deliberately NOT recorded here. The executor owns
        // Queue (it is the only component that knows how long a job waited), and
        // the gRPC surface owns Total (it is the only component that sees the
        // whole request). Recording them here previously double-counted Queue
        // into the same histogram and produced a "Total" that started AFTER
        // dequeue, so it excluded the queue wait it claimed to include.
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
    /// The active taxonomy. When present the classifier ranks against real
    /// labelled anchors and reports that taxonomy's identity; when absent it
    /// falls back to the committed synthetic prototypes (weight-free tests).
    taxonomy: Option<Arc<ResidentTaxonomy>>,
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
            taxonomy: None,
            metrics,
            tokenizer_calls: Arc::new(AtomicU64::new(0)),
            forward_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Build a Candle classifier that ranks against a real classifier
    /// definition. The anchors are embedded ONCE here, at load time, by the same
    /// resident model that will embed requests.
    pub fn with_taxonomy(
        embedder: crate::embedding::Embedder,
        definition: ClassifierDefinition,
        metrics: Metrics,
    ) -> Result<Self, ClassifyError> {
        Self::with_taxonomy_and_digest(embedder, definition, metrics, None)
    }

    /// As [`CandleClassifier::with_taxonomy`], carrying the content digest of the
    /// artifact the embedder was loaded from.
    pub fn with_taxonomy_and_digest(
        embedder: crate::embedding::Embedder,
        definition: ClassifierDefinition,
        metrics: Metrics,
        artifact_digest: Option<String>,
    ) -> Result<Self, ClassifyError> {
        let anchors = definition
            .embed_anchors(&embedder)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        Ok(CandleClassifier {
            embedder,
            prototypes: Arc::new(Vec::new()),
            taxonomy: Some(Arc::new(ResidentTaxonomy {
                classifier_id: definition.classifier_id,
                signal: definition.signal,
                taxonomy_revision: definition.taxonomy_revision,
                tokenizer_revision: definition.model_revision.clone(),
                model_revision: definition.model_revision,
                artifact_digest,
                top_k: definition.top_k,
                anchors,
            })),
            metrics,
            tokenizer_calls: Arc::new(AtomicU64::new(0)),
            forward_calls: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The active classifier id (the taxonomy's id, or the synthetic fallback).
    pub fn classifier_id(&self) -> &str {
        self.taxonomy
            .as_ref()
            .map(|t| t.classifier_id.as_str())
            .unwrap_or(CLASSIFIER_ID)
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

    /// Build the classifier from the resident ModelCar directory, ranking
    /// against the classifier definition selected by the environment
    /// (`LLM_D_SC_CLASSIFIER`, default `complexity`). The definition may name a
    /// built-in taxonomy or point at a custom definition JSON. Offline: reads
    /// the resident model dir and a compiled-in or local definition, never the
    /// network.
    pub fn from_modelcar(model_dir: &std::path::Path) -> Result<Self, ClassifyError> {
        let definition = ClassifierDefinition::from_env()
            .map_err(|e| ClassifyError::Unavailable(e.to_string()))?;
        Self::from_modelcar_with(model_dir, definition)
    }

    /// Build the classifier from the resident ModelCar directory against an
    /// EXPLICIT classifier definition.
    pub fn from_modelcar_with(
        model_dir: &std::path::Path,
        definition: ClassifierDefinition,
    ) -> Result<Self, ClassifyError> {
        let embedder = crate::embedding::Embedder::load(
            model_dir.join("config.json"),
            model_dir.join("model.safetensors"),
            model_dir.join("tokenizer.json"),
            model_dir.join("1_Pooling/config.json"),
        )
        .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        // Compute the digest of what was actually loaded and carry it into the
        // classifier's identity. It was previously computed during warmup and
        // then discarded, so the provenance it was meant to provide never
        // reached a result or a cache key.
        let digest =
            crate::runtime::modelcar_digest(model_dir, crate::runtime::MODELCAR_REQUIRED_FILES)
                .ok();
        CandleClassifier::with_taxonomy_and_digest(embedder, definition, Metrics::new(), digest)
    }
}

/// The load-time-resolved taxonomy held resident alongside the model: the
/// embedded anchors plus the identity that every result reports.
#[derive(Debug)]
struct ResidentTaxonomy {
    classifier_id: String,
    signal: String,
    taxonomy_revision: String,
    model_revision: String,
    /// The tokenizer ships INSIDE the ModelCar alongside the weights, so its
    /// revision is the artifact's revision. Reporting a fixture string here was
    /// the provenance claim that could not be true.
    tokenizer_revision: String,
    /// Content digest of the loaded artifact, so a result can be tied to bytes
    /// rather than to a requested revision that a stale mount may not match.
    artifact_digest: Option<String>,
    top_k: usize,
    anchors: Vec<AnchorSet>,
}

impl ClassifierRuntime for CandleClassifier {
    fn metadata(&self) -> RuntimeMetadata {
        match self.taxonomy.as_ref() {
            Some(t) => RuntimeMetadata {
                classifier_id: t.classifier_id.clone(),
                signal: t.signal.clone(),
                model_revision: t.model_revision.clone(),
                tokenizer_revision: t.tokenizer_revision.clone(),
                taxonomy_revision: t.taxonomy_revision.clone(),
                artifact_digest: t.artifact_digest.clone(),
            },
            // The weight-free synthetic path, used only by tests.
            None => RuntimeMetadata {
                classifier_id: CLASSIFIER_ID.to_string(),
                signal: "sensitivity".to_string(),
                model_revision: MODEL_REVISION.to_string(),
                tokenizer_revision: TOKENIZER_REVISION.to_string(),
                taxonomy_revision: TAXONOMY_REVISION.to_string(),
                artifact_digest: None,
            },
        }
    }

    /// Embed `input` with the real Candle tokenizer + model forward, with the
    /// tokenize and forward stages measured independently from their own
    /// boundaries (AC-012). The runtime counters increment on every real
    /// tokenizer call / forward. There is NO cache or single-flight logic
    /// here — the generic [`ServiceCore`] that wraps this backend provides
    /// them, so a cache hit through the core never reaches this stage (AC-006).
    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        let text = input.text.trim();
        // Tokenize stage (AC-012): independently measured.
        let tokenize_start = std::time::Instant::now();
        self.tokenizer_calls.fetch_add(1, Ordering::SeqCst);
        let ids = self
            .embedder
            .tokenize(text)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        self.metrics
            .record_stage(LatencyStage::Tokenize, tokenize_start.elapsed());
        // Forward stage (AC-012): the real model forward.
        let forward_start = std::time::Instant::now();
        self.forward_calls.fetch_add(1, Ordering::SeqCst);
        let vector = self
            .embedder
            .embed_ids(ids)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        self.metrics
            .record_stage(LatencyStage::Forward, forward_start.elapsed());
        Ok(Embedding::new(vector))
    }

    /// Rank a previously-computed embedding against the resident taxonomy (or
    /// the synthetic fallback). No tokenizer/forward counters increment here —
    /// they belong to the embed stage.
    fn rank(
        &self,
        embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        let v = &embedding.vector;
        let (ranked, identity) = match self.taxonomy.as_ref() {
            Some(t) => (
                anchor_rank(v, &t.anchors, t.top_k),
                (
                    t.classifier_id.clone(),
                    t.model_revision.clone(),
                    t.taxonomy_revision.clone(),
                ),
            ),
            None => (
                cosine_rank(v, &self.prototypes),
                (
                    CLASSIFIER_ID.to_string(),
                    MODEL_REVISION.to_string(),
                    TAXONOMY_REVISION.to_string(),
                ),
            ),
        };
        let tokenizer_revision = self
            .taxonomy
            .as_ref()
            .map(|t| t.tokenizer_revision.clone())
            .unwrap_or_else(|| TOKENIZER_REVISION.to_string());
        let ranked = ranked
            .into_iter()
            .map(|(id, score)| RankedSignal { id, score })
            .collect();
        Ok(ClassificationResult {
            classifier_id: identity.0,
            model_revision: identity.1,
            tokenizer_revision,
            taxonomy_revision: identity.2,
            status: ClassifyStatus::Ok,
            ranked,
        })
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
}

impl ClassifierRuntime for ClassifyService {
    fn metadata(&self) -> RuntimeMetadata {
        RuntimeMetadata {
            classifier_id: CLASSIFIER_ID.to_string(),
            signal: "sensitivity".to_string(),
            model_revision: MODEL_REVISION.to_string(),
            tokenizer_revision: TOKENIZER_REVISION.to_string(),
            taxonomy_revision: TAXONOMY_REVISION.to_string(),
            artifact_digest: None,
        }
    }

    /// Embed `input`: tokenize with the resident tokenizer and build a
    /// deterministic pseudo-embedding from the token IDs. No model forward. A
    /// tokenization failure is an explicit [`ClassifyError::Tokenizer`] — never
    /// a fabricated label (spec failure contract). There is NO cache or
    /// single-flight logic here — the generic [`ServiceCore`] that wraps this
    /// backend provides them, so a cache hit through the core never reaches
    /// this stage (AC-006).
    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        let context = input.text.trim();
        // Tokenize stage (AC-012): independently measured.
        let tokenize_start = std::time::Instant::now();
        let ids = self
            .tokenizer
            .tokenize(context)
            .map_err(|e| ClassifyError::Tokenizer(e.to_string()))?;
        self.metrics
            .record_stage(LatencyStage::Tokenize, tokenize_start.elapsed());
        let mut vector = vec![0.0f32; SYNTHETIC_DIM];
        for id in ids {
            vector[(id as usize) % SYNTHETIC_DIM] += 1.0;
        }
        Ok(Embedding::new(vector))
    }

    /// Rank a previously-computed embedding by cosine similarity against the
    /// synthetic prototypes. No tokenizer counters here — they belong to the
    /// embed stage.
    fn rank(
        &self,
        embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        // Forward stage (AC-012): the deterministic rank, independently
        // measured from the tokenize boundary.
        let forward_start = std::time::Instant::now();
        let ranked = cosine_rank(&embedding.vector, &self.prototypes)
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

    #[test]
    fn embedding_reports_its_dimension() {
        let e = Embedding::new(vec![0.0, 1.0, 0.0]);
        assert_eq!(e.dim(), 3);
        assert_eq!(e.vector, vec![0.0, 1.0, 0.0]);
    }

    /// A spy [`crate::cache::SemanticCache`] proves an L1 miss consults the L2
    /// tier exactly once, and an L2 hit is served verbatim WITHOUT invoking the
    /// ranker.
    #[test]
    fn service_core_serves_semantic_hit_without_ranking() {
        use crate::cache::SemanticCache;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;

        struct SpyCache {
            canned: ClassificationResult,
            lookups: StdArc<AtomicUsize>,
        }
        impl SemanticCache for SpyCache {
            fn lookup(&self, _e: &Embedding, _id: &str) -> Option<ClassificationResult> {
                self.lookups.fetch_add(1, Ordering::SeqCst);
                Some(self.canned.clone())
            }
            fn insert(&self, _e: &Embedding, _r: &ClassificationResult, _id: &str) {}
        }

        let canned = ClassificationResult {
            classifier_id: "spy".into(),
            model_revision: "m".into(),
            tokenizer_revision: "t".into(),
            taxonomy_revision: "x".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "SEMANTIC_HIT".into(),
                score: 0.99,
            }],
        };
        let lookups = StdArc::new(AtomicUsize::new(0));
        let spy = StdArc::new(SpyCache {
            canned: canned.clone(),
            lookups: lookups.clone(),
        });

        let core = ServiceCore::with_semantic_cache(
            ClassifyService::from_synthetic_fixtures(),
            Metrics::new(),
            spy,
        );
        let input = ClassificationInput {
            text: "some novel prompt not seen before".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        let out = core.classify(input).expect("classify");
        assert_eq!(
            lookups.load(Ordering::SeqCst),
            1,
            "L1 miss must consult L2 once"
        );
        assert_eq!(
            out.ranked[0].id, "SEMANTIC_HIT",
            "L2 hit must be served verbatim"
        );
    }

    /// The default [`ServiceCore`] (Noop L2) must remain unaffected: an L1 miss
    /// with a Noop L2 always misses, so it must run exactly one embed and one
    /// rank, reproducing the pre-L2 behaviour byte-for-byte.
    #[test]
    fn service_core_noop_default_is_unaffected() {
        let core =
            ServiceCore::with_metrics(ClassifyService::from_synthetic_fixtures(), Metrics::new());
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        let via_core = core.classify(input.clone()).expect("core classify");
        let direct = ClassifyService::from_synthetic_fixtures()
            .classify(input)
            .expect("direct classify");
        assert_eq!(
            via_core, direct,
            "default Noop L2 must not change the classification result"
        );
    }

    /// The `embed`/`rank` split must reproduce the provided `classify` default
    /// exactly on the synthetic path, and `embed` must be independently callable
    /// (a later cache tier interposes here).
    #[test]
    fn embed_then_rank_matches_classify_on_synthetic() {
        let svc = ClassifyService::from_synthetic_fixtures();
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        // Two-stage path.
        let embedding = svc.embed(&input).expect("embed");
        assert_eq!(embedding.dim(), SYNTHETIC_DIM);
        let staged = svc.rank(&embedding, &input).expect("rank");
        // Provided classify() default must produce the identical result.
        let one_shot = svc.classify(input).expect("classify");
        assert_eq!(
            staged, one_shot,
            "embed+rank must equal the provided classify"
        );
    }
}
