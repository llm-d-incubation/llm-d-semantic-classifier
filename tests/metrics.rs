//! AC-012 proving tests (integration): queue/tokenize/forward/total latency visible.
//!
//! This slice selects the local deterministic metrics proving tests for AC-012:
//! U-080 (queue/tokenize/forward/total metrics emitted), U-081 (cache hit/miss
//! counters correct), and I-080 (latency decomposition metrics visible over a
//! real gRPC round trip). S-080 (OpenShift system evidence distinguishing
//! RTT/queue/forward) is the deployment phase and is deferred, consistent with
//! how AC-009/AC-011 deferred the cluster E2E.
//!
//! AC-012 requires the queue, tokenize, forward, and total service latency to be
//! independently visible. The pipeline in `src/classify.rs` currently measures
//! nothing and there is no `metrics` module, so these tests cannot compile until
//! a metrics registry recording per-request queue/tokenize/forward/total latency
//! and cache hit/miss counters exists — that is the expected RED for this slice.

use std::time::Duration;

use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};
use llm_d_sc::metrics::{LatencyStage, Metrics, MetricsSnapshot};

/// U-080 (AC-012): queue/tokenize/forward/total metrics are emitted.
///
/// A metrics registry must record the four latency stages independently: queue
/// wait, tokenize, model forward, and total service latency. Recording a real
/// request and snapshotting it must expose all four, each strictly positive on a
/// cache miss, and each component must be bounded by the total.
#[test]
fn u080_queue_tokenize_forward_total_metrics_emitted() {
    let metrics = Metrics::new();
    metrics.record_stage(LatencyStage::Queue, Duration::from_millis(1));
    metrics.record_stage(LatencyStage::Tokenize, Duration::from_millis(2));
    metrics.record_stage(LatencyStage::Forward, Duration::from_millis(3));
    metrics.record_stage(LatencyStage::Total, Duration::from_millis(6));

    let snap: MetricsSnapshot = metrics.snapshot();
    assert!(snap.queue > Duration::ZERO, "queue latency must be emitted");
    assert!(
        snap.tokenize > Duration::ZERO,
        "tokenize latency must be emitted"
    );
    assert!(
        snap.forward > Duration::ZERO,
        "forward latency must be emitted"
    );
    assert!(
        snap.total > Duration::ZERO,
        "total service latency must be emitted"
    );
    // The decomposition is consistent: each stage is a component of the total.
    assert!(snap.queue <= snap.total);
    assert!(snap.tokenize <= snap.total);
    assert!(snap.forward <= snap.total);
}

/// U-081 (AC-012): cache hit/miss counters are correct.
///
/// The metrics registry must count cache hits and misses independently and
/// exactly. A hit increments the hit counter, a miss increments the miss
/// counter; neither leaks across the other.
#[test]
fn u081_cache_hit_miss_counters_correct() {
    let metrics = Metrics::new();
    metrics.record_cache_hit();
    metrics.record_cache_hit();
    metrics.record_cache_miss();

    let snap: MetricsSnapshot = metrics.snapshot();
    assert_eq!(snap.cache_hits, 2, "two hits must be counted");
    assert_eq!(snap.cache_misses, 1, "one miss must be counted");
    assert_eq!(
        snap.cache_hits + snap.cache_misses,
        3,
        "hit + miss counters must partition every request"
    );
}

/// I-080 (AC-012): latency decomposition metrics are visible over a real gRPC
/// round trip.
///
/// A real classify server serves the deterministic pipeline (tokenizer ->
/// versioned cache -> single-flight -> ranker). Driving a cache miss then a
/// cache hit over the persistent channel must leave queue/tokenize/forward/total
/// latency and cache hit/miss counters visible on the server's metrics snapshot.
#[test]
fn i080_latency_decomposition_metrics_visible() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect");

    // A cache miss: unique context forces tokenize + forward.
    let miss = client
        .classify(llm_d_sc::grpc::classify::ClassifyRequest {
            request_id: "req-miss".to_string(),
            session_id: "sess-0001".to_string(),
            context: "unique sensitivity context with distinct tokens".to_string(),
            signals: vec!["sensitivity".to_string()],
        })
        .expect("cache-miss classify must succeed");
    assert!(!miss.ranked.is_empty(), "miss must return ranked signals");
    assert!(
        miss.status == llm_d_sc::grpc::classify::generated::ClassificationStatus::Ok as i32,
        "cache-miss classify must return status OK"
    );

    // A cache hit: the same context is served without tokenize/forward.
    let hit = client
        .classify(llm_d_sc::grpc::classify::ClassifyRequest {
            request_id: "req-hit".to_string(),
            session_id: "sess-0001".to_string(),
            context: "unique sensitivity context with distinct tokens".to_string(),
            signals: vec!["sensitivity".to_string()],
        })
        .expect("cache-hit classify must succeed");
    assert!(!hit.ranked.is_empty(), "hit must return ranked signals");
    assert!(
        hit.status == llm_d_sc::grpc::classify::generated::ClassificationStatus::Ok as i32,
        "cache-hit classify must return status OK"
    );

    // The server's metrics expose the latency decomposition: all four stages
    // and the cache hit/miss counters are visible.
    let snap: MetricsSnapshot = server.metrics_snapshot();
    assert!(snap.queue > Duration::ZERO, "queue latency must be visible");
    assert!(
        snap.tokenize > Duration::ZERO,
        "tokenize latency must be visible"
    );
    assert!(
        snap.forward > Duration::ZERO,
        "forward latency must be visible"
    );
    assert!(snap.total > Duration::ZERO, "total latency must be visible");
    assert!(
        snap.cache_misses >= 1,
        "the miss must be counted as a cache miss"
    );
    assert!(
        snap.cache_hits >= 1,
        "the repeated context must be counted as a cache hit"
    );
}
