# AC-006 RED evidence — SERVICE-CORE (generic core for production cache)

## Criterion / slice
P0 corrective slice: put the real Candle path behind the SAME cache/metrics core
as the synthetic one. The generic `ServiceCore<R>` struct `{ runtime, cache:
SharedCache, metrics: Metrics }` must be used by BOTH the synthetic and Candle
backends so every backend inherits caching, single-flight coalescing, metrics
and error behaviour. A production (Candle) cache hit must perform ZERO tokenizer
calls and ZERO model forwards.

## Proving test (parity tier, `#[ignore]`)
`src/classify.rs::service_core_production_candle_cache_hit_zero_tokenizer_zero_forward`:
builds a real `CandleClassifier` from the fetched sensitivity model, instruments
counters on the runtime (tokenizer calls / model forwards), wraps it in the
generic `ServiceCore`, and asserts:
- miss -> exactly 1 tokenizer call + 1 forward;
- identical hit -> ZERO new tokenizer calls + ZERO new forwards, exact cached result;
- distinct input -> fresh miss (counters increment to 2).

## Command
```
cargo test --locked --lib -- --ignored service_core_production_candle_cache_hit_zero_tokenizer_zero_forward
```

## Worktree
SHA `1bd6596` (uncommitted). No commits/pushes.

## Failure excerpt
```
error[E0599]: no method named `tokenizer_call_counter` found for struct `classify::CandleClassifier`
error[E0599]: no method named `forward_call_counter` found for struct `classify::CandleClassifier`
error[E0433]: cannot find type `ServiceCore` in this scope
error: could not compile `llm-d-sc` (lib test) due to 3 previous errors
```

## Why this is the expected failure
The generic service core does not exist yet: `ServiceCore<R>` is undeclared, and
`CandleClassifier` has no instrumented tokenizer/forward counters. Today the
cache/single-flight/metrics logic is duplicated inside each backend's own
`classify` (synthetic `ClassifyService` and `CandleClassifier`), so there is no
single core that guarantees a production cache hit bypasses the real Candle
tokenizer + model forward. The test cannot even compile, hence RED for the
expected reason.
