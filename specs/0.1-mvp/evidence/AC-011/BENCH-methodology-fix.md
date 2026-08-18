# BENCH-methodology-fix.md — benchmark methodology bug fixed (CONVERGENCE SLICE 4)

## Problem (external review)
The benchmark harness (`llm_d_sc::bench::BenchmarkRun`) had a methodology bug:
`warmup(n)` sent keys `0..n` and `measure(n)` sent the SAME `0..n`. In
`CacheMode::Miss` every measured request whose key was pre-warmed was actually a
cache HIT — the miss benchmark measured hits. The measured cache-miss RTT
distribution was therefore invalid before any cluster measurement ran.

## Fixes implemented (this slice)

### 1. Separate key spaces (per-run namespaces)
- `measure_context(run_id, i)` = `measure-{run_id}-{i}`, the PER-RUN measured
  namespace. `run_id` is unique per `BenchmarkRun` (static `AtomicU64`), so two
  runs never share measured keys even with identical warmup/measure counts.
- `warmup_context`:
  - `CacheMode::Miss` -> `warm-{i}` (a namespace DISJOINT from the measured keys,
    so measured miss keys are never pre-warmed).
  - `CacheMode::Hit` -> EXACTLY the measured key `measure-{run_id}-{i}`, so a
    cache-hit workload's measured requests genuinely hit.
- `src/bench.rs`: `measure_context`, `warmup_context`, `verify_window`.

### 2. The harness PROVES its own methodology
- `BenchmarkRun::with_metrics(addr, topology, cache_mode, metrics)` shares the
  server's `Metrics` handle (the server is bound with
  `ClassifyServer::bind_with_metrics`, added in `src/grpc/classify.rs`).
- `measure` and `measure_concurrent` snapshot the service's
  `cache_hits`/`cache_misses` deltas around the measured window and assert:
  - Miss mode: `delta_misses == measured_count` AND `delta_hits == 0`.
  - Hit mode: `delta_hits == measured_count` AND `delta_misses == 0`.
- A violation returns `BenchError::Methodology` instead of silently producing
  invalid numbers, so a future refactor that silently collides warmup/measured
  keys fails the harness's own assertion.

### 3. Concurrency measurement
- `BenchmarkRun::measure_concurrent(n, concurrency)` distributes `n` requests
  across `concurrency` worker threads, each with its OWN per-worker
  `DummyPraxis` client over the persistent channel (P-020 concurrency 1 /
  P-021 concurrency 4). The `Mutex<DummyPraxis>` serial loop cannot overlap
  requests, so it is replaced with per-worker clients. It records the SAME
  percentile distribution and applies the SAME methodology self-check.

## Evidence

### Unit tests (`cargo test --locked --lib bench`)
```
running 6 tests
test bench::tests::percentile_accessors_are_monotone ... ok
test bench::tests::distinct_runs_use_distinct_measured_namespaces ... ok
test bench::tests::hit_warmup_prewarms_exactly_the_measured_keys ... ok
test bench::tests::miss_methodology_rejects_prewarmed_measured_keys ... ok
test bench::tests::hit_methodology_rejects_unprewarmed_measured_keys ... ok
test bench::tests::miss_warmup_and_measured_keyspaces_are_disjoint ... ok
test result: ok. 6 passed; 0 failed; ...
```
These prove the key-space separation and that the methodology guard REJECTS a
pre-warmed miss key (old bug) and an un-pre-warmed hit key.

### Integration tests (`cargo test --locked --test bench_rtt`)
```
running 13 tests
... all ok ...
test result: ok. 13 passed; 0 failed; ...
```
Covers: 4 serial distribution captures (sidecar/clusterip x hit/miss), the two
direct counter-invariant guards (`miss_measurement_records_all_misses` and
`hit_measurement_records_all_hits`), concurrency 1 and 4 for miss/hit on sidecar
and concurrency 4 for miss/hit on ClusterIP, and the wired-in self-check guard.

### End-to-end proof the old bug is caught (`artifacts/old_bug_proof.rs`, gitignored)
Simulates the original bug (reusing the same measured keys across two windows in
miss mode). The second window sees cached keys and the harness's own self-check
REJECTS it:
```
test old_bug_key_reuse_across_windows_is_rejected ... ok
```
This proves a regression that reintroduces the key-space collision cannot
silently produce invalid benchmark numbers.

## Worktree / SHA
- HEAD: `5d67e52` (harness+docs: close external-review findings on the evidence
  layer) — uncommitted working-tree changes only.
- Files changed:
  - `src/bench.rs` (methodology fix, key namespaces, concurrency)
  - `src/grpc/classify.rs` (added `ClassifyServer::bind_with_metrics`)
  - `tests/bench_rtt.rs` (updated + new methodology/concurrency tests)
- No commits/pushes.

## AC-011 note
This fixes the benchmark METHODOLOGY so the cluster measurements (P-030..P-033,
S-001/S-002) — which remain PENDING — measure genuine cache misses and genuine
cache hits. It does not discharge the cluster-tier criterion tests.
