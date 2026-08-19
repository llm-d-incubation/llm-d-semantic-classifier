//! Performance evidence for the cache and the executor pool (P-001, P-004,
//! P-020, P-021).
//!
//! These assert RELATIONSHIPS, not absolute milliseconds: absolute timings vary
//! by host and would make the suite a flaky liability. The relationships are the
//! actual claims:
//!   - a cache hit is orders of magnitude cheaper than a miss;
//!   - a same-key burst coalesces instead of running one forward per caller;
//!   - four concurrent forwards deliver more throughput than one, which is
//!     exactly what the single-threaded executor could NOT do.
//!
//! Every run prints its p50/p95/p99 from the per-stage histograms, so a human
//! reading CI output gets the distribution rather than a mean.
//!
//! Requires fetched weights; run with `cargo test --test perf_concurrency -- --ignored --nocapture`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use llm_d_sc::classify::{
    CandleClassifier, ClassificationInput, ClassifierRuntime, ServiceCore,
};
use llm_d_sc::handoff::InferenceExecutor;
use llm_d_sc::metrics::{LatencyStage, Metrics};
use llm_d_sc::taxonomy::ClassifierDefinition;

fn classifier() -> CandleClassifier {
    let def = ClassifierDefinition::built_in("complexity").unwrap().unwrap();
    CandleClassifier::from_modelcar_with(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/models/complexity"),
        def,
    )
    .expect("complexity artifact must load")
}

fn input(text: &str) -> ClassificationInput {
    ClassificationInput {
        text: text.to_string(),
        requested_signals: vec!["sensitivity".to_string()],
        session_metadata: Default::default(),
    }
}

/// P-001: an in-process exact-result cache hit must be far cheaper than a miss.
#[test]
#[ignore]
fn p001_in_process_cache_hit_is_orders_of_magnitude_cheaper_than_a_miss() {
    let core = ServiceCore::new(classifier());

    let t0 = Instant::now();
    core.classify(input("a distinctive prompt for the miss path")).unwrap();
    let miss = t0.elapsed();

    // Warm, then measure hits.
    core.classify(input("a distinctive prompt for the miss path")).unwrap();
    const HITS: u32 = 200;
    let t1 = Instant::now();
    for _ in 0..HITS {
        core.classify(input("a distinctive prompt for the miss path")).unwrap();
    }
    let hit = t1.elapsed() / HITS;

    println!("P-001: miss {miss:?}, mean hit {hit:?}, ratio {:.0}x",
             miss.as_secs_f64() / hit.as_secs_f64().max(1e-9));
    assert!(
        hit * 20 < miss,
        "a cache hit ({hit:?}) must be dramatically cheaper than a miss ({miss:?})"
    );
}

/// P-004: a same-key burst must coalesce to a bounded forward count.
#[test]
#[ignore]
fn p004_same_key_burst_miss_coalesces() {
    const CALLERS: usize = 64;
    let clf = classifier();
    let forwards = clf.forward_call_counter();
    let core = Arc::new(ServiceCore::new(clf));

    let barrier = Arc::new(std::sync::Barrier::new(CALLERS));
    let started = Instant::now();
    let handles: Vec<_> = (0..CALLERS)
        .map(|_| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.classify(input("burst key")).is_ok()
            })
        })
        .collect();
    let ok = handles.into_iter().map(|h| h.join().unwrap()).filter(|b| *b).count();
    let elapsed = started.elapsed();

    let n = forwards.load(Ordering::SeqCst);
    println!("P-004: {CALLERS} same-key callers in {elapsed:?}, {n} forward(s)");
    assert_eq!(ok, CALLERS);
    assert!(
        n < CALLERS as u64 / 8,
        "{CALLERS} same-key callers ran {n} forwards; coalescing must keep this small"
    );
}

/// Drive `n` DISTINCT prompts through an executor of the given width and return
/// (wall clock, forward-stage percentiles).
fn run_at_width(width: usize, jobs: usize) -> (std::time::Duration, String) {
    // Take the CLASSIFIER's metrics handle and share it outward, which is how
    // the server wires it (`bind_with_classifier` calls `classifier.metrics()`).
    // Creating a fresh registry and injecting it downward instead leaves the
    // Candle forward recording into a different registry, and the Forward
    // histogram silently reports n=0.
    let clf = classifier();
    let metrics = clf.metrics();
    let core = ServiceCore::with_metrics(clf, metrics.clone());
    let executor = InferenceExecutor::spawn_with_workers(core, metrics.clone(), 256, width);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let started = Instant::now();
    // Distinct texts so every request is a real cache miss and the executor is
    // actually exercised; identical texts would be answered by the cache and
    // would measure nothing about worker width.
    let receivers: Vec<_> = (0..jobs)
        .map(|i| {
            executor
                .try_enqueue(input(&format!(
                    "distinct benchmark prompt number {i} about an unrelated subject"
                )))
                .expect("bound of 256 must admit")
        })
        .collect();
    for rx in receivers {
        rt.block_on(rx).unwrap().unwrap();
    }
    let elapsed = started.elapsed();

    let p = metrics.stage_percentiles(LatencyStage::Forward);
    (
        elapsed,
        format!("forward p50 {:?} p95 {:?} p99 {:?} (n={})", p.p50, p.p95, p.p99, p.count),
    )
}

/// P-020 and P-021: concurrency 1 versus concurrency 4.
///
/// This is the before/after for the executor pool fix. Under the previous
/// single-threaded executor these two configurations were the SAME
/// configuration, so throughput was flat and added concurrency only inflated
/// latency.
#[test]
#[ignore]
fn p020_p021_concurrency_one_versus_four() {
    const JOBS: usize = 24;

    let (serial, serial_stats) = run_at_width(1, JOBS);
    println!("P-020 concurrency 1: {JOBS} jobs in {serial:?} -> {serial_stats}");

    let (parallel, parallel_stats) = run_at_width(4, JOBS);
    println!("P-021 concurrency 4: {JOBS} jobs in {parallel:?} -> {parallel_stats}");

    let speedup = serial.as_secs_f64() / parallel.as_secs_f64();
    println!("P-021: speedup {speedup:.2}x over concurrency 1");

    assert!(
        speedup > 1.3,
        "four workers gave only {speedup:.2}x over one ({serial:?} -> {parallel:?}); \
         the executor is not executing forwards in parallel"
    );
}
