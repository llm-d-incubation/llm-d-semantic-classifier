//! Reviewer-run concurrency measurement (temporary). Shared resident classifier,
//! N threads issuing real forwards, measuring per-request latency + aggregate
//! throughput. Reveals whether a single forward already saturates the cores.
use llm_d_sc::classify::{CandleClassifier, ClassificationInput, ClassifierRuntime};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn pct(v: &[Duration], p: f64) -> Duration {
    let mut s = v.to_vec(); s.sort();
    s[(((s.len() as f64) * p).ceil() as usize).saturating_sub(1).min(s.len() - 1)]
}

#[test]
#[ignore]
fn concurrency_scaling() {
    let dir = std::path::PathBuf::from(
        std::env::var("LLM_D_SC_MODEL_DIR").unwrap_or_else(|_| "artifacts/models/sensitivity".into()));
    let clf = Arc::new(CandleClassifier::from_modelcar(&dir).expect("model must load"));
    let text = "sensitivity classification workload token ".repeat(13); // ~64 tokens
    let mk = || ClassificationInput { text: text.clone(),
        requested_signals: vec!["sensitivity".into()], session_metadata: HashMap::new() };
    for _ in 0..20 { let _ = clf.classify(mk()); }
    println!("\n  cores: {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    println!("  conc | reqs |     p50 |     p95 |     p99 |     max | agg req/s");
    println!("  -----+------+---------+---------+---------+---------+----------");
    for conc in [1usize, 2, 4, 8] {
        let per = 60;
        let t0 = Instant::now();
        let hs: Vec<_> = (0..conc).map(|_| {
            let c = Arc::clone(&clf); let t = text.clone();
            std::thread::spawn(move || {
                let mut lat = Vec::with_capacity(per);
                for _ in 0..per {
                    let i = ClassificationInput { text: t.clone(),
                        requested_signals: vec!["sensitivity".into()], session_metadata: HashMap::new() };
                    let s = Instant::now();
                    c.classify(i).expect("classify must succeed");
                    lat.push(s.elapsed());
                }
                lat })
        }).collect();
        let mut all = Vec::new();
        for h in hs { all.extend(h.join().unwrap()); }
        let wall = t0.elapsed();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!("  {:>4} | {:>4} | {:>6.1}m | {:>6.1}m | {:>6.1}m | {:>6.1}m | {:>8.0}",
            conc, all.len(), ms(pct(&all,0.5)), ms(pct(&all,0.95)), ms(pct(&all,0.99)),
            ms(*all.iter().max().unwrap()), all.len() as f64 / wall.as_secs_f64());
    }
    println!();
}
