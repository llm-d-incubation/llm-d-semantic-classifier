//! AC-014 proving tests: default telemetry contains no raw prompt/session text.
//!
//! AC-014 requires the service's default telemetry (logs, metrics, trace
//! capture) to never contain raw prompt or session text. Per
//! `specs/0.1-mvp/test-plan.md`, AC-014 maps to U-085 (raw prompt absent from
//! default logs/metrics) and I-085 (trace capture has IDs/hashes but no raw
//! prompt). No Kubernetes system test is mapped to AC-014, so this slice covers
//! the local deterministic mechanics only.
//!
//! The service currently has NO telemetry/logging/trace surface: `src/lib.rs`
//! registers no `telemetry` module, `src/metrics.rs` records only latency
//! stages and cache hit/miss counters (no labels, nothing from the request),
//! `ClassifyServer` exposes no `trace_capture`, and the classify pipeline
//! records no request telemetry. These tests reference a proposed
//! `llm_d_sc::telemetry` recorder that must emit default output containing
//! request IDs and context/session hashes but NEVER raw prompt or session text
//! — that is the expected RED for this slice.

use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};
use llm_d_sc::telemetry::{RequestEvent, Telemetry, TraceEvent};

/// U-085 (AC-014): raw prompt/session text is absent from default logs/metrics.
///
/// A telemetry recorder records a request event carrying a request id, a
/// session id, and the context text. The default serialized logs/metrics output
/// must surface the request id and a context hash but never the raw prompt text
/// or the raw session text.
#[test]
fn u085_raw_prompt_absent_from_default_logs_metrics() {
    let telemetry = Telemetry::new();
    telemetry.record_request(RequestEvent {
        request_id: "req-085".to_string(),
        session_id: "sess-top-secret".to_string(),
        context: "this RAW secret prompt must never appear in default telemetry".to_string(),
    });

    let out = telemetry.default_output();
    assert!(out.contains("req-085"), "request id must appear");
    assert!(
        out.contains("ctx_"),
        "a context hash (ctx_...) must appear in default telemetry"
    );
    assert!(
        !out.contains("RAW secret prompt"),
        "default telemetry must not contain the raw prompt text"
    );
    assert!(
        !out.contains("sess-top-secret"),
        "default telemetry must not contain raw session text"
    );
}

/// I-085 (AC-014): trace capture has IDs/hashes but no raw prompt.
///
/// Driving a real classify request over the persistent gRPC channel must leave a
/// captured trace containing the request id and a context hash, and no trace
/// event may contain the raw prompt text or the raw session text.
#[test]
fn i085_trace_capture_has_ids_hashes_no_raw_prompt() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect");

    client
        .classify(llm_d_sc::grpc::classify::ClassifyRequest {
            request_id: "req-085".to_string(),
            session_id: "sess-trace-secret".to_string(),
            context: "a TRACE secret prompt that must stay out of traces".to_string(),
            signals: vec!["sensitivity".to_string()],
        })
        .expect("classify must succeed");

    let trace: Vec<TraceEvent> = server.trace_capture();
    assert!(
        trace.iter().any(|ev| ev.request_id == "req-085"),
        "trace must capture the request id"
    );
    for ev in trace {
        assert!(
            !ev.context_hash.is_empty(),
            "trace event must carry a context hash"
        );
        assert!(
            !ev.context_hash.contains("TRACE secret prompt"),
            "trace context must be a hash, never the raw prompt"
        );
        assert!(
            !ev.session_hash.contains("sess-trace-secret"),
            "trace session must be a hash, never the raw session text"
        );
    }
}
