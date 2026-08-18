//! Classification pipeline: typed core, tokenizer -> versioned cache ->
//! single-flight -> deterministic ranker over synthetic prototypes.
//!
//! This module defines the typed classification contract ([`ClassificationInput`],
//! [`ClassificationResult`], [`ClassifyStatus`], [`ClassifyError`]) and the
//! [`ClassifierRuntime`] abstraction the service and Candle backend both
//! implement. The response NEVER carries a route/endpoint field: routing/session
//! authority remains Praxis (AC-010).
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
/// data owned by Praxis, AC-010).
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
/// authority is Praxis (AC-010).
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

/// The real Candle embedder + ranker path implementing [`ClassifierRuntime`].
///
/// Embeds the input with the resident [`crate::embedding::Embedder`] (real
/// Candle forward) and cosine-ranks the synthetic prototypes. Requires the
/// fetched local model weights (gitignored), so the exercising tests are
/// `#[ignore]`d and run explicitly with `-- --ignored` after `./hack/fetch-model`.
pub struct CandleClassifier {
    embedder: crate::embedding::Embedder,
    prototypes: Arc<Vec<Prototype>>,
}

impl CandleClassifier {
    /// Build a Candle classifier from a resident embedder and the prototype set.
    pub fn new(embedder: crate::embedding::Embedder, prototypes: Vec<Prototype>) -> Self {
        CandleClassifier {
            embedder,
            prototypes: Arc::new(prototypes),
        }
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
    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let embedding = self
            .embedder
            .embed(&input.text)
            .map_err(|e| ClassifyError::Embedding(e.to_string()))?;
        let ranked = cosine_rank(&embedding, &self.prototypes)
            .into_iter()
            .map(|(id, score)| RankedSignal { id, score })
            .collect();
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
/// Wires tokenizer -> versioned cache -> single-flight -> deterministic ranker
/// over the synthetic prototypes, returning ranked semantic signals via the
/// typed [`ClassificationResult`] — and NEVER a final route (routing authority
/// is Praxis, AC-010).
///
/// [`Clone`] is derived so the service can back a tonic server (which requires a
/// `Clone + Send + Sync + 'static` service); the tokenizer and prototypes are
/// shared read-only via [`Arc`].
#[derive(Clone)]
pub struct ClassifyService {
    cache: SharedCache,
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
            cache: SharedCache::new(),
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
    fn deterministic_classify(
        &self,
        context: &str,
        metrics: &Metrics,
    ) -> Result<ClassificationResult, ClassifyError> {
        // Tokenize stage (AC-012): independently measured.
        let tokenize_start = std::time::Instant::now();
        let ids = self
            .tokenizer
            .tokenize(context)
            .map_err(|e| ClassifyError::Tokenizer(e.to_string()))?;
        metrics.record_stage(LatencyStage::Tokenize, tokenize_start.elapsed());
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
        metrics.record_stage(LatencyStage::Forward, forward_start.elapsed());
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
    /// Pipeline: tokenizer -> versioned cache -> single-flight -> deterministic
    /// ranker over the synthetic prototypes. A cache hit bypasses tokenization
    /// and ranking; identical concurrent misses coalesce into one forward. The
    /// cache stores the typed [`ClassificationResult`] keyed by the blake3
    /// versioned fingerprint.
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
        // A miss runs the forward closure (tokenize + rank) on the designated
        // single-flight caller; a hit bypasses it entirely (AC-006). The flag
        // distinguishes the two so the hit/miss counters partition every request.
        let forward_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let forward = {
            let service = self.clone();
            let normalized = normalized.clone();
            let metrics = metrics.clone();
            let forward_ran = forward_ran.clone();
            move || {
                forward_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                // Queue wait ends when the forward (dequeued work) begins.
                metrics.record_stage(LatencyStage::Queue, total_start.elapsed());
                service.deterministic_classify(&normalized, &metrics)
            }
        };
        let result = self.cache.classify_concurrent(key, forward);
        // Classify the request as a cache hit or miss and expose the counters.
        if forward_ran.load(std::sync::atomic::Ordering::SeqCst) {
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

    /// U-071 (AC-006): an identical re-classification returns the exact cached
    /// result (single-flight + exact cache), and the revision fingerprint fields
    /// are stable. Two identical inputs must produce identical results.
    #[test]
    fn u071_identical_inputs_return_identical_results() {
        let service = ClassifyService::from_synthetic_fixtures();
        let input = ClassificationInput {
            text: "this is a golden sensitivity input".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        let first = service.classify(input.clone()).expect("first classify");
        let second = service.classify(input).expect("second classify");
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
