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
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use llm_d_sc::classify::{
    ClassificationInput, ClassificationResult, ClassifierRuntime, ClassifyError, ClassifyStatus,
    Embedding, RankedSignal, RuntimeMetadata,
};
use llm_d_sc::handoff::{InferenceExecutor, QueueAdmissionError};
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
            ranking_mode: llm_d_sc::classify::RankingMode::AnchorCosine,
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
                    context_completeness: Default::default(),
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
                    context_completeness: Default::default(),
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

struct CancellationClassifier {
    calls: Arc<AtomicUsize>,
    first_started: Arc<Barrier>,
    release_first: Arc<Barrier>,
}

impl ClassifierRuntime for CancellationClassifier {
    // This test double overrides `classify` directly to drive the barriers;
    // `embed`/`rank` are never reached.
    fn embed(&self, _input: &ClassificationInput) -> Result<Embedding, ClassifyError> {
        unimplemented!("CancellationClassifier overrides classify directly")
    }

    fn rank(
        &self,
        _embedding: &Embedding,
        _input: &ClassificationInput,
    ) -> Result<ClassificationResult, ClassifyError> {
        unimplemented!("CancellationClassifier overrides classify directly")
    }

    fn metadata(&self) -> RuntimeMetadata {
        RuntimeMetadata {
            classifier_id: "test-cancellation".into(),
            signal: "sensitivity".into(),
            model_revision: "test".into(),
            tokenizer_revision: "test".into(),
            taxonomy_revision: "test".into(),
            artifact_digest: None,
            ranking_mode: llm_d_sc::classify::RankingMode::AnchorCosine,
        }
    }

    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if input.text == "blocking" {
            self.first_started.wait();
            self.release_first.wait();
        }
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
fn i092_cancelled_queued_job_is_skipped_and_capacity_recovers() {
    let calls = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(Barrier::new(2));
    let release_first = Arc::new(Barrier::new(2));
    let classifier = CancellationClassifier {
        calls: calls.clone(),
        first_started: first_started.clone(),
        release_first: release_first.clone(),
    };
    let metrics = Metrics::new();
    let executor = InferenceExecutor::spawn_with_workers(classifier, metrics.clone(), 2, 1);

    let first = executor
        .try_enqueue(ClassificationInput {
            text: "blocking".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("first job must be admitted");
    first_started.wait();

    let cancelled = executor
        .try_enqueue(ClassificationInput {
            text: "cancelled".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("second job must be admitted behind the first");
    drop(cancelled);

    release_first.wait();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(first)
        .expect("first job must receive a response")
        .expect("first forward must succeed");

    // The cancelled job must be discarded, allowing a new job to acquire the
    // second admission permit. Retry briefly because the worker may still be
    // between the first forward and the cancelled-job dequeue.
    let deadline = Instant::now() + Duration::from_secs(1);
    let third = loop {
        if let Ok(receiver) = executor.try_enqueue(ClassificationInput {
            text: "third".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        }) {
            break receiver;
        }
        assert!(
            Instant::now() < deadline,
            "cancelled job did not release capacity"
        );
        std::thread::yield_now();
    };
    rt.block_on(third)
        .expect("third job must receive a response")
        .expect("third forward must succeed");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the cancelled queued job must not invoke the classifier"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.queued_cancelled, 1);
    assert_eq!(snapshot.queued_expired, 0);
}

#[test]
fn i093_expired_queued_job_is_skipped_and_reports_deadline() {
    let calls = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(Barrier::new(2));
    let release_first = Arc::new(Barrier::new(2));
    let classifier = CancellationClassifier {
        calls: calls.clone(),
        first_started: first_started.clone(),
        release_first: release_first.clone(),
    };
    let metrics = Metrics::new();
    let executor = InferenceExecutor::spawn_with_workers(classifier, metrics.clone(), 2, 1);

    let first = executor
        .try_enqueue(ClassificationInput {
            text: "blocking".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("first job must be admitted");
    first_started.wait();

    let expired = executor
        .try_enqueue_with_deadline(
            ClassificationInput {
                text: "expired".into(),
                requested_signals: vec!["sensitivity".into()],
                session_metadata: Default::default(),
                context_completeness: Default::default(),
            },
            Some(Instant::now() - Duration::from_millis(1)),
        )
        .expect("expired job must be admitted before the worker checks it");

    release_first.wait();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(first)
        .expect("first job must receive a response")
        .expect("first forward must succeed");
    let result = rt
        .block_on(expired)
        .expect("expired job must receive a result");

    assert!(
        matches!(result, Err(ClassifyError::RequestExpired)),
        "expired queued work must return RequestExpired, got {result:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the expired queued job must not invoke the classifier"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.queued_expired, 1);
    assert_eq!(snapshot.queued_cancelled, 0);
}

#[test]
fn u035_shutdown_rejects_new_work_and_drains_admitted_jobs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(Barrier::new(2));
    let release_first = Arc::new(Barrier::new(2));
    let classifier = CancellationClassifier {
        calls: calls.clone(),
        first_started: first_started.clone(),
        release_first: release_first.clone(),
    };
    let executor = Arc::new(InferenceExecutor::spawn_with_workers(
        classifier,
        Metrics::new(),
        2,
        1,
    ));

    let first = executor
        .try_enqueue(ClassificationInput {
            text: "blocking".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("first job must be admitted");
    first_started.wait();

    let queued = executor
        .try_enqueue(ClassificationInput {
            text: "queued".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("second job must be admitted behind the active forward");

    let shutdown = executor.shutdown_handle();
    shutdown.stop_admission();
    assert!(
        matches!(
            executor.try_enqueue(ClassificationInput {
                text: "after-shutdown".into(),
                requested_signals: vec!["sensitivity".into()],
                session_metadata: Default::default(),
                context_completeness: Default::default(),
            }),
            Err(QueueAdmissionError::ShuttingDown)
        ),
        "shutdown must close admission before draining"
    );

    let join = std::thread::spawn(move || shutdown.drain_and_join(Duration::from_secs(2)));
    release_first.wait();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(first)
        .expect("active job must receive a result")
        .expect("active job must complete");
    rt.block_on(queued)
        .expect("queued job must receive a result")
        .expect("queued job must drain");

    assert!(join.join().expect("shutdown thread must finish"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn u035_shutdown_times_out_at_grace_and_later_joins_workers() {
    let calls = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(Barrier::new(2));
    let release_first = Arc::new(Barrier::new(2));
    let classifier = CancellationClassifier {
        calls,
        first_started: first_started.clone(),
        release_first: release_first.clone(),
    };
    let executor = InferenceExecutor::spawn_with_workers(classifier, Metrics::new(), 1, 1);
    let active = executor
        .try_enqueue(ClassificationInput {
            text: "blocking".into(),
            requested_signals: vec!["sensitivity".into()],
            session_metadata: Default::default(),
            context_completeness: Default::default(),
        })
        .expect("active job must be admitted");
    first_started.wait();

    let shutdown = executor.shutdown_handle();
    let started = Instant::now();
    assert!(
        !shutdown.drain_and_join(Duration::from_millis(10)),
        "blocked forwards must report an incomplete drain"
    );
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "shutdown must respect its grace deadline"
    );

    release_first.wait();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(active)
        .expect("active job must receive a result")
        .expect("active job must complete after release");
    assert!(
        shutdown.drain_and_join(Duration::from_secs(1)),
        "a later join must reap workers after their forward completes"
    );
}
