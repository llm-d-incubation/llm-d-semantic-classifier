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

/// Number of sub-buckets per power-of-two octave. Eight sub-buckets bound the
/// relative bucket width to 1/8, so a reported percentile is within 12.5% of the
/// true sample. That is precise enough to compare a before/after change and
/// cheap enough to keep resident forever.
const SUB_BUCKETS: usize = 8;
/// Octaves covered: 1 microsecond through roughly 2^40 us (~13 days).
const OCTAVES: usize = 41;
const BUCKETS: usize = OCTAVES * SUB_BUCKETS;

/// A fixed-memory logarithmic latency histogram.
///
/// AC-012 forbids average-only latency claims, but a running SUM can only ever
/// yield a mean: it cannot distinguish a service where every request takes 10 ms
/// from one where 99% take 1 ms and 1% take 900 ms. Percentiles need the
/// distribution, and a long-lived service cannot retain every sample, so the
/// distribution is kept in bounded log-scale buckets (about 2.6 KB per stage,
/// constant regardless of request count).
#[derive(Debug)]
struct Histogram {
    buckets: Box<[u64; BUCKETS]>,
    count: u64,
    max_us: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram {
            buckets: Box::new([0; BUCKETS]),
            count: 0,
            max_us: 0,
        }
    }
}

impl Histogram {
    /// Bucket index for a microsecond value: octave (floor log2) plus a linear
    /// sub-bucket taken from the bits just below the leading one.
    fn index(us: u64) -> usize {
        if us < SUB_BUCKETS as u64 {
            return us as usize;
        }
        let octave = 63 - us.leading_zeros() as usize;
        let shift = octave - 3;
        let sub = ((us >> shift) & (SUB_BUCKETS as u64 - 1)) as usize;
        let idx = octave * SUB_BUCKETS + sub;
        idx.min(BUCKETS - 1)
    }

    /// The lower bound in microseconds of a bucket index.
    fn lower_bound(idx: usize) -> u64 {
        if idx < SUB_BUCKETS {
            return idx as u64;
        }
        let octave = idx / SUB_BUCKETS;
        let sub = (idx % SUB_BUCKETS) as u64;
        let shift = octave - 3;
        // index() computes sub = (us >> shift) & 7 where (us >> shift) lies in
        // 8..=15, so the bucket covers [(8 + sub) << shift, (9 + sub) << shift).
        (SUB_BUCKETS as u64 + sub) << shift
    }

    fn record(&mut self, latency: Duration) {
        let us = latency.as_micros().min(u64::MAX as u128) as u64;
        self.buckets[Self::index(us)] += 1;
        self.count += 1;
        self.max_us = self.max_us.max(us);
    }

    /// The value at quantile `q` (0.0..=1.0), reported as the lower bound of the
    /// bucket containing it. Returns zero when no samples were recorded.
    fn quantile(&self, q: f64) -> Duration {
        if self.count == 0 {
            return Duration::ZERO;
        }
        let target = ((self.count as f64) * q).ceil().max(1.0) as u64;
        let mut cumulative = 0u64;
        for (i, n) in self.buckets.iter().enumerate() {
            cumulative += n;
            if cumulative >= target {
                return Duration::from_micros(Self::lower_bound(i));
            }
        }
        Duration::from_micros(self.max_us)
    }
}

/// The percentile decomposition of one latency stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StagePercentiles {
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
    pub count: u64,
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
    hist_queue: Histogram,
    hist_tokenize: Histogram,
    hist_forward: Histogram,
    hist_total: Histogram,
}

impl Metrics {
    /// An empty metrics registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `latency` against a latency stage, accumulating over requests.
    pub fn record_stage(&self, stage: LatencyStage, latency: Duration) {
        let mut guard = self.inner.lock().unwrap();
        // Reborrow through the guard once so the compiler can split the borrow
        // across two distinct fields of `Inner`.
        let inner = &mut *guard;
        let (slot, hist) = match stage {
            LatencyStage::Queue => (&mut inner.queue, &mut inner.hist_queue),
            LatencyStage::Tokenize => (&mut inner.tokenize, &mut inner.hist_tokenize),
            LatencyStage::Forward => (&mut inner.forward, &mut inner.hist_forward),
            LatencyStage::Total => (&mut inner.total, &mut inner.hist_total),
        };
        *slot += latency;
        hist.record(latency);
    }

    /// The percentile decomposition of one latency stage.
    ///
    /// This is the surface AC-012 needs: a mean cannot show a tail, and a
    /// performance claim made from a mean alone is not evidence.
    pub fn stage_percentiles(&self, stage: LatencyStage) -> StagePercentiles {
        let inner = self.inner.lock().unwrap();
        let h = match stage {
            LatencyStage::Queue => &inner.hist_queue,
            LatencyStage::Tokenize => &inner.hist_tokenize,
            LatencyStage::Forward => &inner.hist_forward,
            LatencyStage::Total => &inner.hist_total,
        };
        StagePercentiles {
            p50: h.quantile(0.50),
            p95: h.quantile(0.95),
            p99: h.quantile(0.99),
            max: Duration::from_micros(h.max_us),
            count: h.count,
        }
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
    use super::{Histogram, LatencyStage, Metrics};
    use std::time::Duration;

    /// U-086 (AC-012): percentiles must distinguish distributions a mean cannot.
    ///
    /// Two workloads with the SAME total (and therefore the same mean) but very
    /// different tails must report very different p99. This is the property the
    /// old sum-only registry could not express, and the reason a performance
    /// claim from a mean alone is not evidence.
    #[test]
    fn u086_percentiles_separate_workloads_with_identical_means() {
        let flat = Metrics::new();
        for _ in 0..100 {
            flat.record_stage(LatencyStage::Total, Duration::from_millis(10));
        }
        // 95 fast requests and a 5% tail, summing to the SAME 1000 ms total.
        // The tail must sit at or above the 99th sample for p99 to see it: a
        // single outlier in 100 samples is p100, not p99, and asserting
        // otherwise would be asserting a bug.
        let spiky = Metrics::new();
        for _ in 0..95 {
            spiky.record_stage(LatencyStage::Total, Duration::from_millis(1));
        }
        for _ in 0..5 {
            spiky.record_stage(LatencyStage::Total, Duration::from_millis(181));
        }

        // Same accumulated total => identical means.
        assert_eq!(
            flat.snapshot().total,
            spiky.snapshot().total,
            "the two workloads must have the same sum, so the mean cannot tell them apart"
        );

        let f = flat.stage_percentiles(LatencyStage::Total);
        let s = spiky.stage_percentiles(LatencyStage::Total);
        assert!(
            s.p99 > f.p99 * 10,
            "p99 must expose the tail the mean hides: flat p99 {:?} vs spiky p99 {:?}",
            f.p99,
            s.p99
        );
        assert!(
            s.p50 < f.p50,
            "the spiky workload's median must be LOWER despite the same mean"
        );
    }

    /// U-087: reported quantiles must be within the documented bucket error.
    #[test]
    fn u087_quantiles_are_within_the_documented_bucket_error() {
        // A known distribution: 1..=1000 microseconds, one sample each.
        let m = Metrics::new();
        for us in 1..=1000u64 {
            m.record_stage(LatencyStage::Forward, Duration::from_micros(us));
        }
        let p = m.stage_percentiles(LatencyStage::Forward);
        assert_eq!(p.count, 1000);
        assert_eq!(p.max, Duration::from_micros(1000));

        // Buckets report their LOWER bound, and relative width is <= 1/8, so a
        // reported quantile must lie in ((1 - 1/8) * true, true].
        for (q, truth) in [(0.50, 500.0), (0.95, 950.0), (0.99, 990.0)] {
            let got = m.stage_percentiles(LatencyStage::Forward);
            // Match on the quantile directly. A guard of the form `x if x == k`
            // is just a pattern, and clippy is right that writing it as a guard
            // hides that.
            let v = match q {
                0.50 => got.p50,
                0.95 => got.p95,
                _ => got.p99,
            }
            .as_micros() as f64;
            assert!(
                v <= truth && v >= truth * 0.875,
                "q{q} reported {v}us, expected within [{}, {truth}]",
                truth * 0.875
            );
        }
    }

    /// U-088: every bucket's reported lower bound must actually contain values
    /// mapped to it, so `index` and `lower_bound` cannot drift apart.
    #[test]
    fn u088_bucket_index_and_lower_bound_are_consistent() {
        for us in [0u64, 1, 7, 8, 9, 15, 16, 100, 999, 1_000_000, 60_000_000] {
            let idx = Histogram::index(us);
            let lo = Histogram::lower_bound(idx);
            assert!(
                lo <= us,
                "bucket {idx} for {us}us reports lower bound {lo}us, which is above the sample"
            );
            assert_eq!(
                Histogram::index(lo),
                idx,
                "the lower bound of bucket {idx} must map back to bucket {idx}"
            );
        }
    }

    /// U-089: an empty stage reports zero rather than a misleading value.
    #[test]
    fn u089_empty_stage_reports_zero() {
        let p = Metrics::new().stage_percentiles(LatencyStage::Queue);
        assert_eq!(p.count, 0);
        assert_eq!(p.p50, Duration::ZERO);
        assert_eq!(p.p99, Duration::ZERO);
    }

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
