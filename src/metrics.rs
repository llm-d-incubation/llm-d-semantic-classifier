//! Latency-decomposition metrics for the classification service.
//!
//! AC-012 requires the queue, tokenize, forward, and total service latency to be
//! independently visible. This module defines a small registry that records
//! per-request latency stages and cache hit/miss counters, and exposes an
//! immutable [`MetricsSnapshot`].
//!
//! [`Metrics`] uses interior mutability ([`Arc`]`<Mutex<_>>`) so it can be shared
//! by the concurrently-cloned [`crate::classify::ClassifyService`] and read from
//! the [`crate::grpc::classify::ClassifyServer`] metrics surface without holding
//! a `&mut` across the pipeline. Cloning a [`Metrics`] shares the same counters
//! and accumulators, so a cache hit observed through one clone is reflected in
//! every clone.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One independently-measured latency stage of a classification request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyStage {
    /// Time spent waiting in the bounded queue before work begins.
    Queue,
    /// Time spent in the tokenizer (tokenize stage).
    Tokenize,
    /// Time spent in the model forward (embed/rank stage).
    Forward,
    /// Total end-to-end service latency from admission to response.
    Total,
}

/// An immutable point-in-time view of the recorded latency stages and counters.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricsSnapshot {
    /// Total accumulated queue-wait latency.
    pub queue: Duration,
    /// Total accumulated tokenize latency.
    pub tokenize: Duration,
    /// Total accumulated model-forward latency.
    pub forward: Duration,
    /// Total accumulated end-to-end service latency.
    pub total: Duration,
    /// Number of classification requests served from the exact-result cache.
    pub cache_hits: u64,
    /// Number of classification requests that ran a real forward (cache miss).
    pub cache_misses: u64,
}

/// The shared latency/counter registry behind the metrics surface.
///
/// [`Clone`] shares the same underlying state, so a pipeline and its server can
/// write and snapshot the same decomposition.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    inner: Arc<Mutex<Inner>>,
}

/// The mutable accumulator state guarded by [`Metrics`].
#[derive(Debug, Default)]
struct Inner {
    queue: Duration,
    tokenize: Duration,
    forward: Duration,
    total: Duration,
    cache_hits: u64,
    cache_misses: u64,
}

impl Metrics {
    /// An empty metrics registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `latency` against a latency stage, accumulating over requests.
    pub fn record_stage(&self, stage: LatencyStage, latency: Duration) {
        let mut inner = self.inner.lock().unwrap();
        let slot = match stage {
            LatencyStage::Queue => &mut inner.queue,
            LatencyStage::Tokenize => &mut inner.tokenize,
            LatencyStage::Forward => &mut inner.forward,
            LatencyStage::Total => &mut inner.total,
        };
        *slot += latency;
    }

    /// Record one classification served from the exact-result cache.
    pub fn record_cache_hit(&self) {
        self.inner.lock().unwrap().cache_hits += 1;
    }

    /// Record one classification that ran a real forward (cache miss).
    pub fn record_cache_miss(&self) {
        self.inner.lock().unwrap().cache_misses += 1;
    }

    /// An immutable snapshot of the accumulated latency decomposition.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let inner = self.inner.lock().unwrap();
        MetricsSnapshot {
            queue: inner.queue,
            tokenize: inner.tokenize,
            forward: inner.forward,
            total: inner.total,
            cache_hits: inner.cache_hits,
            cache_misses: inner.cache_misses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LatencyStage, Metrics};
    use std::time::Duration;

    /// U-080 (AC-012): the registry emits queue/tokenize/forward/total stages
    /// independently and each component is bounded by the total.
    #[test]
    fn u080_queue_tokenize_forward_total_emitted() {
        let metrics = Metrics::new();
        metrics.record_stage(LatencyStage::Queue, Duration::from_millis(1));
        metrics.record_stage(LatencyStage::Tokenize, Duration::from_millis(2));
        metrics.record_stage(LatencyStage::Forward, Duration::from_millis(3));
        metrics.record_stage(LatencyStage::Total, Duration::from_millis(6));

        let snap = metrics.snapshot();
        assert!(snap.queue > Duration::ZERO);
        assert!(snap.tokenize > Duration::ZERO);
        assert!(snap.forward > Duration::ZERO);
        assert!(snap.total > Duration::ZERO);
        assert!(snap.queue <= snap.total);
        assert!(snap.tokenize <= snap.total);
        assert!(snap.forward <= snap.total);
    }

    /// U-081 (AC-012): cache hit/miss counters are counted independently and
    /// partition every request exactly.
    #[test]
    fn u081_cache_hit_miss_counters_partition() {
        let metrics = Metrics::new();
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let snap = metrics.snapshot();
        assert_eq!(snap.cache_hits, 2);
        assert_eq!(snap.cache_misses, 1);
        assert_eq!(snap.cache_hits + snap.cache_misses, 3);
    }
}
