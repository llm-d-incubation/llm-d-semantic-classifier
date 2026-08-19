//! AC-008 proving test (integration): the bounded queue is IN the request path.
//!
//! ADR-0002 / AC-008 require the model forward NOT to run on a Tokio network
//! worker: a BOUNDED handoff sits between the gRPC handler and a dedicated
//! inference executor. Queue-full admission is rejected explicitly with
//! resource-exhausted; in-flight + queued work never exceeds the configured
//! bound; and the service recovers once load stops.
//!
//! I-035 (saturation rejects rather than runaway queueing): we saturate the
//! service with a deliberately slow classifier behind a small queue bound and
//! assert:
//!   - explicit resource-exhausted responses appear under saturation;
//!   - in-flight + queued work never exceeds the configured bound;
//!   - queue wait is recorded through the existing Metrics Queue stage;
//!   - the service recovers after load stops (a fresh request succeeds).

use std::time::Duration;

use llm_d_sc::classify::{
    ClassificationInput, ClassificationResult, ClassifierRuntime, ClassifyError, ClassifyStatus,
    RankedSignal, RuntimeMetadata,
};
use llm_d_sc::grpc::classify::generated;
use llm_d_sc::grpc::classify::ClassifyServiceImpl;
use llm_d_sc::metrics::Metrics;
use llm_d_sc::telemetry::Telemetry;

/// Queue bound for the saturation test: small so the queue is easy to fill.
const QUEUE_BOUND: usize = 3;
/// Slow-forward delay: keeps admitted work in-flight so the queue saturates.
const FORWARD_DELAY: Duration = Duration::from_millis(50);
/// Number of concurrent requests fired under saturation.
const SATURATION_LOAD: usize = 20;

/// A deliberately slow classifier (a slow model forward on the executor thread).
///
/// The forward sleeps, so the single dedicated inference executor stays busy and
/// the bounded queue fills under saturation.
struct SlowClassifier {
    forward_delay: Duration,
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

    fn classify(&self, input: ClassificationInput) -> Result<ClassificationResult, ClassifyError> {
        std::thread::sleep(self.forward_delay);
        let _ = input;
        Ok(ClassificationResult {
            classifier_id: "slow-classifier".to_string(),
            model_revision: "slow-model".to_string(),
            tokenizer_revision: "slow-tokenizer".to_string(),
            taxonomy_revision: "slow-taxonomy".to_string(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "sensitivity".to_string(),
                score: 1.0,
            }],
        })
    }
}

/// A saturation classify request carrying a unique context (each a distinct
/// forward, never coalesced by the cache).
fn request(i: usize) -> generated::ClassifyRequest {
    generated::ClassifyRequest {
        request_id: format!("req-sat-{i:04}"),
        session_id: "sess-sat".to_string(),
        context: format!("unique saturation context {i}"),
        signals: vec!["sensitivity".to_string()],
    }
}

/// I-035: saturation rejects rather than runaway queueing.
///
/// A real tonic client/server saturates the service with concurrent requests at
/// a slow forward behind a small bound. The assertions:
///   1. explicit resource-exhausted responses appear (overload is explicit, not
///      silently buffered unboundedly);
///   2. in-flight + queued work never exceeds the configured bound
///      (`max_admitted <= QUEUE_BOUND`);
///   3. queue wait is recorded through the existing Queue metrics stage;
///   4. the service recovers after load stops (a fresh request succeeds).
#[tokio::test]
async fn i035_saturation_rejects_rather_than_runaway_queueing() {
    let metrics = Metrics::new();
    let service = ClassifyServiceImpl::with_executor(
        SlowClassifier {
            forward_delay: FORWARD_DELAY,
        },
        Telemetry::new(),
        metrics.clone(),
        QUEUE_BOUND,
    );
    // Keep a clone to observe the executor's bound/admitted counters after the
    // service is moved into the tonic server.
    let service_handle = service.clone();
    let tonic_server = generated::classify_server::ClassifyServer::new(service);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("classify server must bind an ephemeral port");
    let addr = listener.local_addr().expect("bound addr");
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(tonic_server)
            .serve_with_incoming(incoming)
            .await
            .expect("classify server must serve")
    });

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .expect("channel must be well-formed")
        .connect()
        .await
        .expect("client must connect to the server");

    // Saturate: fire many concurrent requests at a slow forward behind a small
    // bound. The first `QUEUE_BOUND` are admitted (in-flight + queued); the rest
    // must be rejected explicitly with resource-exhausted.
    let mut tasks = Vec::new();
    for i in 0..SATURATION_LOAD {
        let mut client = generated::classify_client::ClassifyClient::new(channel.clone());
        tasks.push(tokio::spawn(
            async move { client.classify(request(i)).await },
        ));
    }
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("saturate task must complete"));
    }

    // 1) Explicit resource-exhausted responses appear under saturation.
    let exhausted = outcomes
        .iter()
        .filter(|r| matches!(r, Err(s) if s.code() == tonic::Code::ResourceExhausted))
        .count();
    assert!(
        exhausted > 0,
        "saturation must produce explicit resource-exhausted responses (got {exhausted})"
    );

    // 2) In-flight + queued work never exceeds the configured bound.
    assert!(
        service_handle.max_admitted() <= QUEUE_BOUND,
        "in-flight + queued must never exceed the configured bound (max {}, bound {})",
        service_handle.max_admitted(),
        QUEUE_BOUND
    );

    // Admitted work completed: some requests succeed with ranked signals.
    let ok_count = outcomes
        .iter()
        .filter(|r| {
            matches!(r, Ok(resp) if resp.get_ref().status == generated::ClassificationStatus::Ok as i32)
        })
        .count();
    assert!(ok_count > 0, "some admitted requests must succeed");

    // 3) Queue wait is recorded through the existing Queue metrics stage.
    let snap = metrics.snapshot();
    assert!(
        snap.queue > Duration::ZERO,
        "queue wait must be recorded through the Queue metrics stage"
    );

    // 4) Recovery: after the saturation load drains, a fresh request succeeds.
    let mut client = generated::classify_client::ClassifyClient::new(channel);
    let recovered = client
        .classify(generated::ClassifyRequest {
            request_id: "req-recovery".to_string(),
            session_id: "sess-recovery".to_string(),
            context: "post-saturation recovery request".to_string(),
            signals: vec!["sensitivity".to_string()],
        })
        .await
        .expect("service must recover after load stops");
    assert_eq!(
        recovered.into_inner().status,
        generated::ClassificationStatus::Ok as i32,
        "recovered request must return status OK"
    );

    server.abort();
}
