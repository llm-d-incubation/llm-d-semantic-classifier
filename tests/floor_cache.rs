//! Reviewer-run cache-hit measurement (temporary). Measures the CACHE LAYER's
//! own cost: BLAKE3 versioned key construction + lookup + result clone, at
//! several input sizes. NOTE: the cache is not yet on the production Candle
//! path (P0 #2), so this is the floor a wired hit will land near, not an
//! end-to-end production hit.
use llm_d_sc::cache::{CacheKey, SharedCache};
use llm_d_sc::classify::{ClassificationResult, ClassifyStatus, RankedSignal};
use std::time::{Duration, Instant};

fn pct(v: &[Duration], p: f64) -> Duration {
    let mut s = v.to_vec();
    s.sort();
    s[(((s.len() as f64) * p).ceil() as usize)
        .saturating_sub(1)
        .min(s.len() - 1)]
}

#[test]
#[ignore]
fn cache_hit_cost() {
    let cache = SharedCache::new();
    println!("\n  input chars |    n  |  key+hit p50 |     p95 |     p99 |  hits/s");
    println!("  ------------+-------+--------------+---------+---------+---------");
    for chars in [128usize, 512, 2048, 8192] {
        let text = "sensitivity classification workload ".repeat(chars / 36 + 1);
        let k = || CacheKey::new("sensitivity", "rev-model", "rev-tok", "rev-tax", &text);
        // prime the entry (this is the miss path; cost measured elsewhere)
        let result = ClassificationResult {
            classifier_id: "sensitivity".into(),
            model_revision: "rev-model".into(),
            tokenizer_revision: "rev-tok".into(),
            taxonomy_revision: "rev-tax".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "proto-a".into(),
                score: 0.42,
            }],
        };
        let r0 = result.clone();
        let _ = cache.classify_concurrent(k(), move || Ok(r0));
        for _ in 0..500 {
            let _ = cache.classify_concurrent(k(), || unreachable!("must be a hit"));
        }
        let n = 5000;
        let mut lat = Vec::with_capacity(n);
        let t0 = Instant::now();
        for _ in 0..n {
            let s = Instant::now();
            let _ = cache.classify_concurrent(k(), || unreachable!("must be a hit"));
            lat.push(s.elapsed());
        }
        let wall = t0.elapsed();
        let us = |d: Duration| d.as_secs_f64() * 1_000_000.0;
        println!(
            "  {:>11} | {:>5} | {:>10.2}us | {:>5.2}us | {:>5.2}us | {:>7.0}",
            text.len(),
            n,
            us(pct(&lat, 0.5)),
            us(pct(&lat, 0.95)),
            us(pct(&lat, 0.99)),
            n as f64 / wall.as_secs_f64()
        );
    }
    println!();
}
