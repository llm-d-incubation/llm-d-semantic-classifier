//! Bounded inference queue for classification work.
//!
//! AC-008 requires the inference queue to be bounded and overload to be
//! explicit:
//! - U-030 the inference queue capacity is bounded (never grows without limit);
//! - U-031 a full queue returns overload / resource-exhausted rather than
//!   silently buffering unboundedly.
//!
//! The queue stands between the exact-result cache (on a miss) and the
//! classifier registry, per `specs/0.1-mvp/design.md` ("bounded scheduler").
//! Admission beyond capacity must be rejected with an explicit error, never
//! queued unboundedly.

/// The kind of error a bounded queue can produce.
///
/// AC-008 failure contract: a full queue must surface an explicit
/// resource-exhausted error rather than unboundedly buffering work.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    /// The queue is already at capacity; further work is rejected explicitly.
    ResourceExhausted,
}

/// A bounded FIFO queue of classification jobs.
///
/// AC-008 requires the inference queue to be bounded and overload to be
/// explicit: admission beyond the configured capacity is rejected with
/// [`QueueError::ResourceExhausted`], never silently buffered without limit.
#[derive(Debug)]
pub struct BoundedQueue<T> {
    capacity: usize,
    jobs: std::collections::VecDeque<T>,
}

impl<T> BoundedQueue<T> {
    /// Create a bounded queue with the given capacity.
    ///
    /// A queue may hold at most `capacity` jobs; admission beyond capacity is
    /// rejected with [`QueueError::ResourceExhausted`].
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            jobs: std::collections::VecDeque::with_capacity(capacity),
        }
    }

    /// Try to admit a job.
    ///
    /// At or over capacity the job is rejected with
    /// [`QueueError::ResourceExhausted`] and the queue does not grow.
    pub fn try_enqueue(&mut self, job: T) -> Result<(), QueueError> {
        if self.jobs.len() >= self.capacity {
            return Err(QueueError::ResourceExhausted);
        }
        self.jobs.push_back(job);
        Ok(())
    }

    /// The number of jobs currently queued.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether the queue currently holds no jobs.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The configured capacity of the queue.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedQueue, QueueError};

    #[test]
    fn u030_inference_queue_capacity_is_bounded() {
        // AC-008 (U-030): the inference queue capacity must be bounded. A
        // bounded queue must never admit more work than its configured
        // capacity; submitting beyond capacity is rejected, never grown
        // without limit.
        let capacity = 3;
        let mut queue = BoundedQueue::new(capacity);

        // Fill up to capacity: admission must succeed.
        for i in 0..capacity {
            queue
                .try_enqueue(format!("job-{i}"))
                .expect("admission under capacity must succeed");
        }
        assert_eq!(
            queue.len(),
            capacity,
            "queue length must never exceed its bounded capacity"
        );

        // At/over capacity: admission must be rejected explicitly, and the
        // queue must NOT grow beyond capacity (no unbounded buffering).
        let over = queue.try_enqueue("overflow-job".to_string());
        assert!(
            matches!(over, Err(QueueError::ResourceExhausted)),
            "admission at/over capacity must be rejected, not unboundedly queued"
        );
        assert_eq!(
            queue.len(),
            capacity,
            "capacity must be preserved under overload (no unbounded growth)"
        );
        assert_eq!(
            queue.capacity(),
            capacity,
            "the configured capacity must be reported unchanged"
        );
    }

    #[test]
    fn u031_full_queue_returns_overload_resource_exhausted() {
        // AC-008 (U-031): a full queue must return an explicit
        // overload / resource-exhausted error rather than silently buffering
        // unboundedly.
        let mut queue = BoundedQueue::new(1);
        queue
            .try_enqueue("only-job".to_string())
            .expect("the single slot admits one job");

        let err = queue
            .try_enqueue("overflow-job".to_string())
            .expect_err("a full queue must reject further work");

        assert_eq!(
            err,
            QueueError::ResourceExhausted,
            "a full queue must report resource exhausted explicitly"
        );
    }
}
