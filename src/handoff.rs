//! Bounded handoff between the gRPC handler and a dedicated inference executor.
//!
//! AC-008 / ADR-0002: the model forward must NOT run on a Tokio network worker.
//! A BOUNDED channel (the handoff) sits between the gRPC handler and a dedicated
//! executor thread that performs the forward, returning the result via a
//! oneshot. Queue admission beyond the configured bound is rejected explicitly
//! (resource exhausted); the total of in-flight + queued work never exceeds the
//! configured bound. Queue wait is recorded through the existing [`Metrics`]
//! Queue stage.
//!
//! Deliberately NOT implemented here (0.20 per VERSIONS.md / ADR-0002): per-job
//! deadlines, queued-request cancellation, load shedding policy, graceful drain,
//! and worker-failure isolation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use crate::classify::{
    ClassificationInput, ClassificationResult, ClassifierRuntime, ClassifyError,
};
use crate::metrics::{LatencyStage, Metrics};

/// A job handed from the gRPC handler to the dedicated inference executor.
struct InferenceJob {
    input: ClassificationInput,
    /// The instant the request was admitted to the queue. Queue wait is measured
    /// from here to forward start through the existing Queue stage.
    queued_at: std::time::Instant,
    /// The oneshot the handler awaits to receive the forward result.
    respond: oneshot::Sender<Result<ClassificationResult, ClassifyError>>,
    /// Held until the forward completes so the total (in-flight + queued) bound
    /// is enforced for the job's whole lifetime.
    _permit: OwnedSemaphorePermit,
}

/// Queue admission was rejected because the bounded handoff is at capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// The bounded handoff + dedicated inference executor.
///
/// A bounded channel (the handoff) sits between the gRPC handler and a dedicated
/// executor thread that runs the model forward off Tokio network workers.
/// Admission beyond the configured `bound` is rejected explicitly
/// ([`QueueFull`] -> tonic resource-exhausted), and the total of in-flight +
/// queued work never exceeds `bound`.
pub struct InferenceExecutor<R> {
    sender: mpsc::Sender<InferenceJob>,
    /// The configured bound on total admitted (in-flight + queued) work.
    bound: usize,
    /// Permits limit total admitted (in-flight + queued) work to `bound`.
    permits: Arc<Semaphore>,
    /// Current total admitted (in-flight + queued) work, for observability.
    current: Arc<AtomicUsize>,
    /// The observed maximum of `current` — must never exceed `bound`.
    max: Arc<AtomicUsize>,
    /// The dedicated executor thread (held so it lives as long as the service).
    _thread: std::thread::JoinHandle<()>,
    _service: std::marker::PhantomData<Arc<R>>,
}

impl<R: ClassifierRuntime + Send + Sync + 'static> InferenceExecutor<R> {
    /// Spawn a dedicated executor thread performing `service`'s forwards behind
    /// a bounded handoff of `bound` total admitted (in-flight + queued) work.
    pub fn spawn(service: R, metrics: Metrics, bound: usize) -> Self {
        let service = Arc::new(service);
        let permits = Arc::new(Semaphore::new(bound));
        let current = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let (tx, mut rx) = mpsc::channel::<InferenceJob>(bound);

        let thread = std::thread::Builder::new()
            .name("inference-executor".to_string())
            .spawn({
                let metrics = metrics.clone();
                let current = current.clone();
                move || {
                    // The dedicated executor thread performs the model forward
                    // (NOT on a Tokio network worker). A job's queue wait ends
                    // when its forward begins.
                    while let Some(job) = rx.blocking_recv() {
                        metrics.record_stage(LatencyStage::Queue, job.queued_at.elapsed());
                        let result = service.classify(job.input);
                        let _ = job.respond.send(result);
                        // `job._permit` is dropped here (releasing the bound), and
                        // the admitted count is decremented once the job completes.
                        current.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })
            .expect("inference executor thread must spawn");

        Self {
            sender: tx,
            bound,
            permits,
            current,
            max,
            _thread: thread,
            _service: std::marker::PhantomData,
        }
    }

    /// Try to admit a classify job. At/over the bound, admission is rejected
    /// with [`QueueFull`] (the gRPC handler maps it to resource-exhausted).
    ///
    /// On success returns a oneshot receiver yielding the forward result.
    pub fn try_enqueue(
        &self,
        input: ClassificationInput,
    ) -> Result<oneshot::Receiver<Result<ClassificationResult, ClassifyError>>, QueueFull> {
        let (respond_tx, respond_rx) = oneshot::channel();
        // Acquire a permit for the total (in-flight + queued) bound; a full
        // bound rejects admission explicitly.
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| QueueFull)?;
        // Track the admitted count BEFORE the job is visible to the executor
        // thread. If this increment ran after `try_send`, the executor could
        // already have received the job and decremented (`fetch_sub`) before the
        // increment landed, wrapping `current` to usize::MAX and overflowing the
        // `+ 1` below. Incrementing first keeps the count balanced: the executor
        // decrements exactly once per job it processes.
        let n = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(n, Ordering::SeqCst);
        let job = InferenceJob {
            input,
            queued_at: std::time::Instant::now(),
            respond: respond_tx,
            _permit: permit,
        };
        // The bounded channel handoff; a full channel rejects admission. On
        // rejection the acquired permit and the admitted count are both released
        // (the job was never admitted).
        if self.sender.try_send(job).is_err() {
            self.current.fetch_sub(1, Ordering::SeqCst);
            return Err(QueueFull);
        }
        Ok(respond_rx)
    }

    /// The configured bound on total admitted (in-flight + queued) work.
    pub fn bound(&self) -> usize {
        self.bound
    }

    /// The observed maximum of total admitted (in-flight + queued) work. For a
    /// correct implementation this never exceeds [`InferenceExecutor::bound`].
    pub fn max_admitted(&self) -> usize {
        self.max.load(Ordering::SeqCst)
    }
}
