# AC-011 LOCAL-GREEN evidence — benchmark harness complete; criterion tests PENDING

## Scope (this file)
Harness complete: the `llm_d_sc::bench` RTT-DISTRIBUTION capture instrument exists
and its mechanics are proven locally. The criterion's OWN tests (P-030..P-033,
S-001/S-002) are **PENDING cluster measurement** and are NOT discharged by this
evidence.

## Criterion
AC-011 requires OpenShift sidecar (same-Pod) and ClusterIP RTT DISTRIBUTIONS to
be captured, cache-hit and cache-miss. Per AGENTS.md hard rules, latency
evidence must be percentile distributions (p50/p90/p95/p99/max), never
average-only.

## Why the criterion's tests remain PENDING
`specs/0.1-mvp/test-plan.md` maps AC-011 to P-030..P-033 (perf) and S-001/S-002
(OpenShift system):
- P-030/P-031: dummy Praxis -> same-Pod sidecar RTT distribution (hit/miss)
- P-032/P-033: dummy Praxis -> ClusterIP RTT distribution (hit/miss)
- S-001/S-002: OpenShift cluster E2E

P-030..P-033 are CLUSTER measurements (same-Pod / ClusterIP RTT on OpenShift). A
loopback-vs-distinct-address simulation on a laptop cannot discharge them;
claiming otherwise would make the perf tier unfalsifiable. So P-030..P-033 and
S-001/S-002 remain PENDING until measured on the cluster (deployment phase,
consistent with AC-009's deferral of S-001/S-002).

## What this evidence proves (harness mechanics) — all GREEN
The harness-mechanics tests in `tests/bench_rtt.rs` (plain `#[test]`, offline —
the deterministic pipeline requires no model forward). Each binds a
`ClassifyServer`, drives the benchmark harness, warms up, measures a
1000-request workload, and asserts a real percentile distribution
(p50 <= p90 <= p95 <= p99 <= max, strictly positive p50):
- `harness_captures_distribution_sidecar_hit`
- `harness_captures_distribution_sidecar_miss`
- `harness_captures_distribution_clusterip_hit`
- `harness_captures_distribution_clusterip_miss`

## Implementation
Added the missing RTT-distribution benchmark harness (the RED was that it did
not exist):
- `src/bench.rs` (new): `BenchmarkRun` / `Topology` (Sidecar/ClusterIp) /
  `CacheMode` (Hit/Miss) / `RttDistribution` (p50/p90/p95/p99/max via
  nearest-rank percentile over sorted per-request RTT samples). Drives the
  dummy Praxis over the persistent gRPC channel (I-008), never a route (AC-010).
  `CacheMode::Hit` reuses one fixed context (exact-result cache hit);
  `CacheMode::Miss` sends a unique context per request (cache miss).
- `src/lib.rs`: added `pub mod bench;`.

## Command
```
cargo test --locked --test bench_rtt
```

## Result
PASSED. All 4 harness-mechanics tests pass:
```
running 4 tests
test harness_captures_distribution_sidecar_hit ... ok
test harness_captures_distribution_clusterip_hit ... ok
test harness_captures_distribution_sidecar_miss ... ok
test harness_captures_distribution_clusterip_miss ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;
finished in 0.52s
```
Exit code 0.

## Why this proves the harness mechanics
Each scenario returns a real percentile distribution, never an average:
`assert_distribution_captured` checks p50 > 0 and p50 <= p90 <= p95 <= p99 <=
max. The `RttDistribution` accessors are nearest-rank percentiles over the
sorted measured per-request RTTs, so the monotonic invariant holds
deterministically and a mean-only harness cannot satisfy the tests.

## Worktree / SHA
- HEAD SHA: `d3b467cb952818c455f20ba372b2257b868bd08a`
- Working tree changes (uncommitted): `tests/bench_rtt.rs` (untracked),
  `src/bench.rs` (new), `src/lib.rs` (modified). No commits/pushes.

## Next suites
- `./hack/test-impact <changed files>`
- `./hack/spec-check 0.1-mvp`
- required local suite via `./hack/verify`
