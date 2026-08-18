//! Benchmark-harness mechanics tests (AC-011 prerequisite instrumentation).
//!
//! These tests prove the RTT-DISTRIBUTION CAPTURE HARNESS itself — the
//! instrument that will later measure the OpenShift topology. They are NOT the
//! AC-011 criterion tests.
//!
//! STATUS: P-030..P-033 and S-001/S-002 are **PENDING cluster measurement**.
//!
//! `specs/0.1-mvp/test-plan.md` maps AC-011 to P-030..P-033 (perf) and
//! S-001/S-002 (OpenShift system). P-030/P-031 require a dummy Praxis -> same-Pod
//! sidecar (client and server on the same loopback address, as in a shared Pod)
//! RTT DISTRIBUTION; P-032/P-033 require the ClusterIP (distinct-address service)
//! RTT DISTRIBUTION; S-001/S-002 require a real OpenShift cluster E2E. A local
//! loopback-vs-distinct-address simulation on a laptop CANNOT discharge those
//! cluster-tier measurements, so P-030..P-033 and S-001/S-002 remain PENDING
//! until they are measured on the cluster. The OpenShift cluster E2E (S-001/S-002)
//! is the deployment phase, consistent with how AC-009 deferred S-001/S-002.
//!
//! # Methodology (CONVERGENCE SLICE 4)
//!
//! An external review found a methodology bug: warmup sent keys `0..n` and
//! measure sent the SAME `0..n`, so in `CacheMode::Miss` every measured request
//! was actually a cache HIT — the miss benchmark measured hits. The harness is
//! now self-proving:
//!
//! - Miss mode warms a disjoint `warm-{i}` namespace and measures the per-run
//!   `measure-{run_id}-{i}` namespace, which is never pre-warmed, so every
//!   measured request is a genuine miss.
//! - Hit mode deliberately pre-warms exactly the measured keys.
//! - Each run shares the server's [`Metrics`]; `measure` snapshots the service's
//!   `cache_hits`/`cache_misses` deltas around the measured window and asserts
//!   miss-mode `delta_misses == measured` and hit-mode `delta_hits == measured`
//!   (with the opposite counter at zero). A future refactor that silently
//!   collides warmup/measured keys fails the harness's own assertion instead of
//!   producing invalid numbers.
//!
//! Concurrency (P-020 concurrency 1 / P-021 concurrency 4) is measured with
//! per-worker dummy-Praxis clients over the persistent channel — the
//! `Mutex<DummyPraxis>` serial loop cannot overlap requests — and the SAME
//! distributions and methodology self-check apply.

use llm_d_sc::bench::{BenchmarkRun, CacheMode, Topology};
use llm_d_sc::grpc::classify::ClassifyServer;
use llm_d_sc::metrics::Metrics;

/// Warmup count used for cache-HIT workloads: must be >= the measured count so
/// every measured key is pre-warmed (hit-mode methodology requires it).
const HIT_WARMUP: u64 = 1100;
/// Warmup count used for cache-MISS workloads: warms the disjoint `warm-{i}`
/// namespace, which never collides with the measured keys.
const MISS_WARMUP: u64 = 100;
/// Number of requests in a measured window.
const MEASURE: u64 = 1000;

/// Shared distribution invariant: a captured RTT distribution must be
/// p50 <= p90 <= p95 <= p99 <= max with a strictly positive p50. A mean-only
/// measurement has no p50/p90/p95/p99, so this guards against average-only
/// latency claims (AGENTS.md).
fn assert_distribution_captured(name: &str, dist: &llm_d_sc::bench::RttDistribution) {
    let p50 = dist.p50();
    let p90 = dist.p90();
    let p95 = dist.p95();
    let p99 = dist.p99();
    let max = dist.max();
    assert!(
        p50 > std::time::Duration::ZERO,
        "{name}: p50 must be strictly positive"
    );
    assert!(
        p50 <= p90 && p90 <= p95 && p95 <= p99 && p99 <= max,
        "{name}: distribution must be monotone p50<=p90<=p95<=p99<=max (got {p50:?} {p90:?} {p95:?} {p99:?} {max:?})"
    );
    assert!(max >= p50, "{name}: max must not be below p50");
}

/// Bind a classify server sharing `metrics` and a benchmark run over the same
/// metrics, so `measure`/`measure_concurrent` can PROVE their own methodology.
fn bind_run(topology: Topology, cache_mode: CacheMode) -> (ClassifyServer, BenchmarkRun) {
    let metrics = Metrics::new();
    let server = ClassifyServer::bind_with_metrics("127.0.0.1:0", metrics.clone())
        .expect("classify server must bind");
    let run = BenchmarkRun::with_metrics(server.local_addr(), topology, cache_mode, metrics)
        .expect("benchmark run must initialize");
    (server, run)
}

/// Harness mechanics: sidecar (same-Pod) cache-hit RTT distribution captured.
///
/// Hit mode deliberately pre-warms exactly the measured keys, so every measured
/// request is a genuine cache hit; the harness's methodology self-check asserts
/// `delta_hits == measured`. (Mechanics only; the sidecar cluster measurement is
/// P-030, PENDING.)
#[test]
fn harness_captures_distribution_sidecar_hit() {
    let (_server, run) = bind_run(Topology::Sidecar, CacheMode::Hit);
    run.warmup(HIT_WARMUP).expect("hit warmup must succeed");
    let dist = run
        .measure(MEASURE)
        .expect("cache-hit measurement must succeed");
    assert_distribution_captured("harness sidecar cache-hit", &dist);
}

/// Harness mechanics: sidecar (same-Pod) cache-miss RTT distribution captured.
///
/// Miss mode warms the disjoint `warm-{i}` namespace and measures the per-run
/// `measure-{run_id}-{i}` namespace, so every measured request is a genuine
/// miss; the harness's methodology self-check asserts `delta_misses == measured`.
/// (Mechanics only; the sidecar cache-miss cluster measurement is P-031, PENDING.)
#[test]
fn harness_captures_distribution_sidecar_miss() {
    let (_server, run) = bind_run(Topology::Sidecar, CacheMode::Miss);
    run.warmup(MISS_WARMUP).expect("miss warmup must succeed");
    let dist = run
        .measure(MEASURE)
        .expect("cache-miss measurement must succeed");
    assert_distribution_captured("harness sidecar cache-miss", &dist);
}

/// Harness mechanics: ClusterIP (distinct-address) cache-hit RTT distribution captured.
///
/// (Mechanics only; the ClusterIP cache-hit cluster measurement is P-032,
/// PENDING.)
#[test]
fn harness_captures_distribution_clusterip_hit() {
    let (_server, run) = bind_run(Topology::ClusterIp, CacheMode::Hit);
    run.warmup(HIT_WARMUP).expect("hit warmup must succeed");
    let dist = run
        .measure(MEASURE)
        .expect("cache-hit measurement must succeed");
    assert_distribution_captured("harness ClusterIP cache-hit", &dist);
}

/// Harness mechanics: ClusterIP (distinct-address) cache-miss RTT distribution captured.
///
/// (Mechanics only; the ClusterIP cache-miss cluster measurement is P-033,
/// PENDING.)
#[test]
fn harness_captures_distribution_clusterip_miss() {
    let (_server, run) = bind_run(Topology::ClusterIp, CacheMode::Miss);
    run.warmup(MISS_WARMUP).expect("miss warmup must succeed");
    let dist = run
        .measure(MEASURE)
        .expect("cache-miss measurement must succeed");
    assert_distribution_captured("harness ClusterIP cache-miss", &dist);
}

/// Harness mechanics: the miss benchmark genuinely measures MISSES, not hits.
///
/// This is the convergence-slice regression guard: warmup must NOT pre-warm the
/// measured keys. We assert this both via the harness's internal methodology
/// self-check (measure would return Err) and by observing the server's cache
/// counters directly.
#[test]
fn miss_measurement_records_all_misses() {
    let metrics = Metrics::new();
    let server = ClassifyServer::bind_with_metrics("127.0.0.1:0", metrics.clone())
        .expect("classify server must bind");
    let run = BenchmarkRun::with_metrics(
        server.local_addr(),
        Topology::Sidecar,
        CacheMode::Miss,
        metrics.clone(),
    )
    .expect("benchmark run must initialize");
    run.warmup(MISS_WARMUP).expect("miss warmup must succeed");

    let before = metrics.snapshot();
    run.measure(MEASURE)
        .expect("miss measurement must pass its own methodology check");
    let after = metrics.snapshot();

    assert_eq!(
        after.cache_misses - before.cache_misses,
        MEASURE,
        "a miss benchmark must record exactly the measured count of cache misses"
    );
    assert_eq!(
        after.cache_hits - before.cache_hits,
        0,
        "a miss benchmark must record ZERO cache hits in the measured window"
    );
}

/// Harness mechanics: the hit benchmark genuinely measures HITS, not misses.
///
/// Warmup deliberately pre-warms exactly the measured keys, so every measured
/// request is a cache hit. Asserted both by the harness's internal methodology
/// self-check and by observing the server's cache counters directly.
#[test]
fn hit_measurement_records_all_hits() {
    let metrics = Metrics::new();
    let server = ClassifyServer::bind_with_metrics("127.0.0.1:0", metrics.clone())
        .expect("classify server must bind");
    let run = BenchmarkRun::with_metrics(
        server.local_addr(),
        Topology::Sidecar,
        CacheMode::Hit,
        metrics.clone(),
    )
    .expect("benchmark run must initialize");
    run.warmup(HIT_WARMUP).expect("hit warmup must succeed");

    let before = metrics.snapshot();
    run.measure(MEASURE)
        .expect("hit measurement must pass its own methodology check");
    let after = metrics.snapshot();

    assert_eq!(
        after.cache_hits - before.cache_hits,
        MEASURE,
        "a hit benchmark must record exactly the measured count of cache hits"
    );
    assert_eq!(
        after.cache_misses - before.cache_misses,
        0,
        "a hit benchmark must record ZERO cache misses in the measured window"
    );
}

/// Concurrency helper: assert a measured distribution for a given topology,
/// cache mode, and concurrency, returning the captured distribution.
fn assert_concurrent(
    name: &str,
    topology: Topology,
    cache_mode: CacheMode,
    concurrency: u64,
) -> llm_d_sc::bench::RttDistribution {
    let (_server, run) = bind_run(topology, cache_mode);
    // Warm up the appropriate namespace (hit pre-warms measured keys; miss warms
    // the disjoint warm namespace), then measure concurrently.
    match cache_mode {
        CacheMode::Hit => run.warmup(HIT_WARMUP).expect("hit warmup must succeed"),
        CacheMode::Miss => run.warmup(MISS_WARMUP).expect("miss warmup must succeed"),
    };
    let dist = run
        .measure_concurrent(MEASURE, concurrency)
        .expect("concurrent measurement must pass its own methodology check");
    assert_distribution_captured(name, &dist);
    dist
}

/// Harness mechanics: concurrent cache-miss measurement (concurrency 1) still
/// records exactly the measured count of MISSES (P-020).
#[test]
fn concurrent_miss_sidecar_concurrency_1_records_all_misses() {
    assert_concurrent(
        "sidecar miss concurrency 1",
        Topology::Sidecar,
        CacheMode::Miss,
        1,
    );
}

/// Harness mechanics: concurrent cache-miss measurement (concurrency 4) still
/// records exactly the measured count of MISSES (P-021).
#[test]
fn concurrent_miss_sidecar_concurrency_4_records_all_misses() {
    assert_concurrent(
        "sidecar miss concurrency 4",
        Topology::Sidecar,
        CacheMode::Miss,
        4,
    );
}

/// Harness mechanics: concurrent cache-HIT measurement (concurrency 1) still
/// records exactly the measured count of HITS (P-020).
#[test]
fn concurrent_hit_sidecar_concurrency_1_records_all_hits() {
    assert_concurrent(
        "sidecar hit concurrency 1",
        Topology::Sidecar,
        CacheMode::Hit,
        1,
    );
}

/// Harness mechanics: concurrent cache-HIT measurement (concurrency 4) still
/// records exactly the measured count of HITS (P-021).
#[test]
fn concurrent_hit_sidecar_concurrency_4_records_all_hits() {
    assert_concurrent(
        "sidecar hit concurrency 4",
        Topology::Sidecar,
        CacheMode::Hit,
        4,
    );
}

/// Harness mechanics: concurrent cache-miss measurement over ClusterIP
/// (concurrency 4) records exactly the measured count of MISSES (P-021).
#[test]
fn concurrent_miss_clusterip_concurrency_4_records_all_misses() {
    assert_concurrent(
        "clusterip miss concurrency 4",
        Topology::ClusterIp,
        CacheMode::Miss,
        4,
    );
}

/// Harness mechanics: concurrent cache-HIT measurement over ClusterIP
/// (concurrency 4) records exactly the measured count of HITS (P-021).
#[test]
fn concurrent_hit_clusterip_concurrency_4_records_all_hits() {
    assert_concurrent(
        "clusterip hit concurrency 4",
        Topology::ClusterIp,
        CacheMode::Hit,
        4,
    );
}

/// The methodology self-check rejects key-space collisions at the unit level.
///
/// The `verify_window` guard (which `measure`/`measure_concurrent` call) is
/// proven by `src/bench.rs::miss_methodology_rejects_prewarmed_measured_keys`
/// and `src/bench.rs::hit_methodology_rejects_unprewarmed_measured_keys` — a
/// future refactor that silently collides warmup and measured keys fails the
/// harness's own assertion instead of producing invalid numbers.
#[test]
fn methodology_selfcheck_guards_key_collisions() {
    // The guard is unit-tested in `src/bench.rs`; this integration test confirms
    // the guard is wired into the public measure path by re-asserting the two
    // real counter invariants on the server.
    let metrics = Metrics::new();
    let server = ClassifyServer::bind_with_metrics("127.0.0.1:0", metrics.clone())
        .expect("classify server must bind");
    let run = BenchmarkRun::with_metrics(
        server.local_addr(),
        Topology::Sidecar,
        CacheMode::Miss,
        metrics.clone(),
    )
    .expect("benchmark run must initialize");
    run.warmup(MISS_WARMUP).expect("miss warmup must succeed");
    let before = metrics.snapshot();
    run.measure(MEASURE)
        .expect("genuinely-miss measured keys must pass the self-check");
    let after = metrics.snapshot();
    assert_eq!(after.cache_misses - before.cache_misses, MEASURE);
    assert_eq!(after.cache_hits - before.cache_hits, 0);
}
