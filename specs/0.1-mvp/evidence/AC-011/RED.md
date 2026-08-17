# AC-011 RED evidence — OpenShift sidecar/ClusterIP RTT distributions captured

## Criterion
AC-011 requires OpenShift sidecar (same-Pod) and ClusterIP RTT DISTRIBUTIONS to
be captured — cache-hit and cache-miss. Per AGENTS.md hard rules, latency
evidence must be percentile distributions (p50/p90/p95/p99/max), never
average-only.

## Proving tests (this slice)
- P-030 `p030_sidecar_cache_hit_rtt_distribution_captured`
- P-031 `p031_sidecar_cache_miss_rtt_distribution_captured`
- P-032 `p032_clusterip_cache_hit_rtt_distribution_captured`
- P-033 `p033_clusterip_cache_miss_rtt_distribution_captured`

All in `tests/bench_rtt.rs` (integration, plain `#[test]`, offline — the
deterministic pipeline requires no model forward). Each test drives a benchmark
harness (`llm_d_sc::bench::{BenchmarkRun, Topology, CacheMode,
RttDistribution}`) for a given topology/cache-mode, warms up, measures a
workload, and asserts a real percentile distribution (p50 <= p90 <= p95 <=
p99 <= max, strictly positive p50) — a mean-only measurement cannot satisfy it.

`specs/0.1-mvp/test-plan.md` maps AC-011 to P-030..P-033 (perf) and S-001/S-002
(OpenShift system). This slice selects the local deterministic mechanics proving
tests P-030..P-033; the OpenShift cluster E2E (S-001/S-002) is the deployment
phase and is deferred, consistent with how AC-009 deferred S-001/S-002.

## RED state (RTT distribution capture does not exist)
There is no RTT distribution capture anywhere in the crate. The only RTT
measurement is a single `Duration` on `DummyOutcome.rtt`
(`src/dummy_praxis.rs:58,90`) — no percentile distribution, no benchmark
harness, and no `bench` module (`src/lib.rs` registers only cache/classify/
config/dummy_praxis/embedding/grpc/queue/ranker/runtime/tokenizer). The proving
tests reference `llm_d_sc::bench`, which is undefined, so they cannot compile.

## Command
```
cargo test --locked --test bench_rtt
```

## Result
FAILED. Expected RED reason: the RTT-distribution benchmark harness (AC-011)
does not exist yet, so the proving tests cannot compile, let alone pass.

Failure excerpt:
```
error[E0432]: unresolved import `llm_d_sc::bench`
  --> tests/bench_rtt.rs:30:15
   |
30 | use llm_d_sc::bench::{BenchmarkRun, CacheMode, Topology};
   |               ^^^^^ could not find `bench` in `llm_d_sc`

error[E0433]: cannot find `bench` in `llm_d_sc`
  --> tests/bench_rtt.rs:37:62
   |
37 | fn assert_distribution_captured(name: &str, dist: &llm_d_sc::bench::RttDistribution) {
   |                                                              ^^^^^ could not find `bench` in `llm_d_sc`

Some errors have detailed explanations: E0432, E0433.
For more information about an error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (test "bench_rtt") due to 2 previous errors
```
Exit code: 101.

## Why this is the expected failure
AC-011 demands that OpenShift sidecar and ClusterIP RTT distributions be
captured. No such capture exists: the crate only records a single scalar RTT
(`DummyOutcome.rtt`) and has no benchmark harness computing percentile
distributions. Because `llm_d_sc::bench` is undefined, `cargo test --locked
--test bench_rtt` fails at exit 101 with "could not find `bench` in
`llm_d_sc`". This is precisely the expected RED: the feature (RTT distribution
capture) does not exist, so the proving tests cannot run, let alone pass. The
failure is deterministic and confirms the tests are non-vacuous — once a
`bench` harness that measures per-request RTT and returns a percentile
distribution (p50/p90/p95/p99/max) is implemented, P-030..P-033 become
selectable and must pass.

## Worktree / SHA
- HEAD SHA: `d3b467cb952818c455f20ba372b2257b868bd08a`
- Working tree: `tests/bench_rtt.rs` added (untracked); `src/` unchanged.
- `git status`:
  ```
   ?? tests/bench_rtt.rs
  ```
  No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-011 -> P-030..P-033, S-001/S-002
- `tests/TEST_MATRIX.md` P-030..P-033
- `tests/HOMELAB.md` benchmark protocol (p50/p90/p95/p99/max, warmup, measured
  workload, 3+ trials)
- AGENTS.md hard rules (no average-only latency claims; percentile evidence)
