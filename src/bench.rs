//! Benchmark harness: captures per-request RTT DISTRIBUTIONS for the dummy
//! Praxis -> llm-d-sc path, never average-only latency.
//!
//! AC-011 requires OpenShift sidecar (same-Pod) and ClusterIP RTT distributions
//! to be captured, for cache-hit and cache-miss workloads. AGENTS.md hard rules
//! forbid average-only latency claims and require p50/p95/p99 percentile
//! evidence. This harness therefore measures a workload of per-request RTTs over
//! the persistent gRPC channel and reduces them to a percentile distribution
//! (p50/p90/p95/p99/max), which a mean-only measurement cannot produce.
//!
//! The harness drives the deterministic pipeline (tokenizer -> versioned cache
//! -> single-flight -> ranker over synthetic prototypes, no Candle forward),
//! consistent with prior slices. [`Topology`] labels the network path (sidecar =
//! same-address loopback; ClusterIP = distinct-address service) and [`CacheMode`]
//! selects a cache-hit (warmed exact-result cache) or cache-miss (unique context
//! per request) workload. Routing/session authority stays with the dummy Praxis
//! (AC-010); this module only measures RTT, never a route.
//!
//! # Methodology (this slice)
//!
//! The cache-mode workloads are keyed by a distinct, PER-RUN NAMESPACE so that a
//! measured request can never be silently served from the cache when it is meant
//! to be a miss, and vice-versa:
//!
//! - [`CacheMode::Miss`]: warmup keys live in the `warm-{i}` namespace, which is
//!   NEVER measured, and measured keys live in the `measure-{run_id}-{i}`
//!   namespace, which is NEVER pre-warmed. Every measured request is therefore a
//!   genuine cache miss.
//! - [`CacheMode::Hit`]: warmup deliberately pre-warms EXACTLY the measured keys
//!   (`measure-{run_id}-{i}`), so every measured request is a genuine cache hit
//!   (the caller must warm at least as many keys as it measures).
//!
//! `run_id` is unique per [`BenchmarkRun`], so two runs never share measured
//! keys even with identical warmup/measure counts.
//!
//! When a [`Metrics`] handle is supplied (the same one the server records into),
//! the harness PROVES ITS OWN METHODOLOGY: it snapshots the service's
//! `cache_hits`/`cache_misses` deltas around the measured window and asserts
//! `miss-mode delta_misses == measured_count` and `hit-mode delta_hits ==
//! measured_count` (with the opposite counter at zero). A future refactor that
//! silently collides warmup and measured keys fails the harness's own assertion
//! instead of producing invalid numbers.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::dummy_praxis::{DummyPraxis, DummyRequest};
use crate::metrics::{Metrics, MetricsSnapshot};

/// Unique-per-run counter so two benchmark runs never share measured keys.
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);

/// The network topology under test. Both connect over the persistent gRPC
/// channel to the supplied address; the variant is recorded so the captured
/// distribution is attributable to the intended OpenShift path (sidecar =
/// same-Pod loopback; ClusterIP = distinct-address service).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    /// same-Pod: dummy Praxis and llm-d-sc on the same loopback address.
    Sidecar,
    /// ClusterIP: dummy Praxis reaches llm-d-sc via a distinct-address service.
    ClusterIp,
}

/// The cache workload to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Exact-result cache hit: the measured keys are pre-warmed so the cache hits.
    Hit,
    /// Cache miss: every measured request carries a unique, never-pre-warmed
    /// key, so the cache never hits.
    Miss,
}

/// A captured percentile distribution of per-request RTTs.
///
/// Reduces a measured workload to p50/p90/p95/p99/max. The accessors are
/// monotone (p50 <= p90 <= p95 <= p99 <= max), satisfying the AC-011 and
/// AGENTS.md requirement for percentile latency evidence (never average-only).
pub struct RttDistribution {
    samples: Vec<std::time::Duration>,
}

impl RttDistribution {
    /// Build a distribution from an unsorted set of per-request RTT samples.
    pub fn from_samples(mut samples: Vec<std::time::Duration>) -> Self {
        samples.sort();
        RttDistribution { samples }
    }

    /// The 50th percentile RTT (nearest-rank).
    pub fn p50(&self) -> std::time::Duration {
        percentile(&self.samples, 50)
    }

    /// The 90th percentile RTT (nearest-rank).
    pub fn p90(&self) -> std::time::Duration {
        percentile(&self.samples, 90)
    }

    /// The 95th percentile RTT (nearest-rank).
    pub fn p95(&self) -> std::time::Duration {
        percentile(&self.samples, 95)
    }

    /// The 99th percentile RTT (nearest-rank).
    pub fn p99(&self) -> std::time::Duration {
        percentile(&self.samples, 99)
    }

    /// The maximum observed RTT.
    pub fn max(&self) -> std::time::Duration {
        self.samples.last().copied().unwrap_or_default()
    }
}

/// The nearest-rank percentile of a sorted (ascending) sample set.
fn percentile(sorted: &[std::time::Duration], p: u64) -> std::time::Duration {
    if sorted.is_empty() {
        return std::time::Duration::ZERO;
    }
    let rank = ((p as f64 / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

/// The measured-key context for index `index` under `run_id`.
///
/// Lives in the per-run `measure-{run_id}-{i}` namespace, which is NEVER
/// pre-warmed in [`CacheMode::Miss`] and EXACTLY pre-warmed in
/// [`CacheMode::Hit`]. Distinct per run and per index, so a measured request is
/// never a silent cache hit when it should be a miss.
fn measure_context(run_id: u64, index: u64) -> String {
    format!("measure-{run_id}-{index}")
}

/// The warmup-key context for index `index` under `run_id`.
///
/// - [`CacheMode::Miss`]: `warm-{index}` — a namespace disjoint from the
///   measured keys, so measured miss keys are never pre-warmed.
/// - [`CacheMode::Hit`]: EXACTLY the measured key, so a cache-hit workload's
///   measured requests genuinely hit.
fn warmup_context(cache_mode: CacheMode, run_id: u64, index: u64) -> String {
    match cache_mode {
        CacheMode::Hit => measure_context(run_id, index),
        CacheMode::Miss => format!("warm-{index}"),
    }
}

/// The seed-aware measured context: an optional length-specific `seed` (e.g. a
/// base input approximating a target sequence length) is prefixed onto the
/// per-run `measure-{run_id}-{index}` namespace. An empty seed reproduces
/// [`measure_context`] exactly.
fn seeded_measure_context(seed: &str, run_id: u64, index: u64) -> String {
    if seed.is_empty() {
        measure_context(run_id, index)
    } else {
        format!("{seed}-measure-{run_id}-{index}")
    }
}

/// The seed-aware warmup context (see [`warmup_context`]): Hit pre-warms
/// EXACTLY the seeded measured key; Miss warms a seed-scoped `-warm-{index}`
/// namespace disjoint from the measured namespace. An empty seed reproduces
/// [`warmup_context`] exactly.
fn seeded_warm_context(cache_mode: CacheMode, seed: &str, run_id: u64, index: u64) -> String {
    match cache_mode {
        CacheMode::Hit => seeded_measure_context(seed, run_id, index),
        CacheMode::Miss => {
            if seed.is_empty() {
                warmup_context(cache_mode, run_id, index)
            } else {
                format!("{seed}-warm-{index}")
            }
        }
    }
}

/// Verify the harness's OWN methodology from the cache counters captured around
/// a measured window.
///
/// Miss-mode: every measured request must be a genuine miss, so
/// `delta_misses == measured` and `delta_hits == 0`. Hit-mode: every measured
/// request must be a genuine hit, so `delta_hits == measured` and
/// `delta_misses == 0`. A violation (e.g. warmup/measured key collision after a
/// refactor) returns a [`BenchError::Methodology`] instead of silently producing
/// invalid benchmark numbers.
fn verify_window(
    cache_mode: CacheMode,
    before: MetricsSnapshot,
    after: MetricsSnapshot,
    measured: u64,
) -> Result<(), BenchError> {
    let delta_hits = after.cache_hits.saturating_sub(before.cache_hits);
    let delta_misses = after.cache_misses.saturating_sub(before.cache_misses);
    match cache_mode {
        CacheMode::Miss => {
            if delta_misses != measured {
                return Err(BenchError::Methodology(format!(
                    "miss-mode methodology violation: expected {measured} cache misses in the measured window, observed delta_misses={delta_misses}; measured keys must never be pre-warmed"
                )));
            }
            if delta_hits != 0 {
                return Err(BenchError::Methodology(format!(
                    "miss-mode methodology violation: expected 0 cache hits in the measured window, observed delta_hits={delta_hits}"
                )));
            }
        }
        CacheMode::Hit => {
            if delta_hits != measured {
                return Err(BenchError::Methodology(format!(
                    "hit-mode methodology violation: expected {measured} cache hits in the measured window, observed delta_hits={delta_hits}; measured keys must be pre-warmed"
                )));
            }
            if delta_misses != 0 {
                return Err(BenchError::Methodology(format!(
                    "hit-mode methodology violation: expected 0 cache misses in the measured window, observed delta_misses={delta_misses}"
                )));
            }
        }
    }
    Ok(())
}

/// A benchmark run over the persistent gRPC channel for a topology/cache-mode.
///
/// Connects the dummy Praxis once (persistent channel, I-008: no reconnect per
/// call), warms the requested cache mode, then measures a workload of
/// per-request RTTs and reduces them to a percentile distribution. When a
/// [`Metrics`] handle is supplied it also PROVES its own methodology (see the
/// module docs). Serial measurement uses the shared [`Mutex`]`<DummyPraxis>`;
/// [`BenchmarkRun::measure_concurrent`] uses one per-worker client per worker
/// thread over the persistent channel.
pub struct BenchmarkRun {
    addr: String,
    praxis: Mutex<DummyPraxis>,
    cache_mode: CacheMode,
    run_id: u64,
    metrics: Option<Metrics>,
    /// A per-run seed (e.g. a length-specific base input) prefixed onto every
    /// measured/warmup context so a scenario genuinely exercises the intended
    /// input length while keeping the disjoint warmup/measured namespaces.
    /// Empty (the default) reproduces the original key scheme exactly.
    seed: String,
}

impl BenchmarkRun {
    /// Connect the dummy Praxis to `addr` and configure a topology/cache-mode.
    ///
    /// No methodology self-check is performed (no shared [`Metrics`] handle).
    /// Prefer [`BenchmarkRun::with_metrics`] when a server's counters are
    /// available so the harness proves its own methodology.
    pub fn new(
        addr: impl AsRef<str>,
        topology: Topology,
        cache_mode: CacheMode,
    ) -> io::Result<Self> {
        Self::connect(addr, topology, cache_mode, None)
    }

    /// Connect and share the server's [`Metrics`] handle so the harness can
    /// PROVE its own methodology (cache-hit/miss deltas over the measured window).
    pub fn with_metrics(
        addr: impl AsRef<str>,
        topology: Topology,
        cache_mode: CacheMode,
        metrics: Metrics,
    ) -> io::Result<Self> {
        Self::connect(addr, topology, cache_mode, Some(metrics))
    }

    fn connect(
        addr: impl AsRef<str>,
        _topology: Topology,
        cache_mode: CacheMode,
        metrics: Option<Metrics>,
    ) -> io::Result<Self> {
        let addr = addr.as_ref().to_string();
        let praxis = DummyPraxis::connect(&addr)?;
        Ok(BenchmarkRun {
            addr,
            praxis: Mutex::new(praxis),
            cache_mode,
            run_id: NEXT_RUN_ID.fetch_add(1, Ordering::SeqCst),
            metrics,
            seed: String::new(),
        })
    }

    /// Prefix a length-specific seed onto every measured/warmup context.
    ///
    /// The seed (e.g. a base input whose tokenized length approximates a target
    /// sequence length) is prepended to each per-run `measure-{run_id}-{index}`
    /// / `warm-{index}` key, so the scenario's requests genuinely carry the
    /// intended input length while the harness's disjoint key namespaces and
    /// methodology self-check are preserved. An empty seed (the default) keeps
    /// the original key scheme byte-for-byte identical.
    pub fn with_seed(mut self, seed: impl Into<String>) -> Self {
        self.seed = seed.into();
        self
    }

    /// The unique per-run id, usable to reconstruct the exact measured contexts.
    pub fn run_id(&self) -> u64 {
        self.run_id
    }

    /// The measured context for `index`: the length-specific `seed` (if any)
    /// plus a per-run, per-index `measure-{run_id}-{index}` namespace that is
    /// NEVER pre-warmed in Miss mode and EXACTLY pre-warmed in Hit mode.
    fn measured_context(&self, index: u64) -> String {
        seeded_measure_context(&self.seed, self.run_id, index)
    }

    /// The warmup context for `index` (see [`warmup_context`]): Hit pre-warms
    /// EXACTLY the measured key; Miss warms a seed-scoped `-warm-{index}`
    /// namespace disjoint from the measured namespace.
    fn warm_context(&self, index: u64) -> String {
        seeded_warm_context(self.cache_mode, &self.seed, self.run_id, index)
    }

    /// Send one classify-and-route turn with an explicit context and return its
    /// measured RTT, over the shared serial dummy-Praxis client.
    fn send_one(&self, context: &str, index: u64) -> Result<std::time::Duration, tonic::Status> {
        let req = DummyRequest {
            request_id: format!("bench-{index}"),
            session_id: format!("bench-session-{}", index % 4),
            context: context.to_string(),
            signals: vec!["sensitivity".to_string()],
            deadline: None,
        };
        let outcome = self.praxis.lock().unwrap().classify_and_route(req)?;
        Ok(outcome.rtt)
    }

    /// Warm up the requested cache mode with `n` requests (results discarded).
    ///
    /// - [`CacheMode::Miss`]: warms the disjoint `warm-{i}` namespace so measured
    ///   miss keys are never pre-warmed.
    /// - [`CacheMode::Hit`]: deliberately pre-warms EXACTLY the measured keys
    ///   (`measure-{run_id}-{i}`), so a later measurement genuinely hits.
    pub fn warmup(&self, n: u64) -> Result<(), BenchError> {
        for i in 0..n {
            let context = self.warm_context(i);
            self.send_one(&context, i).map_err(BenchError::Request)?;
        }
        Ok(())
    }

    /// Measure `n` requests (per-run `measure-{run_id}-{i}` keys) and reduce
    /// their per-request RTTs to a distribution.
    ///
    /// With a shared [`Metrics`] handle, PROVES the methodology: snapshots the
    /// service's cache-hit/miss counters before and after the window and asserts
    /// the expected deltas (miss-mode `delta_misses == n`; hit-mode
    /// `delta_hits == n`). A violation returns a [`BenchError::Methodology`].
    pub fn measure(&self, n: u64) -> Result<RttDistribution, BenchError> {
        let before = self.metrics.as_ref().map(|m| m.snapshot());
        let mut samples = Vec::with_capacity(n as usize);
        for i in 0..n {
            samples.push(
                self.send_one(&self.measured_context(i), i)
                    .map_err(BenchError::Request)?,
            );
        }
        self.assert_methodology(before, n)?;
        Ok(RttDistribution::from_samples(samples))
    }

    /// Measure `n` requests under `concurrency` concurrent workers, each with its
    /// OWN dummy-Praxis client over the persistent channel, and reduce their RTTs
    /// to the same percentile distribution.
    ///
    /// The `Mutex<DummyPraxis>` serial loop cannot overlap requests, so this uses
    /// one per-worker client per worker thread (P-020 concurrency 1 / P-021
    /// concurrency 4). Keys are the per-run `measure-{run_id}-{i}` namespace,
    /// exactly as [`BenchmarkRun::measure`], and the same methodology self-check
    /// is applied.
    pub fn measure_concurrent(
        &self,
        n: u64,
        concurrency: u64,
    ) -> Result<RttDistribution, BenchError> {
        let before = self.metrics.as_ref().map(|m| m.snapshot());
        let samples = self.run_concurrent(n, concurrency)?;
        self.assert_methodology(before, n)?;
        Ok(RttDistribution::from_samples(samples))
    }

    /// Assert the methodology by comparing cache-hit/miss deltas over the window.
    fn assert_methodology(
        &self,
        before: Option<MetricsSnapshot>,
        measured: u64,
    ) -> Result<(), BenchError> {
        let Some(metrics) = &self.metrics else {
            return Ok(());
        };
        let before = before.ok_or_else(|| {
            BenchError::Methodology("no baseline metrics snapshot captured".to_string())
        })?;
        verify_window(self.cache_mode, before, metrics.snapshot(), measured)
    }

    /// Distribute `n` requests across `concurrency` worker threads, each with a
    /// fresh per-worker dummy-Praxis client over the persistent channel, and
    /// collect every measured RTT.
    fn run_concurrent(
        &self,
        n: u64,
        concurrency: u64,
    ) -> Result<Vec<std::time::Duration>, BenchError> {
        let workers = concurrency.max(1);
        let addr = self.addr.clone();
        let run_id = self.run_id;
        let seed = self.seed.clone();
        let mut handles = Vec::with_capacity(workers as usize);
        let base = n / workers;
        let extra = n % workers;
        let mut start = 0u64;
        for w in 0..workers {
            let count = base + if w < extra { 1 } else { 0 };
            let end = start + count;
            let addr = addr.clone();
            let seed = seed.clone();
            handles.push(std::thread::spawn(move || -> Result<Vec<_>, BenchError> {
                // Per-worker client over the persistent channel (I-008).
                let mut praxis = DummyPraxis::connect(addr).map_err(BenchError::Io)?;
                let mut samples = Vec::with_capacity(count as usize);
                for i in start..end {
                    let context = seeded_measure_context(&seed, run_id, i);
                    let req = DummyRequest {
                        request_id: format!("bench-concurrent-{i}"),
                        session_id: format!("bench-session-{}", i % 4),
                        context,
                        signals: vec!["sensitivity".to_string()],
                        deadline: None,
                    };
                    let outcome = praxis
                        .classify_and_route(req)
                        .map_err(BenchError::Request)?;
                    samples.push(outcome.rtt);
                }
                Ok(samples)
            }));
            start = end;
        }
        let mut samples = Vec::new();
        for handle in handles {
            let worker = handle.join().map_err(|_| BenchError::Thread)?;
            samples.extend(worker?);
        }
        Ok(samples)
    }
}

/// A benchmark failure: a request could not be completed, the methodology
/// self-check failed, or a worker thread panicked.
#[derive(Debug)]
pub enum BenchError {
    /// A classify request failed over the gRPC channel.
    Request(tonic::Status),
    /// A per-worker dummy-Praxis client could not connect.
    Io(std::io::Error),
    /// A concurrent worker thread panicked.
    Thread,
    /// The harness's OWN methodology self-check failed (e.g. a measured miss key
    /// was pre-warmed, or a measured hit key was not).
    Methodology(String),
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::Request(s) => write!(f, "benchmark request failed: {s}"),
            BenchError::Io(e) => write!(f, "benchmark worker connect failed: {e}"),
            BenchError::Thread => write!(f, "benchmark worker thread panicked"),
            BenchError::Methodology(m) => write!(f, "benchmark methodology violation: {m}"),
        }
    }
}

impl std::error::Error for BenchError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricsSnapshot;

    /// The percentile accessors are monotone and the max is at least p99, so a
    /// captured distribution always satisfies the AC-011 invariant that a
    /// mean-only measurement cannot.
    #[test]
    fn percentile_accessors_are_monotone() {
        let dist = RttDistribution::from_samples(vec![
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(2),
            std::time::Duration::from_millis(3),
            std::time::Duration::from_millis(4),
            std::time::Duration::from_millis(5),
        ]);
        assert!(dist.p50() <= dist.p90());
        assert!(dist.p90() <= dist.p95());
        assert!(dist.p95() <= dist.p99());
        assert!(dist.p99() <= dist.max());
        assert!(dist.p50() > std::time::Duration::ZERO);
    }

    /// Miss-mode warmup keys (`warm-{i}`) must be DISJOINT from the measured keys
    /// (`measure-{run_id}-{i}`), so a measured miss request is never a silent hit.
    #[test]
    fn miss_warmup_and_measured_keyspaces_are_disjoint() {
        let run_id = 7;
        for i in 0..100 {
            let warm = warmup_context(CacheMode::Miss, run_id, i);
            let measured = measure_context(run_id, i);
            assert_ne!(
                warm, measured,
                "miss-mode warmup key must never collide with a measured key"
            );
            assert!(
                !measured.starts_with("warm-"),
                "measured keys must live in the measure namespace"
            );
        }
    }

    /// Hit-mode warmup must pre-warm EXACTLY the measured keys, so a measured hit
    /// request is a genuine cache hit.
    #[test]
    fn hit_warmup_prewarms_exactly_the_measured_keys() {
        let run_id = 9;
        for i in 0..100 {
            assert_eq!(
                warmup_context(CacheMode::Hit, run_id, i),
                measure_context(run_id, i),
                "hit-mode warmup must pre-warm exactly the measured key"
            );
        }
    }

    /// Distinct runs must use distinct measured key namespaces.
    #[test]
    fn distinct_runs_use_distinct_measured_namespaces() {
        for i in 0..10 {
            assert_ne!(
                measure_context(1, i),
                measure_context(2, i),
                "two runs must not share measured keys"
            );
        }
    }

    /// With a non-empty seed (the benchmark runner's length-specific base), the
    /// measured and miss-warmup namespaces stay DISJOINT and hit-mode warmup
    /// pre-warms EXACTLY the measured keys, so the harness's methodology
    /// self-check still holds for length-variable scenarios.
    #[test]
    fn seed_aware_namespaces_preserve_methodology() {
        let seed = "benchmark benchmark";
        for i in 0..100 {
            // Miss-mode warmup must never collide with a measured key.
            assert_ne!(
                seeded_warm_context(CacheMode::Miss, seed, 7, i),
                seeded_measure_context(seed, 7, i),
                "seeded miss-mode warmup must never collide with a measured key"
            );
            // The measured context carries the seed prefix (the intended length).
            assert!(
                seeded_measure_context(seed, 7, i).starts_with(seed),
                "seeded measured context must carry the length-specific seed"
            );
            // Hit-mode warmup pre-warms EXACTLY the measured key.
            assert_eq!(
                seeded_warm_context(CacheMode::Hit, seed, 7, i),
                seeded_measure_context(seed, 7, i),
                "seeded hit-mode warmup must pre-warm exactly the measured key"
            );
        }
        // An empty seed reproduces the original key scheme exactly.
        assert_eq!(seeded_measure_context("", 7, 3), measure_context(7, 3));
        assert_eq!(
            seeded_warm_context(CacheMode::Miss, "", 7, 3),
            warmup_context(CacheMode::Miss, 7, 3)
        );
    }

    /// A snapshot helper: a metrics snapshot with the given cache counters.
    fn snap(hits: u64, misses: u64) -> MetricsSnapshot {
        MetricsSnapshot {
            queue: std::time::Duration::ZERO,
            tokenize: std::time::Duration::ZERO,
            forward: std::time::Duration::ZERO,
            total: std::time::Duration::ZERO,
            cache_hits: hits,
            cache_misses: misses,
        }
    }

    /// Miss-mode methodology: measured-count misses in the window with zero hits
    /// PASSES; a pre-warmed measured key (delta_misses != measured) FAILS.
    #[test]
    fn miss_methodology_rejects_prewarmed_measured_keys() {
        // Correct: 1000 misses, 0 hits in the window.
        verify_window(CacheMode::Miss, snap(0, 0), snap(0, 1000), 1000)
            .expect("a genuine miss window must pass");

        // Buggy: measured miss keys were pre-warmed -> only 200 misses in a 1000
        // window (the other 800 were hits). The harness must reject this.
        let err = verify_window(CacheMode::Miss, snap(0, 0), snap(800, 200), 1000)
            .expect_err("a miss window whose measured keys were pre-warmed must fail");
        assert!(err.to_string().contains("miss-mode"));
    }

    /// Hit-mode methodology: measured-count hits in the window with zero misses
    /// PASSES; an un-pre-warmed measured key (delta_hits != measured) FAILS.
    #[test]
    fn hit_methodology_rejects_unprewarmed_measured_keys() {
        // Correct: 1000 hits, 0 misses in the window.
        verify_window(CacheMode::Hit, snap(0, 0), snap(1000, 0), 1000)
            .expect("a genuine hit window must pass");

        // Buggy: only 100 of the measured keys were pre-warmed -> 100 hits and
        // 900 misses in a 1000 window. The harness must reject this.
        let err = verify_window(CacheMode::Hit, snap(0, 0), snap(100, 900), 1000)
            .expect_err("a hit window with un-pre-warmed measured keys must fail");
        assert!(err.to_string().contains("hit-mode"));
    }
}
