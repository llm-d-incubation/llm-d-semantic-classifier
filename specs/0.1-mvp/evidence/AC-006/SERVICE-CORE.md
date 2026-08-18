# SERVICE-CORE: shared cache/metrics core for every backend (synthetic + Candle)

## Decision
Introduce a single generic service core that OWNS the exact-result cache,
single-flight coalescing, and cache hit/miss/total/queue metrics, and wrap EVERY
backend (synthetic `ClassifyService` AND production `CandleClassifier`) in it.
Backends are raw forwards: tokenize + rank, nothing more.

## Motivation
Previously the Candle path carried its OWN cache/metrics inside
`CandleClassifier` while the synthetic path had them in `ClassifyService` — two
parallel implementations of the same AC-006/AC-007/AC-008 behaviour, with no
guarantee the production path matched. This slice makes the cache pipeline live
in ONE place, `ServiceCore<R>`, so the production Candle path provably inherits
exact-result caching and single-flight coalescing.

## Design

```rust
pub struct ServiceCore<R> {
    runtime: R,
    cache: SharedCache,
    metrics: Metrics,
}

impl<R> ServiceCore<R> {
    pub fn with_metrics(runtime: R, metrics: Metrics) -> Self;
    pub fn metrics(&self) -> Metrics;
    pub fn forward_count(&self) -> u64;
}

impl<R> ClassifierRuntime for ServiceCore<R> { /* classify: cache-first */ }
```

- `classify` computes `CacheKey::new(CLASSIFIER_ID, MODEL_REVISION,
  TOKENIZER_REVISION, TAXONOMY_REVISION, &normalized)` then
  `self.cache.classify_concurrent(key, forward)`. On miss the raw `runtime`
  classifies; on hit the cached exact result is returned with ZERO runtime calls.
- Queue stage is recorded via an `Arc<AtomicBool>` flag: a cache hit (no forward)
  still records the Queue/Total latency stages but skips tokenize/forward stage
  recording.
- `forward_count` delegates to `self.cache.forward_count()` (single-flight
  count), NOT the runtime's forward counter — the cache owns forward accounting.

## Backend contract
- `CandleClassifier` (production): removed `cache` field. Added
  `tokenizer_calls`/`forward_calls` as `Arc<AtomicU64>` with
  `tokenizer_call_counter()`/`forward_call_counter()` accessors. `real_forward`
  now takes only `&str` (uses `self.metrics`); `classify()` is a raw forward.
  Constructors: `new(embedder, prototypes)`, `with_metrics(...)`.
- `ClassifyService` (synthetic): removed `cache` field. `deterministic_classify`
  uses `self.metrics` directly; `classify()` is a raw deterministic forward.
- Neither backend does any caching or single-flight — that lives only in
  `ServiceCore`.

## Wiring
- `ClassifyServiceImpl<R>::with_executor` wraps `service` (the raw backend) in
  `ServiceCore::with_metrics(service, metrics.clone())` and spawns the executor
  over the core. The executor field is `Arc<InferenceExecutor<ServiceCore<R>>>`.
- All server bind functions (`bind`, `bind_with_metrics`, `bind_with_classifier`)
  share ONE `Metrics` handle across the backend, the core, and the executor, so
  cache counters, queue-wait, and tokenize/forward stage recordings land in the
  same registry the server snapshots.

## Metrics ownership
- Cache hit/miss counters, total/queue stage: recorded by the core.
- Tokenize/forward stage: recorded by the backend via its shared `Metrics` handle.
- Single-flight forward accounting: owned by `SharedCache`.

## Proof
- Parity test `service_core_production_candle_cache_hit_zero_tokenizer_zero_forward`
  (GREEN): a Candle cache hit performs ZERO tokenizer calls and ZERO forwards.
- Existing AC-006/AC-007 tests (u070/u071, cache u040..u044, metrics u080/u081,
  bench_rtt/metrics/grpc integration) all GREEN through the core.
- See `RED-SERVICE-CORE.md` / `GREEN-SERVICE-CORE.md`.
