# AC-012 RED evidence — queue/tokenize/forward/total latency visible

## Criterion
AC-012 requires the queue, tokenize, forward, and total service latency to be
independently visible. Per `specs/0.1-mvp/test-plan.md`, AC-012 maps to U-080
(queue/tokenize/forward/total metrics emitted), U-081 (cache hit/miss counters
correct), I-080 (latency decomposition metrics visible), and S-080 (system
evidence distinguishes RTT/queue/forward).

## Proving tests (this slice)
- U-080 `u080_queue_tokenize_forward_total_metrics_emitted`
- U-081 `u081_cache_hit_miss_counters_correct`
- I-080 `i080_latency_decomposition_metrics_visible`

All in `tests/metrics.rs` (integration, plain `#[test]`, offline — the
deterministic pipeline requires no model forward). Each test drives a proposed
metrics registry (`llm_d_sc::metrics::{Metrics, MetricsSnapshot, LatencyStage}`)
that must record per-request queue/tokenize/forward/total latency and cache
hit/miss counters. U-080 asserts all four stages are emitted and each component
is bounded by the total; U-081 asserts the hit/miss counters partition every
request exactly; I-080 binds a real `ClassifyServer`, drives a cache miss then a
cache hit over the persistent channel, and reads `server.metrics_snapshot()`
to assert the latency decomposition and counters are visible.

`specs/0.1-mvp/test-plan.md` maps AC-012 to U-080/U-081 (unit), I-080
(integration), and S-080 (OpenShift system). This slice selects the local
deterministic mechanics proving tests U-080/U-081/I-080; the OpenShift cluster
E2E (S-080) is the deployment phase and is deferred, consistent with how
AC-009/AC-011 deferred the cluster E2E.

## RED state (latency decomposition does not exist)
There is no metrics/latency-decomposition infrastructure anywhere in the crate:
- no `metrics` module (`src/lib.rs` registers only bench/cache/classify/config/
  dummy_praxis/embedding/grpc/queue/ranker/runtime/tokenizer);
- no metrics dependency in `Cargo.toml` (only serde/serde_json/toml/candle/
  tonic/prost/tokio/blake3/tokenizers);
- the pipeline in `src/classify.rs` measures nothing; `BoundedQueue` in
  `src/queue.rs` does not measure queue-wait; `ExactCache`/`SharedCache` in
  `src/cache.rs` expose `forward_count`/`hit_count` but nothing records
  queue/tokenize/forward/total latency; `ClassifyServer` has no
  `metrics_snapshot`.

The proving tests reference `llm_d_sc::metrics` and `server.metrics_snapshot()`,
which are undefined, so they cannot compile.

## Command
```
cargo test --locked --test metrics
```

## Result
FAILED. Expected RED reason: the latency-decomposition metrics registry
(AC-012) does not exist yet, so the proving tests cannot compile, let alone pass.

Failure excerpt:
```
error[E0432]: unresolved import `llm_d_sc::metrics`
  --> tests/metrics.rs:19:15
   |
19 | use llm_d_sc::metrics::{LatencyStage, Metrics, MetricsSnapshot};
   |               ^^^^^^^ could not find `metrics` in `llm_d_sc`

error[E0599]: no method named `metrics_snapshot` found for struct `llm_d_sc::grpc::classify::ClassifyServer` in the current scope
   --> tests/metrics.rs:117:40
    |
117 |     let snap: MetricsSnapshot = server.metrics_snapshot();
    |                                        ^^^^^^^^^^^^^^^^ method not found in `llm_d_sc::grpc::classify::ClassifyServer`

Some errors have detailed explanations: E0432, E0599.
For more information about an error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (test "metrics") due to 2 previous errors
```
Exit code: 101.

## Why this is the expected failure
AC-012 demands that queue/tokenize/forward/total latency be visible. No such
capture exists: the crate has no `metrics` module recording a per-request
latency decomposition or cache hit/miss counters, and `ClassifyServer` exposes
no metrics snapshot. Because `llm_d_sc::metrics` is undefined and
`ClassifyServer::metrics_snapshot` does not exist, `cargo test --locked --test
metrics` fails at exit 101 with "could not find `metrics` in `llm_d_sc`" and
"method not found". This is precisely the expected RED: the feature (latency
decomposition metrics) does not exist, so the proving tests cannot run, let
alone pass. The failure is deterministic and confirms the tests are non-vacuous —
once a `metrics` registry that records per-request queue/tokenize/forward/total
latency and cache hit/miss counters (and a `ClassifyServer::metrics_snapshot`
surface) is implemented, U-080/U-081/I-080 become selectable and must pass.

## Worktree / SHA
- HEAD SHA: `06f342182c2a7ebfa65accd65cbcca9b87c3e35e`
- Working tree: `tests/metrics.rs` added (untracked); `src/` unchanged.
- `git status`:
  ```
   ?? tests/metrics.rs
  ```
  No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-012 -> U-080/U-081, I-080, S-080
- `tests/TEST_MATRIX.md` U-080/U-081/I-080/S-080
- `docs/SDD.md` latency decomposition ("queue, tokenization, model-forward and
  total service latency are independently measured")
- `docs/TDD.md` required evidence includes queue/tokenize/forward/end-to-end
  decomposition
- AC-011 RED precedent: `specs/0.1-mvp/evidence/AC-011/RED.md` (undefined
  harness module -> E0432/E0433 exit 101)
