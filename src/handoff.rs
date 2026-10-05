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
//! Queued-request deadlines/cancellation and graceful drain are handled at the
//! handoff boundary. Load shedding policy and worker-failure isolation remain
//! separate concerns.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// Monotonic deadline inherited from the caller's gRPC timeout.
    deadline: Option<std::time::Instant>,
    /// The oneshot the handler awaits to receive the forward result.
    respond: oneshot::Sender<Result<ClassificationResult, ClassifyError>>,
    /// Held until the forward completes so the total (in-flight + queued) bound
    /// is enforced for the job's whole lifetime.
    _permit: OwnedSemaphorePermit,
}

/// Why a classify request could not be admitted to the inference executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAdmissionError {
    /// The bounded handoff is at capacity.
    Full,
    /// The executor has begun shutting down and no longer accepts work.
    ShuttingDown,
}

struct ExecutorLifecycle {
    accepting: AtomicBool,
    sender: std::sync::Mutex<Option<mpsc::Sender<InferenceJob>>>,
    threads: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}

/// Cloneable control handle used to stop admission and join the executor during
/// graceful server shutdown.
#[derive(Clone)]
pub struct ExecutorShutdownHandle {
    lifecycle: Arc<ExecutorLifecycle>,
}

impl ExecutorShutdownHandle {
    /// Reject future inference work. Any enqueue already holding the sender lock
    /// completes before this method returns and is included in the drain.
    pub fn stop_admission(&self) {
        self.lifecycle.accepting.store(false, Ordering::Release);
        // Synchronize with an enqueue that passed the first atomic check before
        // shutdown began. It either sends before this lock is acquired or sees
        // `accepting == false` and is rejected.
        drop(
            self.lifecycle
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    /// Close the handoff, drain accepted jobs, and join workers within `timeout`.
    /// Returns false if one or more forwards are still running when the timeout
    /// expires; their JoinHandles are retained so a later call can join them.
    pub fn drain_and_join(&self, timeout: Duration) -> bool {
        self.stop_admission();

        // Closing the final sender lets workers finish queued work and then
        // leave `blocking_recv`. The channel is FIFO, so accepted jobs drain
        // before workers exit.
        self.lifecycle
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        let deadline = Instant::now().checked_add(timeout);
        loop {
            let all_finished = self
                .lifecycle
                .threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .all(std::thread::JoinHandle::is_finished);

            if all_finished {
                let threads = std::mem::take(
                    &mut *self
                        .lifecycle
                        .threads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                let mut joined = true;
                for thread in threads {
                    joined &= thread.join().is_ok();
                }
                return joined;
            }

            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Environment variable overriding the executor worker width.
pub const WORKERS_ENV: &str = "LLM_D_SC_INFERENCE_WORKERS";

/// The default number of executor threads.
///
/// This default is a LATENCY choice, not a throughput knee. Measured on the
/// reference homelab (P-023, 64 concurrent misses):
///
/// | width | wall clock | forward p50 | forward p99 |
/// |-------|-----------|-------------|-------------|
/// | 4     | 150.6 ms  | 8.19 ms     | 13.3 ms     |
/// | 32    | 80.3 ms   | 32.8 ms     | 65.5 ms     |
///
/// Oversubscribing keeps improving aggregate throughput, because a CPU Candle
/// forward is itself internally threaded and the extra work packs the cores.
/// But it does so by queueing inside each forward, and p99 blows past the
/// sub-20ms budget this service exists to meet. A classifier that answers late
/// is useless to a router that has a request waiting, so the default optimises
/// the tail. Raise `LLM_D_SC_INFERENCE_WORKERS` if you are batching offline and
/// genuinely want throughput instead.
pub fn default_worker_width() -> usize {
    if let Ok(v) = std::env::var(WORKERS_ENV) {
        if let Ok(n) = v.parse::<usize>() {
            if n >= 1 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|p| p.get().min(4))
        .unwrap_or(1)
        .max(1)
}

/// The bounded handoff + dedicated inference executor.
///
/// A bounded channel (the handoff) sits between the gRPC handler and a dedicated
/// executor thread that runs the model forward off Tokio network workers.
/// Admission beyond the configured `bound` is rejected explicitly
/// ([`QueueAdmissionError::Full`] -> tonic resource-exhausted), and the total of in-flight +
/// queued work never exceeds `bound`.
pub struct InferenceExecutor<R> {
    lifecycle: Arc<ExecutorLifecycle>,
    /// The configured bound on total admitted (in-flight + queued) work.
    bound: usize,
    /// Permits limit total admitted (in-flight + queued) work to `bound`.
    permits: Arc<Semaphore>,
    /// Current total admitted (in-flight + queued) work, for observability.
    current: Arc<AtomicUsize>,
    /// The observed maximum of `current` — must never exceed `bound`.
    max: Arc<AtomicUsize>,
    /// The wrapped service, retained so its identity can be queried.
    service: Arc<R>,
    /// Number of executor threads performing forwards in parallel.
    workers: usize,
    _service: std::marker::PhantomData<Arc<R>>,
}

impl<R: ClassifierRuntime + Send + Sync + 'static> InferenceExecutor<R> {
    /// Spawn the executor with the DEFAULT worker width (see
    /// [`default_worker_width`]).
    pub fn spawn(service: R, metrics: Metrics, bound: usize) -> Self {
        Self::spawn_with_workers(service, metrics, bound, default_worker_width())
    }

    /// Spawn `workers` dedicated executor threads performing `service`'s
    /// forwards behind a bounded handoff of `bound` total admitted (in-flight +
    /// queued) work.
    ///
    /// The bound governs ADMISSION; the worker width governs PARALLELISM. They
    /// are independent: a single worker with a bound of 32 admits 32 requests
    /// and then executes them one at a time, which is a queue, not concurrency.
    ///
    /// Each worker blocks on the shared receiver behind a mutex, so exactly one
    /// worker waits for the next job while the others run forwards. Handing the
    /// job off before the forward means the lock is never held across
    /// `classify`.
    pub fn spawn_with_workers(service: R, metrics: Metrics, bound: usize, workers: usize) -> Self {
        let workers = workers.max(1);
        let service = Arc::new(service);
        let permits = Arc::new(Semaphore::new(bound));
        let current = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<InferenceJob>(bound);
        let rx = Arc::new(std::sync::Mutex::new(rx));

        let threads = (0..workers)
            .map(|i| {
                std::thread::Builder::new()
                    .name(format!("inference-executor-{i}"))
                    .spawn({
                        let metrics = metrics.clone();
                        let current = current.clone();
                        let service = service.clone();
                        let rx = rx.clone();
                        move || {
                            // The dedicated executor threads perform the model
                            // forward (NOT on a Tokio network worker). A job's
                            // queue wait ends when its forward begins.
                            loop {
                                // Scope the lock so it is released BEFORE the
                                // forward runs; otherwise the pool would
                                // serialise on the mutex and the extra workers
                                // would buy nothing.
                                let job = {
                                    let mut guard = match rx.lock() {
                                        Ok(g) => g,
                                        Err(_) => return,
                                    };
                                    guard.blocking_recv()
                                };
                                let Some(job) = job else { return };
                                let expired = job
                                    .deadline
                                    .is_some_and(|deadline| std::time::Instant::now() >= deadline);
                                // The handler owns the oneshot receiver. If it
                                // was cancelled (for example because the
                                // caller disconnected or its gRPC timeout
                                // elapsed), there is no useful result to send.
                                // Drop the queued job before invoking the
                                // classifier so cancelled work does not spend
                                // model time. Dropping the job also releases
                                // its admission permit.
                                if job.respond.is_closed() {
                                    current.fetch_sub(1, Ordering::SeqCst);
                                    if expired {
                                        metrics.record_queued_expired();
                                    } else {
                                        metrics.record_queued_cancelled();
                                    }
                                    drop(job);
                                    continue;
                                }
                                if expired {
                                    current.fetch_sub(1, Ordering::SeqCst);
                                    metrics.record_queued_expired();
                                    let _ = job.respond.send(Err(ClassifyError::RequestExpired));
                                    continue;
                                }
                                metrics.record_stage(LatencyStage::Queue, job.queued_at.elapsed());
                                let result = service.classify(job.input);
                                let _ = job.respond.send(result);
                                // `job._permit` is dropped here (releasing the
                                // bound), and the admitted count is decremented
                                // once the job completes.
                                current.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                    })
                    .expect("inference executor thread must spawn")
            })
            .collect();

        let lifecycle = Arc::new(ExecutorLifecycle {
            accepting: AtomicBool::new(true),
            sender: std::sync::Mutex::new(Some(tx)),
            threads: std::sync::Mutex::new(threads),
        });

        Self {
            lifecycle,
            bound,
            permits,
            current,
            max,
            service,
            workers,
            _service: std::marker::PhantomData,
        }
    }

    /// The number of executor threads running forwards in parallel.
    pub fn workers(&self) -> usize {
        self.workers
    }

    /// The identity of the classifier this executor runs work against.
    ///
    /// The gRPC surface needs it to validate a requested signal, and the
    /// executor owns the only handle to the service, so it forwards the query
    /// rather than making callers hold a second reference.
    pub fn metadata(&self) -> crate::classify::RuntimeMetadata {
        self.service.metadata()
    }

    /// Return a cloneable handle for server shutdown coordination.
    pub fn shutdown_handle(&self) -> ExecutorShutdownHandle {
        ExecutorShutdownHandle {
            lifecycle: self.lifecycle.clone(),
        }
    }

    /// Try to admit a classify job. At/over the bound, admission is rejected
    /// with [`QueueAdmissionError::Full`]. During shutdown, it is rejected with
    /// [`QueueAdmissionError::ShuttingDown`].
    ///
    /// On success returns a oneshot receiver yielding the forward result.
    pub fn try_enqueue(
        &self,
        input: ClassificationInput,
    ) -> Result<oneshot::Receiver<Result<ClassificationResult, ClassifyError>>, QueueAdmissionError>
    {
        self.try_enqueue_with_deadline(input, None)
    }

    /// Try to admit a classify job with an optional monotonic deadline.
    pub fn try_enqueue_with_deadline(
        &self,
        input: ClassificationInput,
        deadline: Option<std::time::Instant>,
    ) -> Result<oneshot::Receiver<Result<ClassificationResult, ClassifyError>>, QueueAdmissionError>
    {
        if !self.lifecycle.accepting.load(Ordering::Acquire) {
            return Err(QueueAdmissionError::ShuttingDown);
        }

        let (respond_tx, respond_rx) = oneshot::channel();
        // Acquire a permit for the total (in-flight + queued) bound; a full
        // bound rejects admission explicitly.
        let permit = self.permits.clone().try_acquire_owned().map_err(|_| {
            if self.lifecycle.accepting.load(Ordering::Acquire) {
                QueueAdmissionError::Full
            } else {
                QueueAdmissionError::ShuttingDown
            }
        })?;
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
            deadline,
            respond: respond_tx,
            _permit: permit,
        };
        // The bounded channel handoff; a full channel rejects admission. On
        // rejection the acquired permit and the admitted count are both released
        // (the job was never admitted).
        let send_result = {
            let sender = self
                .lifecycle
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self.lifecycle.accepting.load(Ordering::Acquire) {
                Err(QueueAdmissionError::ShuttingDown)
            } else if let Some(sender) = sender.as_ref() {
                sender.try_send(job).map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => QueueAdmissionError::Full,
                    mpsc::error::TrySendError::Closed(_) => QueueAdmissionError::ShuttingDown,
                })
            } else {
                Err(QueueAdmissionError::ShuttingDown)
            }
        };
        if let Err(error) = send_result {
            self.current.fetch_sub(1, Ordering::SeqCst);
            return Err(error);
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
