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

use std::io;
use std::sync::Mutex;

use crate::dummy_praxis::{DummyPraxis, DummyRequest};

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
    /// Exact-result cache hit: the same context is classified repeatedly.
    Hit,
    /// Cache miss: every request carries a unique context, so the cache never hits.
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

/// A benchmark run over the persistent gRPC channel for a topology/cache-mode.
///
/// Connects the dummy Praxis once (persistent channel, I-008: no reconnect per
/// call), warms the requested cache mode, then measures a workload of
/// per-request RTTs and reduces them to a percentile distribution.
pub struct BenchmarkRun {
    praxis: Mutex<DummyPraxis>,
    cache_mode: CacheMode,
    /// The fixed context reused for cache-hit workloads (same exact-result key).
    hit_context: String,
}

impl BenchmarkRun {
    /// Connect the dummy Praxis to `addr` and configure a topology/cache-mode.
    pub fn new(
        addr: impl AsRef<str>,
        _topology: Topology,
        cache_mode: CacheMode,
    ) -> io::Result<Self> {
        let praxis = DummyPraxis::connect(addr)?;
        Ok(BenchmarkRun {
            praxis: Mutex::new(praxis),
            cache_mode,
            hit_context: "benchmark golden sensitivity input".to_string(),
        })
    }

    /// Send one classify-and-route turn and return its measured RTT.
    fn send_one(&self, index: u64) -> Result<std::time::Duration, tonic::Status> {
        let context = match self.cache_mode {
            CacheMode::Hit => self.hit_context.clone(),
            CacheMode::Miss => format!("unique sensitivity context {index} with distinct tokens"),
        };
        let req = DummyRequest {
            request_id: format!("bench-{index}"),
            session_id: format!("bench-session-{}", index % 4),
            context,
            signals: vec!["sensitivity".to_string()],
            deadline: None,
        };
        let outcome = self.praxis.lock().unwrap().classify_and_route(req)?;
        Ok(outcome.rtt)
    }

    /// Warm up the requested cache mode with `n` requests (results discarded).
    pub fn warmup(&self, n: u64) -> Result<(), BenchError> {
        for i in 0..n {
            self.send_one(i).map_err(BenchError::Request)?;
        }
        Ok(())
    }

    /// Measure `n` requests and reduce their per-request RTTs to a distribution.
    pub fn measure(&self, n: u64) -> Result<RttDistribution, BenchError> {
        let mut samples = Vec::with_capacity(n as usize);
        for i in 0..n {
            samples.push(self.send_one(i).map_err(BenchError::Request)?);
        }
        Ok(RttDistribution::from_samples(samples))
    }
}

/// A benchmark failure: a request could not be completed over the channel.
#[derive(Debug)]
pub enum BenchError {
    /// A classify request failed over the gRPC channel.
    Request(tonic::Status),
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::Request(s) => write!(f, "benchmark request failed: {s}"),
        }
    }
}

impl std::error::Error for BenchError {}

#[cfg(test)]
mod tests {
    use super::*;

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
}
