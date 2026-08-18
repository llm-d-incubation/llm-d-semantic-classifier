//! Reviewer-run inference floor measurement (temporary; not an AC test).
//! Measures the REAL Candle classify() latency by input length, single-request,
//! no cache, no network — the architecture-independent lower bound.
use llm_d_sc::classify::{CandleClassifier, ClassificationInput, ClassifierRuntime};
use std::collections::HashMap;
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
fn inference_floor_by_length() {
    let dir = std::path::PathBuf::from(
        std::env::var("LLM_D_SC_MODEL_DIR")
            .unwrap_or_else(|_| "artifacts/models/sensitivity".into()),
    );
    let clf = CandleClassifier::from_modelcar(&dir).expect("real model must load");
    let word = "sensitivity classification workload token ";
    println!("\n  tokens |   n |     p50 |     p90 |     p95 |     p99 |     max |   req/s");
    println!("  -------+-----+---------+---------+---------+---------+---------+--------");
    for target in [32usize, 64, 128, 256] {
        let text = word.repeat(target / 5 + 1);
        let inp = ClassificationInput {
            text: text.clone(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: HashMap::new(),
        };
        for _ in 0..20 {
            let _ = clf.classify(inp.clone());
        } // warm caches/allocator
        let n = 200;
        let mut lat = Vec::with_capacity(n);
        let t0 = Instant::now();
        for _ in 0..n {
            let s = Instant::now();
            clf.classify(inp.clone()).expect("classify must succeed");
            lat.push(s.elapsed());
        }
        let wall = t0.elapsed();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!(
            "  {:>6} | {:>3} | {:>6.2}m | {:>6.2}m | {:>6.2}m | {:>6.2}m | {:>6.2}m | {:>6.0}",
            target,
            n,
            ms(pct(&lat, 0.50)),
            ms(pct(&lat, 0.90)),
            ms(pct(&lat, 0.95)),
            ms(pct(&lat, 0.99)),
            ms(*lat.iter().max().unwrap()),
            n as f64 / wall.as_secs_f64()
        );
    }
    println!();
}
