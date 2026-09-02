//! Metrics accuracy tests for the single-flight cache path (AC-007/AC-012).
//!
//! The classification pipeline has three distinct request paths:
//!   1. TRUE HIT:       cached result, no forward              → cache_hits
//!   2. TRUE MISS:      real forward (tokenize + embed + rank) → cache_misses
//!   3. COALESCED WAIT: waited for another thread's forward    → cache_coalesced
//!
//! `coalesced_waiters_counted_in_metrics` uses a deliberately slow fake
//! runtime to make coalescing deterministic without model weights.
//! `mixed_workload_counters_match_ground_truth` uses the real Candle
//! classifier and verifies exact ground-truth counters across all three
//! paths in a mixed workload.
//!
//! Ignored tests require fetched weights:
//!   ./hack/fetch-model --classifier complexity
//!
//! Run:
//!   cargo test --test metrics_accuracy                          # synthetic only
//!   cargo test --test metrics_accuracy -- --ignored --nocapture # all (needs weights)

use std::sync::{Arc, Barrier};
use std::time::Duration;

use llm_d_sc::classify::{
    CandleClassifier, ClassificationInput, ClassificationResult, ClassifierRuntime, ClassifyError,
    ClassifyStatus, Embedding, RankedSignal, RuntimeMetadata, ServiceCore,
};
use llm_d_sc::metrics::Metrics;
use llm_d_sc::taxonomy::ClassifierDefinition;

fn classifier() -> CandleClassifier {
    let def = ClassifierDefinition::built_in("complexity")
        .unwrap()
        .unwrap();
    CandleClassifier::from_modelcar_with(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/models/complexity"),
        def,
    )
    .expect("complexity artifact must load (run ./hack/fetch-model first)")
}

fn input(text: &str) -> ClassificationInput {
    ClassificationInput {
        text: text.to_string(),
        requested_signals: vec!["complexity".to_string()],
        session_metadata: Default::default(),
    }
}

/// A fake runtime whose forward sleeps long enough for all concurrent
/// callers to enter the cache layer and hit the in-flight slot before the
/// designated forwarder completes (same approach as the u041 cache test).
struct SlowRuntime;

impl ClassifierRuntime for SlowRuntime {
    // The slow model forward runs in `embed`. `ServiceCore` drives the runtime
    // as `embed` then `rank`, so holding the forward open here keeps concurrent
    // callers on the in-flight slot rather than finding the result already
    // cached — which is exactly what makes coalescing deterministic.
    fn embed(&self, _input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        std::thread::sleep(Duration::from_millis(250));
        Ok(Embedding::new(vec![0.0]))
    }

    fn rank(
        &self,
        _embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        Ok(ClassificationResult {
            classifier_id: "slow".into(),
            model_revision: "rev".into(),
            tokenizer_revision: "rev".into(),
            taxonomy_revision: "rev".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "label".into(),
                score: 1.0,
            }],
        })
    }

    fn metadata(&self) -> RuntimeMetadata {
        RuntimeMetadata {
            classifier_id: "slow".into(),
            signal: "test".into(),
            model_revision: "rev".into(),
            tokenizer_revision: "rev".into(),
            taxonomy_revision: "rev".into(),
            artifact_digest: None,
        }
    }
}

/// U-081 / U-041 (AC-007/AC-012): concurrent identical misses through
/// ServiceCore record the designated forwarder as a miss and the coalesced
/// waiters as `cache_coalesced`, not `cache_hits`.
///
/// Uses a deliberately blocking fake runtime so the forward stays open
/// until all threads have entered the cache layer.
#[test]
fn coalesced_waiters_counted_in_metrics() {
    const CONCURRENCY: usize = 8;
    let metrics = Metrics::new();
    let core = Arc::new(ServiceCore::with_metrics(SlowRuntime, metrics.clone()));

    let barrier = Arc::new(Barrier::new(CONCURRENCY));
    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.classify(input("identical burst key"))
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().expect("burst must succeed");
    }

    let snap = metrics.snapshot();
    assert_eq!(
        snap.cache_hits + snap.cache_misses + snap.cache_coalesced,
        CONCURRENCY as u64,
        "all requests must be accounted for"
    );
    assert_eq!(snap.cache_misses, 1, "exactly one designated forwarder");
    assert_eq!(
        snap.cache_coalesced,
        (CONCURRENCY as u64) - 1,
        "all other threads must be counted as coalesced"
    );
    assert_eq!(snap.cache_hits, 0, "cold cache must report 0 true hits");
}

/// U-081 (AC-012): mixed workload — hit/miss/coalesced counters match the
/// known ground truth exactly.
///
/// Phases: 10 distinct misses → 100 true hits → 32 concurrent burst
/// (1 miss + 31 coalesced). Total 142 requests with no counter inflation.
#[test]
#[ignore]
fn mixed_workload_counters_match_ground_truth() {
    let metrics = Metrics::new();
    let core = Arc::new(ServiceCore::with_metrics(classifier(), metrics.clone()));

    // Phase 1: 10 distinct prompts (all true misses).
    let prompts: Vec<String> = (0..10)
        .map(|i| format!("distinct prompt number {i} about a unique subject that will not collide"))
        .collect();
    for p in &prompts {
        core.classify(input(p)).expect("miss must succeed");
    }

    // Phase 2: 100 hits on an already-cached prompt.
    for _ in 0..100 {
        core.classify(input(&prompts[0])).expect("hit must succeed");
    }

    // Phase 3: 32 concurrent requests on a new prompt (1 miss + 31 coalesced).
    let burst_text =
        "a completely new prompt that has never been seen by the classifier before now";
    let barrier = Arc::new(Barrier::new(32));
    let handles: Vec<_> = (0..32)
        .map(|_| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.classify(input(burst_text))
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }

    let snap = metrics.snapshot();
    let total = snap.cache_hits + snap.cache_misses + snap.cache_coalesced;

    // Ground truth: 10 phase-1 misses + 1 phase-3 forwarder = 11 misses,
    // 100 phase-2 hits, 31 phase-3 coalesced waits.
    let true_hits: u64 = 100;
    let true_misses: u64 = 11;
    let true_coalesced: u64 = 31;

    println!();
    println!("=== Mixed workload metrics accuracy ===");
    println!("  phase 1: 10 misses  phase 2: 100 hits  phase 3: 1+31 burst");
    println!(
        "  hits: {} (expect {true_hits})  misses: {} (expect {true_misses})  \
         coalesced: {} (expect {true_coalesced})",
        snap.cache_hits, snap.cache_misses, snap.cache_coalesced
    );
    println!(
        "  hit rate: {:.1}%",
        snap.cache_hits as f64 / total as f64 * 100.0
    );

    assert_eq!(total, 142, "all 142 requests must be accounted for");
    assert_eq!(
        snap.cache_hits, true_hits,
        "hit count must match ground truth"
    );
    assert_eq!(
        snap.cache_misses, true_misses,
        "miss count must match ground truth"
    );
    assert_eq!(
        snap.cache_coalesced, true_coalesced,
        "coalesced count must match ground truth"
    );
}
