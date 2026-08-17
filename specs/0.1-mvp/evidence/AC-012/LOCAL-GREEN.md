# AC-012 LOCAL-GREEN evidence — latency decomposition metrics; criterion tests PENDING

## Scope (this file)
The local deterministic metrics mechanics for AC-012 are GREEN: the latency
decomposition registry (`llm_d_sc::metrics`) and the pipeline/server
instrumentation exist and the unit-tier proving tests U-080/U-081 pass locally.
Per the AC-011 precedent, this LOCAL-GREEN does NOT discharge the criterion's
integration/system-tier tests I-080 and S-080, which remain for their tiers.

## Criterion
AC-012 requires the queue, tokenize, forward, and total service latency to be
independently visible. `specs/0.1-mvp/test-plan.md` maps AC-012 to U-080/U-081
(unit), I-080 (integration), and S-080 (OpenShift system).

## Why I-080 / S-080 remain for their tiers
- I-080 (latency decomposition metrics visible over a real gRPC round trip) is
  the INTEGRATION-tier evidence. It is exercised in `tests/metrics.rs` and
  currently passes locally, but it is not discharged by this unit-tier LOCAL-GREEN
  evidence; it is recorded for the integration tier.
- S-080 (system evidence that queue/tokenize/forward/total are distinguished
  independently on OpenShift) is the SYSTEM/cluster E2E and is deferred to the
  deployment phase, consistent with how AC-009/AC-011 deferred the OpenShift
  cluster E2E (S-001/S-002).

## What this evidence proves (local mechanics) — all GREEN
`cargo test --locked --test metrics` — plain `#[test]`, offline (the deterministic
pipeline requires no model forward). Each test drives the real metrics registry:
- `u080_queue_tokenize_forward_total_metrics_emitted` (U-080): all four stages
  emitted strictly positive and each component bounded by the total.
- `u081_cache_hit_miss_counters_correct` (U-081): hit/miss counters counted
  independently and partitioning every request.
- `i080_latency_decomposition_metrics_visible` (I-080, run locally): binds a real
  `ClassifyServer`, drives a cache miss then a cache hit over the persistent
  gRPC channel, and reads `server.metrics_snapshot()` to assert the latency
  decomposition and counters are visible.

## Implementation
Added the missing latency-decomposition metrics (the RED was that they did not
exist):
- `src/metrics.rs` (new): `LatencyStage`, `Metrics` (interior-mutability
  registry, `Clone` shares state), `MetricsSnapshot`.
- `src/classify.rs`: instrumented `ClassifyService` — `deterministic_classify`
  records `Tokenize`/`Forward`; `classify` records `Queue`/`Total` and counts
  each request as a cache hit or miss; added `with_metrics`/
  `from_synthetic_fixtures_with_metrics` constructors.
- `src/grpc/classify.rs`: `ClassifyServer` holds a shared `Metrics` and exposes
  `metrics_snapshot()`.
- `src/lib.rs`: added `pub mod metrics;`.

## Command
```
cargo test --locked --test metrics
```

## Result
PASSED:
```
running 3 tests
test u081_cache_hit_miss_counters_correct ... ok
test u080_queue_tokenize_forward_total_metrics_emitted ... ok
test i080_latency_decomposition_metrics_visible ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```
Exit code 0.

## Gate
`./hack/verify` exit 0 (fmt, clippy `-D warnings`, build, full workspace test
suite). `./hack/spec-check 0.1-mvp` OK. `./hack/test-impact` reported FULL SUITE
(unknown surface `src/metrics.rs`); the full workspace suite passes.

## Worktree / SHA
- HEAD SHA: `e27ccd39a938670d9e9c5858151dd3e5b964b573`
- Working tree (uncommitted): `src/metrics.rs` (new), `src/classify.rs`,
  `src/grpc/classify.rs`, `src/lib.rs` modified; `tests/metrics.rs`,
  `specs/0.1-mvp/evidence/AC-012/` untracked. No commits/pushes.

## Next
I-080 (integration) and S-080 (OpenShift cluster E2E) remain for their tiers.
Do not start AC-013.
