# Semantic Cache Classifier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional, off-by-default Redis-backed semantic cache tier that lets semantically-similar prompts reuse a stored classification label, layered above the existing in-memory exact (1:1) cache.

**Architecture:** Split the `ClassifierRuntime` forward into `embed` + `rank` (embed once), introduce a `SemanticCache` strategy trait (`NoopSemanticCache` default, `RedisSemanticCache` opt-in), and have `ServiceCore` orchestrate `L1 exact → embed → L2 semantic KNN → rank → write-back`. Redis is best-effort and fail-open: any Redis error degrades to compute and never fails a request.

**Tech Stack:** Rust (edition 2021, rustc ≥ 1.75), Candle BERT embeddings (already present), `redis` crate (sync API + `r2d2` pool) against Redis Stack / Redis 8+ (RediSearch vector index), blake3 (existing L1 key).

**Spec:** `docs/superpowers/specs/2026-08-30-semantic-cache-classifier-design.md`

## Global Constraints

- Edition 2021, `rust-version = 1.75`; do not raise the MSRV.
- No network access on the default path. The semantic tier is **off by default** (`LLM_D_SC_CACHE=exact`); when off, behavior must be byte-for-byte identical to today.
- **Fail-open, always:** every Redis error/timeout returns "no hit" (lookup) or is dropped (insert); classification never returns an error because of Redis.
- Preserve the existing L1 exact cache, single-flight coalescing, FIFO eviction, and the blake3 versioned `CacheKey` unchanged.
- Preserve `CandleClassifier`'s `tokenizer_calls` / `forward_calls` counter semantics: a cache MISS runs exactly one tokenize and one model forward; a cache HIT runs zero of each. (Enforced by `service_core_production_candle_cache_hit_zero_tokenizer_zero_forward` in `src/classify.rs`.)
- The classifier response never carries a route/endpoint field (AC-010). Do not add one.
- Follow existing patterns: typed errors (no `unwrap` on I/O), `//!` module docs, `#[ignore]` for tests requiring external resources (model weights / live Redis), `KNOWN_*` registry + validated config.
- Use `cargo add` for new dependencies (do not hand-pin versions): `cargo add redis --features r2d2`, `cargo add r2d2`.

---

### Task 1: `Embedding` value type

**Files:**
- Modify: `src/classify.rs` (add type near `ClassificationInput`, ~line 63)
- Test: `src/classify.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub struct Embedding { pub vector: Vec<f32> }` with `Embedding::new(Vec<f32>) -> Embedding` and `fn dim(&self) -> usize`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn embedding_reports_its_dimension() {
    let e = Embedding::new(vec![0.0, 1.0, 0.0]);
    assert_eq!(e.dim(), 3);
    assert_eq!(e.vector, vec![0.0, 1.0, 0.0]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib classify::tests::embedding_reports_its_dimension`
Expected: FAIL — `cannot find type Embedding`.

- [ ] **Step 3: Write minimal implementation**

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib classify::tests::embedding_reports_its_dimension`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/classify.rs
git commit -s -m "feat(classify): add Embedding value type for two-stage forward"
```

---

### Task 2: Split `ClassifierRuntime` into `embed` + `rank` with a default `classify`

This is the architectural pivot. Add two required methods and a provided `classify`, then port `CandleClassifier` and `ClassifyService` to implement `embed`/`rank` (dropping their explicit `classify`). `ServiceCore` keeps its own `classify` override and delegates `embed`/`rank` to the inner runtime.

**Files:**
- Modify: `src/classify.rs` — `ClassifierRuntime` trait (~line 141), `CandleClassifier` impl (~line 552), `ClassifyService` impl (~line 728), `ServiceCore` impl (~line 247)
- Test: `src/classify.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Embedding` (Task 1).
- Produces:
  ```rust
  fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError>;
  fn rank(&self, embedding: &Embedding, input: &ClassificationInput) -> Result<ClassificationResult, ClassifyError>;
  // provided:
  fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError>;
  ```

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/classify.rs`. This proves the split reproduces the synthetic path exactly, and that `embed` is callable independently.

```rust
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
    assert_eq!(staged, one_shot, "embed+rank must equal the provided classify");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib classify::tests::embed_then_rank_matches_classify_on_synthetic`
Expected: FAIL — `no method named embed`.

- [ ] **Step 3: Write minimal implementation**

3a. Replace the `ClassifierRuntime` trait body (keep the doc comment):

```rust
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
    fn metadata(&self) -> RuntimeMetadata;
}
```

3b. Port `CandleClassifier`. Replace `real_forward` and the `impl ClassifierRuntime for CandleClassifier` `classify` with `embed`/`rank`. Keep the counter semantics: `embed` increments BOTH `tokenizer_calls` and `forward_calls` (embed_ids is the model forward) and records the `Tokenize` + `Forward` stages; `rank` increments neither.

```rust
impl ClassifierRuntime for CandleClassifier {
    fn metadata(&self) -> RuntimeMetadata { /* unchanged */ }

    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        let text = input.text.trim();
        // Tokenize stage (AC-012).
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

    fn rank(
        &self,
        embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        let v = &embedding.vector;
        let (ranked, identity) = match self.taxonomy.as_ref() {
            Some(t) => (
                anchor_rank(v, &t.anchors, t.top_k),
                (t.classifier_id.clone(), t.model_revision.clone(), t.taxonomy_revision.clone()),
            ),
            None => (
                cosine_rank(v, &self.prototypes),
                (CLASSIFIER_ID.to_string(), MODEL_REVISION.to_string(), TAXONOMY_REVISION.to_string()),
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
```

Delete the now-unused `real_forward` method.

3c. Port `ClassifyService` (synthetic). Replace its `impl ClassifierRuntime` `classify` and refactor `deterministic_classify`:

```rust
impl ClassifierRuntime for ClassifyService {
    fn metadata(&self) -> RuntimeMetadata { /* unchanged */ }

    fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        let context = input.text.trim();
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

    fn rank(
        &self,
        embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
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
```

Delete the now-unused `deterministic_classify` method.

3d. Add `embed`/`rank` delegation to `ServiceCore`'s `impl ClassifierRuntime` (it keeps its own `classify` override from the existing code, unchanged for now — the L2 wiring lands in Task 4):

```rust
fn embed(&self, input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
    self.runtime.embed(input)
}
fn rank(
    &self,
    embedding: &Embedding,
    input: &ClassificationInput,
) -> Result<ClassificationResult, ClassifyError> {
    self.runtime.rank(embedding, input)
}
```

- [ ] **Step 4: Run the full lib test suite to verify no regression**

Run: `cargo test --lib`
Expected: PASS, including `u070_*`, `u071_*`, `u001_*`, and the new `embed_then_rank_matches_classify_on_synthetic`. (Weight/parity `#[ignore]`d tests remain ignored.)

- [ ] **Step 5: Commit**

```bash
git add src/classify.rs
git commit -s -m "refactor(classify): split ClassifierRuntime into embed+rank, keep classify as default"
```

---

### Task 3: `SemanticCache` trait + `NoopSemanticCache` + identity tag helper

**Files:**
- Modify: `src/cache.rs` (append trait + noop + helper; add `use crate::classify::Embedding;`)
- Test: `src/cache.rs` (`#[cfg(test)] mod semantic_tests`)

**Interfaces:**
- Consumes: `Embedding`, `ClassificationResult`.
- Produces:
  ```rust
  pub trait SemanticCache: Send + Sync {
      fn lookup(&self, embedding: &Embedding, identity: &str) -> Option<ClassificationResult>;
      fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str);
  }
  pub struct NoopSemanticCache;
  pub fn identity_tag(id: (&str, &str, &str, &str)) -> String;
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod semantic_tests {
    use super::*;
    use crate::classify::{ClassificationResult, ClassifyStatus, Embedding, RankedSignal};

    fn result(id: &str) -> ClassificationResult {
        ClassificationResult {
            classifier_id: "c".into(), model_revision: "m".into(),
            tokenizer_revision: "t".into(), taxonomy_revision: "x".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal { id: id.into(), score: 1.0 }],
        }
    }

    #[test]
    fn noop_semantic_cache_never_hits() {
        let cache = NoopSemanticCache;
        let e = Embedding::new(vec![1.0, 0.0]);
        cache.insert(&e, &result("simple"), "c|m|t|x");
        assert!(cache.lookup(&e, "c|m|t|x").is_none(), "noop must always miss");
    }

    #[test]
    fn identity_tag_is_stable_and_field_separated() {
        assert_eq!(identity_tag(("c", "m", "t", "x")), "c|m|t|x");
        assert_ne!(identity_tag(("a", "bc", "d", "e")), identity_tag(("ab", "c", "d", "e")));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cache::semantic_tests`
Expected: FAIL — `cannot find type NoopSemanticCache`.

- [ ] **Step 3: Write minimal implementation**

```rust
use crate::classify::Embedding;

/// The pluggable L2 (semantic / approximate) cache seam.
///
/// Interposes on the embedding between `embed` and `rank`. It is BEST-EFFORT:
/// `lookup` returns `None` on any error (fail-open to compute) and `insert`
/// is fire-and-forget. `identity` isolates entries by classifier/model/
/// tokenizer/taxonomy so a revision change can never serve a stale label.
pub trait SemanticCache: Send + Sync {
    /// Return a stored result whose embedding is within the configured
    /// similarity threshold of `embedding` and shares `identity`, else `None`.
    fn lookup(&self, embedding: &Embedding, identity: &str) -> Option<ClassificationResult>;

    /// Record `result` under `embedding` and `identity`. Best-effort; never blocks.
    fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str);
}

/// The default L2 cache: always misses, never stores. Zero cost when the
/// semantic tier is disabled.
pub struct NoopSemanticCache;

impl SemanticCache for NoopSemanticCache {
    fn lookup(&self, _embedding: &Embedding, _identity: &str) -> Option<ClassificationResult> {
        None
    }
    fn insert(&self, _embedding: &Embedding, _result: &ClassificationResult, _identity: &str) {}
}

/// Build the L2 isolation tag from a cache-identity tuple (same fields as the
/// blake3 L1 key), pipe-separated so field boundaries cannot alias.
pub fn identity_tag(id: (&str, &str, &str, &str)) -> String {
    format!("{}|{}|{}|{}", id.0, id.1, id.2, id.3)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cache::semantic_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs
git commit -s -m "feat(cache): add SemanticCache trait, NoopSemanticCache, identity_tag"
```

---

### Task 4: Wire the L2 tier into `ServiceCore` (default Noop, behavior unchanged)

**Files:**
- Modify: `src/classify.rs` — `ServiceCore` struct + constructors + `classify` override (~lines 205-314)
- Test: `src/classify.rs` (`mod tests`)

**Interfaces:**
- Consumes: `SemanticCache`, `NoopSemanticCache`, `identity_tag` (Task 3); `embed`/`rank` (Task 2).
- Produces: `ServiceCore::with_semantic_cache(runtime, metrics, Arc<dyn SemanticCache>) -> Self`; the two-tier orchestration in `ServiceCore::classify`.

- [ ] **Step 1: Write the failing test**

A spy `SemanticCache` proves ServiceCore consults L2 on an L1 miss and serves its hit without ranking; and that a default `ServiceCore` (Noop) is unaffected.

```rust
#[test]
fn service_core_serves_semantic_hit_without_ranking() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;
    use crate::cache::SemanticCache;

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
        classifier_id: "spy".into(), model_revision: "m".into(),
        tokenizer_revision: "t".into(), taxonomy_revision: "x".into(),
        status: ClassifyStatus::Ok,
        ranked: vec![RankedSignal { id: "SEMANTIC_HIT".into(), score: 0.99 }],
    };
    let lookups = StdArc::new(AtomicUsize::new(0));
    let spy = StdArc::new(SpyCache { canned: canned.clone(), lookups: lookups.clone() });

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
    assert_eq!(lookups.load(Ordering::SeqCst), 1, "L1 miss must consult L2 once");
    assert_eq!(out.ranked[0].id, "SEMANTIC_HIT", "L2 hit must be served verbatim");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib classify::tests::service_core_serves_semantic_hit_without_ranking`
Expected: FAIL — `no function named with_semantic_cache`.

- [ ] **Step 3: Write minimal implementation**

3a. Add the field and import:

```rust
use crate::cache::{identity_tag, CacheKey, NoopSemanticCache, SemanticCache, SharedCache};
// ...
#[derive(Clone)]
pub struct ServiceCore<R> {
    runtime: Arc<R>,
    cache: SharedCache,
    semantic: Arc<dyn SemanticCache>,
    metrics: Metrics,
}
```

3b. Default constructors keep Noop; add the opt-in constructor:

```rust
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
```

3c. Replace the forward closure in `ServiceCore::classify` so the L1-miss path runs `embed → L2 lookup → rank → L2 insert`. Keep the `forward_ran` flag (it still partitions L1 hit/miss metrics) and the existing key construction:

```rust
let tag = identity_tag(meta.cache_identity());
let forward = {
    let runtime = self.runtime.clone();
    let semantic = self.semantic.clone();
    let forward_ran = forward_ran.clone();
    let tag = tag.clone();
    let input = ClassificationInput {
        text: normalized,
        requested_signals: input.requested_signals,
        session_metadata: input.session_metadata,
    };
    move || {
        forward_ran.store(true, Ordering::SeqCst);
        // Embed once. Reused by both the L2 lookup and the ranker.
        let embedding = runtime.embed(&input)?;
        // L2 semantic lookup (fail-open: None on any Redis trouble).
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
```

Note: `meta` is already bound above (`let meta = self.runtime.metadata();`). Compute `tag` right after it.

- [ ] **Step 4: Run the full lib suite**

Run: `cargo test --lib`
Expected: PASS — new test passes, and `u071_*` (Noop default) still passes unchanged.

- [ ] **Step 5: Commit**

```bash
git add src/classify.rs
git commit -s -m "feat(classify): orchestrate L1->embed->L2->rank in ServiceCore (Noop default)"
```

---

### Task 5: Cache configuration + strategy registry

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs` (`mod tests`)

**Interfaces:**
- Produces:
  ```rust
  pub const KNOWN_CACHE_STRATEGIES: &[&str] = &["exact", "redis-semantic"];
  pub struct CacheConfig {
      pub strategy: String,      // "exact" | "redis-semantic"
      pub redis_url: Option<String>,
      pub threshold: f32,        // cosine similarity, 0.0..=1.0
      pub ttl_secs: u64,
      pub timeout_ms: u64,
  }
  impl CacheConfig { pub fn from_env() -> Result<CacheConfig, ConfigError>; }
  // new ConfigError variants: UnknownCacheStrategy(String), MissingRedisUrl, InvalidThreshold(f32)
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn cache_config_defaults_to_exact() {
    let cfg = CacheConfig::from_env_with(|_| None).expect("defaults");
    assert_eq!(cfg.strategy, "exact");
    assert!(cfg.redis_url.is_none());
    assert!((cfg.threshold - 0.90).abs() < 1e-6);
    assert_eq!(cfg.ttl_secs, 86_400);
    assert_eq!(cfg.timeout_ms, 50);
}

#[test]
fn redis_semantic_requires_url() {
    let get = |k: &str| match k {
        "LLM_D_SC_CACHE" => Some("redis-semantic".to_string()),
        _ => None,
    };
    match CacheConfig::from_env_with(get) {
        Err(ConfigError::MissingRedisUrl) => {}
        other => panic!("expected MissingRedisUrl, got {other:?}"),
    }
}

#[test]
fn unknown_cache_strategy_rejected() {
    let get = |k: &str| match k {
        "LLM_D_SC_CACHE" => Some("memcached".to_string()),
        _ => None,
    };
    match CacheConfig::from_env_with(get) {
        Err(ConfigError::UnknownCacheStrategy(s)) => assert_eq!(s, "memcached"),
        other => panic!("expected UnknownCacheStrategy, got {other:?}"),
    }
}

#[test]
fn threshold_out_of_range_rejected() {
    let get = |k: &str| match k {
        "LLM_D_SC_CACHE" => Some("exact".to_string()),
        "LLM_D_SC_CACHE_THRESHOLD" => Some("1.5".to_string()),
        _ => None,
    };
    match CacheConfig::from_env_with(get) {
        Err(ConfigError::InvalidThreshold(v)) => assert!((v - 1.5).abs() < 1e-6),
        other => panic!("expected InvalidThreshold, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib config::tests::cache_config_defaults_to_exact`
Expected: FAIL — `cannot find type CacheConfig`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/config.rs`:

```rust
/// Cache strategies this crate can host.
pub const KNOWN_CACHE_STRATEGIES: &[&str] = &["exact", "redis-semantic"];

/// L2 semantic-cache configuration, resolved from the environment.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheConfig {
    pub strategy: String,
    pub redis_url: Option<String>,
    pub threshold: f32,
    pub ttl_secs: u64,
    pub timeout_ms: u64,
}

impl CacheConfig {
    /// Resolve from process environment variables.
    pub fn from_env() -> Result<CacheConfig, ConfigError> {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    /// Resolve from an arbitrary getter (injected for tests).
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Result<CacheConfig, ConfigError> {
        let strategy = get("LLM_D_SC_CACHE").unwrap_or_else(|| "exact".to_string());
        if !KNOWN_CACHE_STRATEGIES.contains(&strategy.as_str()) {
            return Err(ConfigError::UnknownCacheStrategy(strategy));
        }
        let redis_url = get("LLM_D_SC_REDIS_URL").filter(|s| !s.trim().is_empty());
        if strategy == "redis-semantic" && redis_url.is_none() {
            return Err(ConfigError::MissingRedisUrl);
        }
        let threshold = get("LLM_D_SC_CACHE_THRESHOLD")
            .map(|s| s.parse::<f32>().map_err(|_| ConfigError::InvalidThreshold(f32::NAN)))
            .transpose()?
            .unwrap_or(0.90);
        if !(0.0..=1.0).contains(&threshold) {
            return Err(ConfigError::InvalidThreshold(threshold));
        }
        let ttl_secs = get("LLM_D_SC_CACHE_TTL")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(86_400);
        let timeout_ms = get("LLM_D_SC_CACHE_TIMEOUT_MS")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(50);
        Ok(CacheConfig { strategy, redis_url, threshold, ttl_secs, timeout_ms })
    }
}
```

Extend `ConfigError`:

```rust
pub enum ConfigError {
    MissingClassifiers,
    UnknownBackend(String),
    DuplicateClassifierId(String),
    InvalidModelPath(String),
    Parse(String),
    UnknownCacheStrategy(String),
    MissingRedisUrl,
    InvalidThreshold(f32),
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config::tests`
Expected: PASS (all new + existing `u001..u005`).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -s -m "feat(config): add CacheConfig + redis-semantic strategy registry"
```

---

### Task 6: Circuit breaker (pure, unit-tested)

Keeps a dead/slow Redis from costing one timeout per request: after N consecutive failures, `allow()` returns false for a cooldown window.

**Files:**
- Create: `src/cache/breaker.rs`
- Modify: `src/cache.rs` — add `pub mod breaker;` (convert `cache.rs` usage as needed; keep `cache.rs` as the module root and add the submodule declaration at its top)
- Test: `src/cache/breaker.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  ```rust
  pub struct CircuitBreaker { /* private */ }
  impl CircuitBreaker {
      pub fn new(failure_threshold: u32, cooldown: std::time::Duration) -> Self;
      pub fn allow(&self) -> bool;
      pub fn record_success(&self);
      pub fn record_failure(&self);
  }
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn opens_after_threshold_and_closes_after_cooldown() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(50));
        assert!(cb.allow(), "starts closed");
        cb.record_failure();
        assert!(cb.allow(), "one failure below threshold");
        cb.record_failure();
        assert!(!cb.allow(), "opens at threshold");
        std::thread::sleep(Duration::from_millis(60));
        assert!(cb.allow(), "closes (half-open) after cooldown");
    }

    #[test]
    fn success_resets_failures() {
        let cb = CircuitBreaker::new(2, Duration::from_secs(10));
        cb.record_failure();
        cb.record_success();
        cb.record_failure();
        assert!(cb.allow(), "success cleared the earlier failure");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cache::breaker`
Expected: FAIL — `cannot find type CircuitBreaker`.

- [ ] **Step 3: Write minimal implementation**

`src/cache/breaker.rs`:

```rust
//! A minimal consecutive-failure circuit breaker for the best-effort L2 cache.
//! After `failure_threshold` consecutive failures it opens for `cooldown`,
//! so a dead Redis costs one probe per cooldown window, not one per request.

use std::sync::Mutex;
use std::time::{Duration, Instant};

struct State {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

pub struct CircuitBreaker {
    failure_threshold: u32,
    cooldown: Duration,
    state: Mutex<State>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        CircuitBreaker {
            failure_threshold: failure_threshold.max(1),
            cooldown,
            state: Mutex::new(State { consecutive_failures: 0, open_until: None }),
        }
    }

    /// True if a call may proceed (closed, or half-open after cooldown).
    pub fn allow(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        match s.open_until {
            Some(t) if Instant::now() < t => false,
            Some(_) => {
                // Cooldown elapsed: half-open, let one probe through.
                s.open_until = None;
                s.consecutive_failures = 0;
                true
            }
            None => true,
        }
    }

    pub fn record_success(&self) {
        let mut s = self.state.lock().unwrap();
        s.consecutive_failures = 0;
        s.open_until = None;
    }

    pub fn record_failure(&self) {
        let mut s = self.state.lock().unwrap();
        s.consecutive_failures += 1;
        if s.consecutive_failures >= self.failure_threshold {
            s.open_until = Some(Instant::now() + self.cooldown);
        }
    }
}
```

Add to the top of `src/cache.rs`: `pub mod breaker;`

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cache::breaker`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/cache/breaker.rs
git commit -s -m "feat(cache): add consecutive-failure circuit breaker"
```

---

### Task 7: Redis entry codec + KNN query builder (pure, unit-tested)

Isolate every piece of the Redis integration that does NOT need a live server: the vector→bytes encoding, the `FT.SEARCH` KNN argument list, the index name, and the result JSON codec. This makes Task 8 thin.

**Files:**
- Create: `src/cache/redis_codec.rs`
- Modify: `src/cache.rs` — add `pub mod redis_codec;`
- Test: `src/cache/redis_codec.rs` (inline `#[cfg(test)]`)
- Add dependency: `cargo add serde_json` is already present; no new dep here.

**Interfaces:**
- Produces:
  ```rust
  pub fn vector_to_bytes(v: &[f32]) -> Vec<u8>;             // little-endian f32 blob
  pub fn index_name() -> &'static str;                       // "sc_semantic_idx"
  pub fn encode_result(r: &crate::classify::ClassificationResult) -> String;   // JSON
  pub fn decode_result(s: &str) -> Option<crate::classify::ClassificationResult>;
  pub fn cosine_score_from_distance(distance: f32) -> f32;   // 1.0 - distance
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{ClassificationResult, ClassifyStatus, RankedSignal};

    #[test]
    fn vector_bytes_are_little_endian_f32() {
        let bytes = vector_to_bytes(&[1.0f32, 2.0f32]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2.0f32.to_le_bytes());
    }

    #[test]
    fn result_round_trips_through_json() {
        let r = ClassificationResult {
            classifier_id: "complexity".into(), model_revision: "m".into(),
            tokenizer_revision: "t".into(), taxonomy_revision: "x".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal { id: "SIMPLE".into(), score: 0.87 }],
        };
        let encoded = encode_result(&r);
        let decoded = decode_result(&encoded).expect("decode");
        assert_eq!(decoded, r);
    }

    #[test]
    fn cosine_score_is_one_minus_distance() {
        assert!((cosine_score_from_distance(0.1) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn bad_json_decodes_to_none() {
        assert!(decode_result("not json").is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cache::redis_codec`
Expected: FAIL — `cannot find function vector_to_bytes`.

- [ ] **Step 3: Write minimal implementation**

This requires `serde` derives on the result types. Add `#[derive(serde::Serialize, serde::Deserialize)]` to `ClassificationResult`, `RankedSignal`, and `ClassifyStatus` in `src/classify.rs` (they already derive `Debug, Clone, PartialEq`). Then:

`src/cache/redis_codec.rs`:

```rust
//! Pure (no-I/O) codec + query-shape helpers for the Redis semantic cache, so
//! the byte encoding, index name, and result serialization are unit-testable
//! without a live Redis.

use crate::classify::ClassificationResult;

/// The RediSearch index over the semantic-cache hash keys.
pub fn index_name() -> &'static str {
    "sc_semantic_idx"
}

/// Encode an embedding as the little-endian f32 blob RediSearch expects for a
/// FLOAT32 vector field.
pub fn vector_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Serialize a classification result for storage.
pub fn encode_result(r: &ClassificationResult) -> String {
    serde_json::to_string(r).expect("ClassificationResult serializes")
}

/// Deserialize a stored result; `None` on any corruption (treated as a miss).
pub fn decode_result(s: &str) -> Option<ClassificationResult> {
    serde_json::from_str(s).ok()
}

/// RediSearch returns COSINE *distance* (0 = identical). Convert to a
/// similarity score in [0, 1] for threshold comparison.
pub fn cosine_score_from_distance(distance: f32) -> f32 {
    1.0 - distance
}
```

Add `pub mod redis_codec;` to `src/cache.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib cache::redis_codec`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/cache/redis_codec.rs src/classify.rs
git commit -s -m "feat(cache): add pure Redis codec/query helpers + serde on result types"
```

---

### Task 8: `RedisSemanticCache` — lookup with fail-open + circuit breaker

Implements `SemanticCache` against Redis Stack. Uses the sync `redis` crate with an `r2d2` pool and per-op timeouts. Lookup wraps every Redis interaction so ANY error returns `None`. Live-Redis behavior is exercised by an `#[ignore]`d integration test (repo convention for external resources); the fail-open path is unit-tested without a server by pointing at a dead address.

**Files:**
- Create: `src/cache/redis.rs`
- Modify: `src/cache.rs` — add `pub mod redis;`
- Test: `tests/redis_semantic.rs` (integration; `#[ignore]` for the live-Redis cases)
- Add dependencies: `cargo add redis --features r2d2` and `cargo add r2d2`

**Interfaces:**
- Consumes: `SemanticCache`, `CircuitBreaker`, `redis_codec::*`, `CacheConfig`.
- Produces:
  ```rust
  pub struct RedisSemanticCache { /* private */ }
  impl RedisSemanticCache {
      pub fn connect(cfg: &crate::config::CacheConfig, metrics: crate::metrics::Metrics)
          -> Result<RedisSemanticCache, String>;
  }
  impl crate::cache::SemanticCache for RedisSemanticCache { /* lookup + insert */ }
  ```

- [ ] **Step 1: Write the failing tests**

`tests/redis_semantic.rs`:

```rust
use llm_d_sc::cache::redis::RedisSemanticCache;
use llm_d_sc::cache::SemanticCache;
use llm_d_sc::classify::{ClassificationResult, ClassifyStatus, Embedding, RankedSignal};
use llm_d_sc::config::CacheConfig;
use llm_d_sc::metrics::Metrics;

fn cfg(url: &str) -> CacheConfig {
    CacheConfig {
        strategy: "redis-semantic".into(),
        redis_url: Some(url.into()),
        threshold: 0.90,
        ttl_secs: 3600,
        timeout_ms: 50,
    }
}

fn result(id: &str) -> ClassificationResult {
    ClassificationResult {
        classifier_id: "complexity".into(), model_revision: "m".into(),
        tokenizer_revision: "t".into(), taxonomy_revision: "x".into(),
        status: ClassifyStatus::Ok,
        ranked: vec![RankedSignal { id: id.into(), score: 0.9 }],
    }
}

#[test]
fn lookup_is_fail_open_when_redis_is_unreachable() {
    // Port 1 is never a Redis; connect may succeed lazily, but lookup must
    // never panic or error — it returns None (fail-open to compute).
    let cache = RedisSemanticCache::connect(&cfg("redis://127.0.0.1:1"), Metrics::new());
    if let Ok(cache) = cache {
        let e = Embedding::new(vec![0.1, 0.2, 0.3]);
        assert!(cache.lookup(&e, "complexity|m|t|x").is_none());
        // insert must not panic either.
        cache.insert(&e, &result("SIMPLE"), "complexity|m|t|x");
    }
}

#[test]
#[ignore] // requires a running Redis Stack at REDIS_URL (see hack/redis-stack.sh)
fn paraphrase_hit_after_insert() {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL for the live test");
    let cache = RedisSemanticCache::connect(&cfg(&url), Metrics::new()).expect("connect");
    let a = Embedding::new(vec![1.0, 0.0, 0.0]);
    let close = Embedding::new(vec![0.98, 0.02, 0.0]); // near-identical direction
    cache.insert(&a, &result("SIMPLE"), "complexity|m|t|x");
    std::thread::sleep(std::time::Duration::from_millis(100)); // async write-back settles
    let hit = cache.lookup(&close, "complexity|m|t|x");
    assert_eq!(hit.map(|r| r.ranked[0].id.clone()), Some("SIMPLE".into()));
}

#[test]
#[ignore] // requires a running Redis Stack at REDIS_URL
fn identity_isolates_across_revisions() {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL for the live test");
    let cache = RedisSemanticCache::connect(&cfg(&url), Metrics::new()).expect("connect");
    let e = Embedding::new(vec![0.0, 1.0, 0.0]);
    cache.insert(&e, &result("SIMPLE"), "complexity|m1|t|x");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(cache.lookup(&e, "complexity|m2|t|x").is_none(), "different revision must not hit");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test redis_semantic lookup_is_fail_open_when_redis_is_unreachable`
Expected: FAIL — unresolved import `llm_d_sc::cache::redis`.

- [ ] **Step 3: Write minimal implementation**

`src/cache/redis.rs`. Uses the pure helpers from Task 7 and the breaker from Task 6. The write path is deferred to Task 9; here `insert` may be a direct best-effort write (Task 9 makes it async). Key points: build the index on connect (idempotent — ignore "Index already exists"); store hashes under `sc:{tag}:{blake3(vector-bytes)}`; run `FT.SEARCH` KNN 1 filtered by the `identity` TAG; compare `cosine_score_from_distance(distance) >= threshold`.

```rust
//! Redis Stack (RediSearch) semantic cache. BEST-EFFORT and FAIL-OPEN: every
//! Redis interaction is wrapped so any error/timeout yields "no hit" (lookup)
//! or a dropped write (insert). Guarded by a circuit breaker so a dead Redis
//! costs one probe per cooldown, not one timeout per request.

use std::time::Duration;

use r2d2::Pool;
use redis::Client;

use crate::cache::breaker::CircuitBreaker;
use crate::cache::redis_codec::{
    cosine_score_from_distance, decode_result, encode_result, index_name, vector_to_bytes,
};
use crate::cache::SemanticCache;
use crate::classify::{ClassificationResult, Embedding};
use crate::config::CacheConfig;
use crate::metrics::Metrics;

pub struct RedisSemanticCache {
    pool: Pool<Client>,
    threshold: f32,
    ttl_secs: u64,
    breaker: CircuitBreaker,
    metrics: Metrics,
}

impl RedisSemanticCache {
    /// Connect, size the pool small (matches the inference pool width), set
    /// per-op timeouts, and ensure the vector index exists.
    pub fn connect(cfg: &CacheConfig, metrics: Metrics) -> Result<RedisSemanticCache, String> {
        let url = cfg.redis_url.as_ref().ok_or("redis-semantic requires a URL")?;
        let client = Client::open(url.as_str()).map_err(|e| e.to_string())?;
        let pool = Pool::builder()
            .max_size(8)
            .connection_timeout(Duration::from_millis(cfg.timeout_ms))
            .build(client)
            .map_err(|e| e.to_string())?;
        let cache = RedisSemanticCache {
            pool,
            threshold: cfg.threshold,
            ttl_secs: cfg.ttl_secs,
            breaker: CircuitBreaker::new(5, Duration::from_secs(10)),
            metrics,
        };
        // Best-effort index creation; a missing index only means lookups miss.
        let _ = cache.ensure_index();
        Ok(cache)
    }

    /// Create the FLAT COSINE vector index if absent. Idempotent: an
    /// "Index already exists" error is treated as success.
    fn ensure_index(&self) -> Result<(), String> {
        // NOTE: dimension is discovered from the first stored vector; RediSearch
        // requires DIM at create time, so create lazily on first insert instead
        // (see insert). This function is a no-op placeholder kept for symmetry.
        Ok(())
    }

    fn with_timeout_conn<T>(
        &self,
        f: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, String> {
        let mut conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.set_read_timeout(Some(Duration::from_millis(50))).ok();
        conn.set_write_timeout(Some(Duration::from_millis(50))).ok();
        f(&mut conn).map_err(|e| e.to_string())
    }
}

impl SemanticCache for RedisSemanticCache {
    fn lookup(&self, embedding: &Embedding, identity: &str) -> Option<ClassificationResult> {
        if !self.breaker.allow() {
            return None;
        }
        let blob = vector_to_bytes(&embedding.vector);
        // FT.SEARCH sc_semantic_idx "(@identity:{<tag>})=>[KNN 1 @vec $BLOB AS dist]"
        //   PARAMS 2 BLOB <blob> DIALECT 2 SORTBY dist RETURN 2 dist payload
        let query = format!("(@identity:{{{identity}}})=>[KNN 1 @vec $BLOB AS dist]");
        let outcome = self.with_timeout_conn(|conn| {
            redis::cmd("FT.SEARCH")
                .arg(index_name())
                .arg(&query)
                .arg("PARAMS").arg(2).arg("BLOB").arg(blob.as_slice())
                .arg("SORTBY").arg("dist")
                .arg("RETURN").arg(2).arg("dist").arg("payload")
                .arg("DIALECT").arg(2)
                .query::<redis::Value>(conn)
        });
        match outcome {
            Ok(value) => {
                self.breaker.record_success();
                match parse_knn_reply(&value) {
                    Some((distance, payload)) if cosine_score_from_distance(distance) >= self.threshold => {
                        self.metrics.record_l2_hit();
                        decode_result(&payload)
                    }
                    _ => {
                        self.metrics.record_l2_miss();
                        None
                    }
                }
            }
            Err(_) => {
                self.breaker.record_failure();
                self.metrics.record_l2_degraded();
                None
            }
        }
    }

    fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str) {
        if !self.breaker.allow() {
            return;
        }
        let blob = vector_to_bytes(&embedding.vector);
        let dim = embedding.dim();
        let payload = encode_result(result);
        let key = format!("sc:{identity}:{}", blake3::hash(&blob).to_hex());
        let ttl = self.ttl_secs;
        let outcome = self.with_timeout_conn(move |conn| {
            // Create the index lazily now that we know the dimension. Ignore the
            // "already exists" error via a best-effort call.
            let _ = redis::cmd("FT.CREATE")
                .arg(index_name())
                .arg("ON").arg("HASH")
                .arg("PREFIX").arg(1).arg("sc:")
                .arg("SCHEMA")
                .arg("identity").arg("TAG")
                .arg("payload").arg("TEXT")
                .arg("vec").arg("VECTOR").arg("FLAT").arg(6)
                .arg("TYPE").arg("FLOAT32")
                .arg("DIM").arg(dim)
                .arg("DISTANCE_METRIC").arg("COSINE")
                .query::<redis::Value>(conn);
            redis::cmd("HSET")
                .arg(&key)
                .arg("identity").arg(identity)
                .arg("payload").arg(&payload)
                .arg("vec").arg(blob.as_slice())
                .query::<redis::Value>(conn)?;
            redis::cmd("EXPIRE").arg(&key).arg(ttl).query::<redis::Value>(conn)
        });
        match outcome {
            Ok(_) => self.breaker.record_success(),
            Err(_) => {
                self.breaker.record_failure();
                self.metrics.record_l2_degraded();
            }
        }
    }
}

/// Extract (distance, payload) from a RediSearch FT.SEARCH reply, or None if the
/// reply shape is empty/unexpected. Fail-open: any parse surprise is a miss.
fn parse_knn_reply(value: &redis::Value) -> Option<(f32, String)> {
    // FT.SEARCH returns: [count, key, [field, val, field, val, ...], ...]
    if let redis::Value::Bulk(items) = value {
        // items[0] = count; the field array is the 3rd element (index 2).
        if let Some(redis::Value::Bulk(fields)) = items.get(2) {
            let mut dist: Option<f32> = None;
            let mut payload: Option<String> = None;
            let mut i = 0;
            while i + 1 < fields.len() {
                let name = as_string(&fields[i]);
                let val = as_string(&fields[i + 1]);
                match name.as_deref() {
                    Some("dist") => dist = val.and_then(|s| s.parse::<f32>().ok()),
                    Some("payload") => payload = val,
                    _ => {}
                }
                i += 2;
            }
            if let (Some(d), Some(p)) = (dist, payload) {
                return Some((d, p));
            }
        }
    }
    None
}

fn as_string(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::Data(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        redis::Value::Status(s) => Some(s.clone()),
        _ => None,
    }
}
```

Add `pub mod redis;` to `src/cache.rs`. Ensure `lib.rs` exposes `pub mod cache;` (already does) so `llm_d_sc::cache::redis` resolves.

> Note on the `redis` crate version: `cargo add` pins the current release. If the installed version renames `redis::Value::Bulk`/`Data` (newer versions use `Array`/`BulkString`), update `parse_knn_reply`/`as_string` to the variants your `Cargo.lock` shows — run `cargo doc -p redis --open` to confirm the enum. The logic is unchanged.

- [ ] **Step 4: Run the fail-open unit test + full build**

Run: `cargo test --test redis_semantic lookup_is_fail_open_when_redis_is_unreachable && cargo build`
Expected: PASS / builds. (The two `#[ignore]`d live tests are skipped.)

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/cache/redis.rs tests/redis_semantic.rs Cargo.toml Cargo.lock
git commit -s -m "feat(cache): add fail-open RedisSemanticCache (KNN lookup + lazy index)"
```

---

### Task 9: Async fire-and-forget write-back

Move `insert`'s Redis write off the caller's thread onto a bounded channel + a background worker, so the inference thread returns immediately and a burst of misses can never block on Redis.

**Files:**
- Modify: `src/cache/redis.rs`
- Test: `tests/redis_semantic.rs` (add a non-ignored test that `insert` returns promptly even against a dead Redis)

**Interfaces:**
- Unchanged public surface (`insert` signature stays). Internally spawns one worker thread and holds a `std::sync::mpsc::SyncSender<WriteJob>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn insert_returns_immediately_even_when_redis_is_down() {
    let cache = RedisSemanticCache::connect(&cfg("redis://127.0.0.1:1"), Metrics::new());
    if let Ok(cache) = cache {
        let e = Embedding::new(vec![0.1, 0.2, 0.3]);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            cache.insert(&e, &result("SIMPLE"), "complexity|m|t|x");
        }
        assert!(start.elapsed() < std::time::Duration::from_millis(200),
            "100 inserts must not block on Redis; write-back is async");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test redis_semantic insert_returns_immediately_even_when_redis_is_down`
Expected: FAIL — synchronous inserts each pay the connection timeout (~50ms × 100).

- [ ] **Step 3: Write minimal implementation**

Add a write channel and worker. Keep the actual Redis write logic (from Task 8's `insert`) in a private `fn write_now(&…)`; `insert` now just enqueues.

```rust
enum WriteJob {
    Put { blob: Vec<u8>, dim: usize, payload: String, identity: String },
}

// In the struct:
//   writer: std::sync::mpsc::SyncSender<WriteJob>,

// In connect(), after building `cache` fields, spawn the worker with a clone of
// the pool/breaker/metrics/ttl and a bounded channel (capacity 1024). On a full
// channel, `try_send` drops the job (best-effort). The worker loops on recv and
// performs the HSET/EXPIRE (+ lazy FT.CREATE) exactly as Task 8's insert did.
```

Concretely, replace `insert`:

```rust
fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str) {
    let job = WriteJob::Put {
        blob: vector_to_bytes(&embedding.vector),
        dim: embedding.dim(),
        payload: encode_result(result),
        identity: identity.to_string(),
    };
    // Best-effort: a full queue drops the write rather than blocking the caller.
    let _ = self.writer.try_send(job);
}
```

And in `connect`, build the channel + worker:

```rust
let (tx, rx) = std::sync::mpsc::sync_channel::<WriteJob>(1024);
{
    let pool = pool.clone();
    let breaker_metrics = metrics.clone();
    let ttl = cfg.ttl_secs;
    std::thread::Builder::new()
        .name("sc-l2-writeback".into())
        .spawn(move || {
            let breaker = CircuitBreaker::new(5, Duration::from_secs(10));
            for job in rx {
                if !breaker.allow() { continue; }
                let WriteJob::Put { blob, dim, payload, identity } = job;
                let key = format!("sc:{identity}:{}", blake3::hash(&blob).to_hex());
                let write = (|| -> Result<(), String> {
                    let mut conn = pool.get().map_err(|e| e.to_string())?;
                    conn.set_write_timeout(Some(Duration::from_millis(50))).ok();
                    let _ = redis::cmd("FT.CREATE")
                        .arg(index_name()).arg("ON").arg("HASH")
                        .arg("PREFIX").arg(1).arg("sc:").arg("SCHEMA")
                        .arg("identity").arg("TAG").arg("payload").arg("TEXT")
                        .arg("vec").arg("VECTOR").arg("FLAT").arg(6)
                        .arg("TYPE").arg("FLOAT32").arg("DIM").arg(dim)
                        .arg("DISTANCE_METRIC").arg("COSINE")
                        .query::<redis::Value>(&mut conn);
                    redis::cmd("HSET").arg(&key)
                        .arg("identity").arg(&identity)
                        .arg("payload").arg(&payload)
                        .arg("vec").arg(blob.as_slice())
                        .query::<redis::Value>(&mut conn).map_err(|e| e.to_string())?;
                    redis::cmd("EXPIRE").arg(&key).arg(ttl)
                        .query::<redis::Value>(&mut conn).map_err(|e| e.to_string())?;
                    Ok(())
                })();
                match write {
                    Ok(_) => breaker.record_success(),
                    Err(_) => { breaker.record_failure(); breaker_metrics.record_l2_degraded(); }
                }
            }
        })
        .map_err(|e| e.to_string())?;
}
// add `writer: tx,` to the returned struct literal.
```

Remove the now-duplicated write body from the `SemanticCache::insert` in Task 8 (only the enqueue remains). Keep `lookup` as-is.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test redis_semantic insert_returns_immediately_even_when_redis_is_down && cargo test --test redis_semantic lookup_is_fail_open_when_redis_is_unreachable`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cache/redis.rs tests/redis_semantic.rs
git commit -s -m "feat(cache): async fire-and-forget L2 write-back with bounded queue"
```

---

### Task 10: L2 metrics counters

**Files:**
- Modify: `src/metrics.rs`
- Test: `src/metrics.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces on `Metrics`: `record_l2_hit()`, `record_l2_miss()`, `record_l2_degraded()`, and a snapshot exposing `l2_hits`, `l2_misses`, `l2_degraded`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn l2_counters_increment_independently() {
    let m = Metrics::new();
    m.record_l2_hit();
    m.record_l2_hit();
    m.record_l2_miss();
    m.record_l2_degraded();
    let s = m.snapshot();
    assert_eq!(s.l2_hits, 2);
    assert_eq!(s.l2_misses, 1);
    assert_eq!(s.l2_degraded, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib metrics::tests::l2_counters_increment_independently`
Expected: FAIL — `no method named record_l2_hit`.

- [ ] **Step 3: Write minimal implementation**

Follow the existing counter pattern in `src/metrics.rs` (mirror how `record_cache_hit`/`record_cache_miss` and the snapshot fields are implemented there — same `AtomicU64` + snapshot struct field style). Add three `AtomicU64` counters, three `record_l2_*` methods, and three `l2_*` fields on the snapshot struct.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib metrics::tests::l2_counters_increment_independently`
Expected: PASS. Then `cargo build` to confirm `src/cache/redis.rs`'s `record_l2_*` calls resolve.

- [ ] **Step 5: Commit**

```bash
git add src/metrics.rs
git commit -s -m "feat(metrics): add L2 hit/miss/degraded counters"
```

---

### Task 11: Select the cache strategy in the server binary

Wire `CacheConfig::from_env()` into `src/bin/server.rs`: when `strategy == "redis-semantic"`, build a `RedisSemanticCache` and construct the core with `ServiceCore::with_semantic_cache`; otherwise use the default (exact/Noop). A Redis connect failure at startup must NOT crash the server — log and fall back to exact (fail-open extends to boot).

**Files:**
- Modify: `src/bin/server.rs`
- Test: manual smoke (documented below) — the binary path is not unit-tested in this repo; the wiring is covered by Tasks 4/5/8 unit tests.

**Interfaces:**
- Consumes: `CacheConfig::from_env`, `RedisSemanticCache::connect`, `ServiceCore::with_semantic_cache`.

- [ ] **Step 1: Read the current core construction**

Run: `grep -n "ServiceCore" src/bin/server.rs`
Identify where the core is built around the loaded `CandleClassifier` and the shared `Metrics`.

- [ ] **Step 2: Implement strategy selection**

At core construction, replace the single `ServiceCore::with_metrics(classifier, metrics)` call with:

```rust
let cache_cfg = llm_d_sc::config::CacheConfig::from_env()
    .unwrap_or_else(|e| {
        eprintln!("invalid cache config ({e:?}); falling back to exact cache");
        llm_d_sc::config::CacheConfig {
            strategy: "exact".into(), redis_url: None,
            threshold: 0.90, ttl_secs: 86_400, timeout_ms: 50,
        }
    });
let core = if cache_cfg.strategy == "redis-semantic" {
    match llm_d_sc::cache::redis::RedisSemanticCache::connect(&cache_cfg, metrics.clone()) {
        Ok(rc) => {
            eprintln!("semantic cache enabled (redis-semantic, threshold {})", cache_cfg.threshold);
            llm_d_sc::classify::ServiceCore::with_semantic_cache(
                classifier, metrics.clone(), std::sync::Arc::new(rc),
            )
        }
        Err(e) => {
            eprintln!("redis-semantic unavailable ({e}); falling back to exact cache");
            llm_d_sc::classify::ServiceCore::with_metrics(classifier, metrics.clone())
        }
    }
} else {
    llm_d_sc::classify::ServiceCore::with_metrics(classifier, metrics.clone())
};
```

Match the actual variable names in `server.rs` (`classifier`, `metrics`) discovered in Step 1.

- [ ] **Step 3: Verify it builds and the default path is unchanged**

Run: `cargo build --bin llm-d-sc-server && cargo test`
Expected: builds; full suite green (default = exact, no behavior change).

- [ ] **Step 4: Manual smoke (optional, requires Redis Stack)**

```bash
# terminal 1
docker run --rm -p 6379:6379 redis/redis-stack-server:latest
# terminal 2
LLM_D_SC_CACHE=redis-semantic LLM_D_SC_REDIS_URL=redis://127.0.0.1:6379 \
  cargo run --bin llm-d-sc-server
# expect log: "semantic cache enabled (redis-semantic, threshold 0.9)"
```

- [ ] **Step 5: Commit**

```bash
git add src/bin/server.rs
git commit -s -m "feat(server): select cache strategy from env, fall back to exact on Redis failure"
```

---

### Task 12: Documentation + dev helper

**Files:**
- Modify: `README.md` (configuration section — add the `LLM_D_SC_CACHE*` env vars table)
- Create: `hack/redis-stack.sh` (starts a local Redis Stack for the `#[ignore]`d tests)
- Modify: `docs/` architecture doc if one enumerates the cache (search: `grep -rn "exact-result cache" docs/`)

**Interfaces:** none (docs/tooling).

- [ ] **Step 1: Write `hack/redis-stack.sh`**

```bash
#!/usr/bin/env bash
# Start a local Redis Stack (RediSearch) for the semantic-cache integration tests.
set -euo pipefail
exec docker run --rm -p 6379:6379 redis/redis-stack-server:latest
```

Make it executable: `chmod +x hack/redis-stack.sh`

- [ ] **Step 2: Document the env vars in `README.md`**

Add a subsection describing the toggle, defaults, and Redis Stack requirement, matching the env-var table in the spec (`docs/superpowers/specs/2026-08-30-semantic-cache-classifier-design.md` §7). Include the one-liner to run the live tests:

```bash
./hack/redis-stack.sh &   # start Redis Stack
REDIS_URL=redis://127.0.0.1:6379 cargo test --test redis_semantic -- --ignored
```

- [ ] **Step 3: Verify docs build/links**

Run: `cargo test --doc` (no doctests expected to break) and re-read the edited README section for accuracy against the implemented env-var names.

- [ ] **Step 4: Commit**

```bash
git add README.md hack/redis-stack.sh docs/
git commit -s -m "docs: document semantic cache toggle + add Redis Stack dev helper"
```

---

### Task 13: Full-suite verification

**Files:** none (verification only).

- [ ] **Step 1: Default path — no regression, no Redis**

Run: `cargo test`
Expected: all non-`#[ignore]` tests pass, including the existing `u040/u041/u070/u071` cache tests and the new Task 1-10 tests.

- [ ] **Step 2: Lints/format match repo standards**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean (fix any clippy findings in the new modules).

- [ ] **Step 3: Live semantic path (requires Redis Stack)**

Run:
```bash
./hack/redis-stack.sh &
REDIS_URL=redis://127.0.0.1:6379 cargo test --test redis_semantic -- --ignored
```
Expected: `paraphrase_hit_after_insert` and `identity_isolates_across_revisions` pass.

- [ ] **Step 4: Parity guard (requires model weights, if available)**

Run: `./hack/test-parity` (or `cargo test -- --ignored` after `./hack/fetch-model`)
Expected: `service_core_production_candle_cache_hit_zero_tokenizer_zero_forward` still passes — proving the embed/rank split preserved the miss=1/hit=0 counter contract.

- [ ] **Step 5: Final commit (if any lint fixes were needed)**

```bash
git add -A
git commit -s -m "chore: clippy/fmt cleanup for semantic cache tier"
```

---

## Self-Review

**Spec coverage:**
- Off-by-default toggle + registry → Task 5, Task 11. ✓
- Two-stage embed/rank (embed once) → Task 1, Task 2. ✓
- `SemanticCache` trait + Noop default → Task 3; wired in Task 4. ✓
- L1→embed→L2→rank→write-back data flow → Task 4 (orchestration), Task 8 (lookup), Task 9 (async write). ✓
- Cache-identity isolation (TAG) → Task 3 (`identity_tag`), Task 8 (TAG filter), integration test in Task 8. ✓
- Fail-open + circuit breaker (pre-mortem #1) → Task 6, Task 8, Task 9, Task 11 (boot fallback). ✓
- TTL + eviction (pre-mortem #3) → Task 8/9 (`EXPIRE`); Redis `maxmemory`/`allkeys-lru` documented in Task 12/spec (operator config, not code). ✓
- FLAT COSINE index, LE f32 blob → Task 7, Task 8. ✓
- Metrics (hit/miss/degrade) → Task 10; consumed in Task 8/9. ✓
- Threshold τ default 0.90, TTL 24h, timeout 50ms → Task 5. ✓
- No-regression when off → Task 2 Step 4, Task 4 Step 4, Task 13 Step 1. ✓

**Placeholder scan:** The only intentionally-descriptive step is Task 10 Step 3 ("mirror the existing counter pattern") and Task 11 (grep-then-edit the binary) — both reference concrete existing patterns the engineer reads in-repo, with exact method names/behavior specified. `ensure_index` is a deliberate no-op (documented why: DIM is known only at first insert). No `TODO`/`TBD`/"add error handling" left.

**Type consistency:** `Embedding`/`Embedding::new`/`dim` (Task 1) used identically in Tasks 2/3/4/7/8/9. `SemanticCache::lookup/insert` signatures (Task 3) match all impls (Noop Task 3, Redis Task 8/9) and the ServiceCore call site (Task 4). `identity_tag` output ("c|m|t|x") matches the TAG filter and the test fixtures across Tasks 3/4/8. `CacheConfig` fields (Task 5) match `RedisSemanticCache::connect` and `server.rs` (Tasks 8/11). `record_l2_hit/miss/degraded` (Task 10) match the calls in Tasks 8/9.
