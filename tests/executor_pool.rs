//! The executor worker width must produce real PARALLELISM, not just admission.
//!
//! The bounded handoff (ADR-0002 / AC-008) governs how much work is ADMITTED.
//! It says nothing about how much work executes at once. The original executor
//! spawned exactly one thread, so a bound of 32 admitted 32 requests and then
//! ran them strictly one after another: a queue wearing the costume of a
//! concurrent service. No existing test detected that, because every test
//! asserted admission behaviour.
//!
//! This test asserts the property the bound cannot: with W workers and W
//! concurrent slow forwards, wall-clock must be close to ONE forward, not W of
//! them. It fails against a single-threaded executor.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use llm_d_sc::classify::{
    ClassificationInput, ClassificationResult, ClassifierRuntime, ClassifyError, ClassifyStatus,
    Embedding, RankedSignal, RuntimeMetadata,
};
use llm_d_sc::handoff::InferenceExecutor;
use llm_d_sc::metrics::Metrics;

/// Per-forward delay: long enough that serialisation is unambiguous.
const FORWARD_DELAY: Duration = Duration::from_millis(200);
const WORKERS: usize = 4;

/// A classifier whose forward sleeps, and which records the peak number of
/// forwards running at the same moment.
struct SlowClassifier {
    in_flight: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl ClassifierRuntime for SlowClassifier {
    fn metadata(&self) -> RuntimeMetadata {
        RuntimeMetadata {
            classifier_id: "test-slow".into(),
            signal: "sensitivity".into(),
            model_revision: "test".into(),
            tokenizer_revision: "test".into(),
            taxonomy_revision: "test".into(),
            artifact_digest: None,
        }
    }

    // This test double overrides `classify` directly to control the forward
    // delay and concurrency bookkeeping; `embed`/`rank` are never reached.
    fn embed(&self, _input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        unimplemented!("SlowClassifier overrides classify directly")
    }

    fn rank(
        &self,
        _embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        unimplemented!("SlowClassifier overrides classify directly")
    }

    fn classify(&self, _input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(FORWARD_DELAY);
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(ClassificationResult {
            classifier_id: "test".into(),
            model_revision: "test".into(),
            tokenizer_revision: "test".into(),
            taxonomy_revision: "test".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "a".into(),
                score: 1.0,
            }],
        })
    }
}

#[test]
fn i090_executor_workers_run_forwards_in_parallel() {
    let peak = Arc::new(AtomicUsize::new(0));
    let classifier = SlowClassifier {
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak: peak.clone(),
    };

    let executor = InferenceExecutor::spawn_with_workers(classifier, Metrics::new(), 32, WORKERS);
    assert_eq!(
        executor.workers(),
        WORKERS,
        "configured width must be honoured"
    );

    let started = Instant::now();
    let receivers: Vec<_> = (0..WORKERS)
        .map(|i| {
            executor
                .try_enqueue(ClassificationInput {
                    text: format!("job {i}"),
                    requested_signals: vec!["sensitivity".into()],
                    session_metadata: Default::default(),
                })
                .expect("bound of 32 must admit 4 jobs")
        })
        .collect();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    for rx in receivers {
        rt.block_on(rx)
            .expect("executor must respond")
            .expect("forward must succeed");
    }
    let elapsed = started.elapsed();

    assert_eq!(
        peak.load(Ordering::SeqCst),
        WORKERS,
        "all {WORKERS} forwards must be in flight simultaneously; a peak of 1 \
         means the executor is serialising behind a single thread"
    );
    // Serialised execution would take WORKERS * FORWARD_DELAY (800ms). Allow a
    // generous ceiling so the assertion targets serialisation, not scheduler noise.
    let serialised = FORWARD_DELAY * WORKERS as u32;
    assert!(
        elapsed < serialised / 2,
        "{WORKERS} parallel forwards took {elapsed:?}; serialised execution \
         would take {serialised:?}, so this indicates no real parallelism"
    );
}

#[test]
fn i091_single_worker_executor_is_observably_serial() {
    // Control: the same harness against width 1 must show a peak of 1. Without
    // this, i070 could pass for reasons unrelated to worker width.
    let peak = Arc::new(AtomicUsize::new(0));
    let classifier = SlowClassifier {
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak: peak.clone(),
    };
    let executor = InferenceExecutor::spawn_with_workers(classifier, Metrics::new(), 32, 1);

    let receivers: Vec<_> = (0..3)
        .map(|i| {
            executor
                .try_enqueue(ClassificationInput {
                    text: format!("job {i}"),
                    requested_signals: vec!["sensitivity".into()],
                    session_metadata: Default::default(),
                })
                .expect("must admit")
        })
        .collect();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    for rx in receivers {
        rt.block_on(rx).expect("respond").expect("forward");
    }

    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "a single-worker executor must never run two forwards at once"
    );
}
