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
//! What these tests DO prove is the harness mechanics: the `llm_d_sc::bench`
//! harness must capture a real percentile distribution — p50/p90/p95/p99/max,
//! never average-only. AGENTS.md hard rules: "no average-only latency claims" and
//! "no performance claim without comparable before/after p50/p95/p99 evidence".
//! So every mechanics test asserts a real percentile distribution
//! (p50 <= p90 <= p95 <= p99 <= max, all strictly positive), which a mean-only
//! measurement cannot satisfy.
//!
//! The harness (`llm_d_sc::bench`) measures per-request RTT over the persistent
//! gRPC channel for a given topology label (sidecar = same-address loopback;
//! ClusterIP = distinct-address) and cache mode (hit = warmed exact-result cache;
//! miss = unique context per request). The pipeline is the deterministic
//! tokenizer -> versioned cache -> single-flight -> ranker path (no Candle
//! forward required), consistent with prior slices.
//!
//! RED: the `llm_d_sc::bench` harness does not exist yet — there is no RTT
//! distribution capture anywhere in the crate (only the single `Duration` RTT on
//! `DummyOutcome`), so these tests cannot compile. That missing capture
//! infrastructure is the expected RED for the harness slice.

use llm_d_sc::bench::{BenchmarkRun, CacheMode, Topology};
use llm_d_sc::grpc::classify::ClassifyServer;

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

/// Harness mechanics: sidecar (same-Pod) cache-hit RTT distribution captured.
///
/// The harness must capture the sidecar-topology RTT DISTRIBUTION for a cache
/// hit: warms the exact-result cache, measures a workload of cache-hit requests,
/// and must return a percentile distribution — never an average. (Mechanics
/// only; the sidecar cluster measurement is P-030, PENDING.)
#[test]
fn harness_captures_distribution_sidecar_hit() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("sidecar classify server must bind");
    let run = BenchmarkRun::new(server.local_addr(), Topology::Sidecar, CacheMode::Hit)
        .expect("benchmark run must initialize");
    run.warmup(100).expect("warmup must succeed");
    let dist = run
        .measure(1000)
        .expect("cache-hit measurement must succeed");

    assert_distribution_captured("harness sidecar cache-hit", &dist);
}

/// Harness mechanics: sidecar (same-Pod) cache-miss RTT distribution captured.
///
/// The harness must capture the sidecar RTT DISTRIBUTION for a cache miss
/// (unique context per request, so the exact-result cache never hits). It
/// measures a cache-miss workload and must return a percentile distribution.
/// (Mechanics only; the sidecar cache-miss cluster measurement is P-031, PENDING.)
#[test]
fn harness_captures_distribution_sidecar_miss() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("sidecar classify server must bind");
    let run = BenchmarkRun::new(server.local_addr(), Topology::Sidecar, CacheMode::Miss)
        .expect("benchmark run must initialize");
    run.warmup(100).expect("warmup must succeed");
    let dist = run
        .measure(1000)
        .expect("cache-miss measurement must succeed");

    assert_distribution_captured("harness sidecar cache-miss", &dist);
}

/// Harness mechanics: ClusterIP (distinct-address) cache-hit RTT distribution captured.
///
/// The harness must capture the ClusterIP-topology RTT DISTRIBUTION for a cache
/// hit. (Mechanics only; the ClusterIP cache-hit cluster measurement is P-032,
/// PENDING.)
#[test]
fn harness_captures_distribution_clusterip_hit() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("clusterip classify server must bind");
    let run = BenchmarkRun::new(server.local_addr(), Topology::ClusterIp, CacheMode::Hit)
        .expect("benchmark run must initialize");
    run.warmup(100).expect("warmup must succeed");
    let dist = run
        .measure(1000)
        .expect("cache-hit measurement must succeed");

    assert_distribution_captured("harness ClusterIP cache-hit", &dist);
}

/// Harness mechanics: ClusterIP (distinct-address) cache-miss RTT distribution captured.
///
/// The harness must capture the ClusterIP RTT DISTRIBUTION for a cache miss.
/// (Mechanics only; the ClusterIP cache-miss cluster measurement is P-033,
/// PENDING.)
#[test]
fn harness_captures_distribution_clusterip_miss() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("clusterip classify server must bind");
    let run = BenchmarkRun::new(server.local_addr(), Topology::ClusterIp, CacheMode::Miss)
        .expect("benchmark run must initialize");
    run.warmup(100).expect("warmup must succeed");
    let dist = run
        .measure(1000)
        .expect("cache-miss measurement must succeed");

    assert_distribution_captured("harness ClusterIP cache-miss", &dist);
}
