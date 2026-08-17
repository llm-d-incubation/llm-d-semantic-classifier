# AC-012 GREEN evidence — U-080 queue/tokenize/forward/total metrics emitted

## Criterion
AC-012 requires the queue, tokenize, forward, and total service latency to be
independently visible. U-080 (from `specs/0.1-mvp/test-plan.md` -> AC-012) asserts
that the metrics registry emits all four latency stages independently (strictly
positive on a cache miss) and that each component is bounded by the total.

## Proving test
`u080_queue_tokenize_forward_total_metrics_emitted` in `tests/metrics.rs`.

## Implementation (smallest change)
- `src/metrics.rs` (new): `LatencyStage` (`Queue`/`Tokenize`/`Forward`/`Total`),
  `Metrics` (interior-mutability registry via `Arc<Mutex<Inner>>`, `Clone` shares
  the same state), and `MetricsSnapshot` (`queue`/`tokenize`/`forward`/`total` +
  `cache_hits`/`cache_misses`).
- `src/classify.rs`: instrumented `ClassifyService` — `deterministic_classify`
  records `Tokenize` and `Forward` stages; `classify` records `Queue` (ends when
  forward begins on a miss / when the cached result is served on a hit) and
  `Total` (admission -> response), and counts each request as a cache hit or miss.
- `src/grpc/classify.rs`: `ClassifyServer` holds a shared `Metrics` and exposes
  `metrics_snapshot()`.
- `src/lib.rs`: added `pub mod metrics;`.

## Command
```
cargo test --locked --test metrics -- u080_queue_tokenize_forward_total_metrics_emitted
```

## Result
PASSED.

## Why this proves U-080
The registry records the four stages independently and the snapshot exposes all
four; the test asserts each is strictly positive and each component
(queue/tokenize/forward) is bounded by the total. A registry that omits any
stage or records only an aggregate cannot satisfy these assertions.

## Worktree / SHA
- HEAD SHA: `e27ccd39a938670d9e9c5858151dd3e5b964b573`
- Working tree (uncommitted): `src/metrics.rs` (new), `src/classify.rs`,
  `src/grpc/classify.rs`, `src/lib.rs` modified; `tests/metrics.rs` untracked.
  No commits/pushes.
