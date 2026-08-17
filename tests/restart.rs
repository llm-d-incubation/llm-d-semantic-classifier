//! AC-013 proving test (integration): restart + complete context recomputes correctly.
//!
//! This slice selects I-045 (restart + full context recomputes correctly) for
//! AC-013. The disposable exact-result cache is in-memory (`SharedCache` inside
//! `ClassifyService`), so a restart loses it. A request carrying COMPLETE context
//! must then RECOMPUTE correctly: it is a fresh cache miss that must run a real
//! forward and return an equivalent ranked result to the one served before the
//! restart — not abstain, not serve a stale result.
//!
//! The restart is simulated by dropping the pre-restart [`ClassifyServer`] and
//! binding a fresh one (a fresh empty cache — exactly what a restarted process
//! would have). The pipeline behind the server is the deterministic tokenizer ->
//! versioned cache -> single-flight -> ranker over the committed synthetic
//! fixtures, so recomputation must be deterministic and equivalent across the
//! restart.

use llm_d_sc::grpc::classify::{ClassifyRequest, ClassifyServer};

/// A gRPC request carrying complete/full context (no weak delta, no missing
/// signal).
fn full_context_request() -> ClassifyRequest {
    ClassifyRequest {
        request_id: "req-045".to_string(),
        session_id: "sess-045".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: vec!["sensitivity".to_string()],
    }
}

/// I-045 (AC-013): restart + full context recomputes correctly.
///
/// A pre-restart server classifies a full-context input and returns ranked
/// signals. After a restart the disposable exact-result cache is gone; a fresh
/// server recomputes the same full-context input and must produce an EQUIVALENT
/// result to the pre-restart one. The restarted server's cache starts empty, so
/// the recomputation is a genuine fresh forward (a cache miss), and the
/// recomputed ranked signals are exactly equivalent to the pre-restart ones.
#[test]
fn i045_restart_full_context_recomputes_correctly() {
    // Pre-restart: a fresh server classifies a complete-context input.
    let server_a = ClassifyServer::bind("127.0.0.1:0").expect("pre-restart server must bind");
    let addr_a = server_a.local_addr();
    let mut client_a = llm_d_sc::grpc::classify::ClassifyClient::connect(addr_a)
        .expect("client must connect pre-restart");

    let before = client_a
        .classify(full_context_request())
        .expect("pre-restart classify must succeed");
    assert!(
        !before.signals.is_empty(),
        "full context must produce ranked signals, not abstain"
    );
    // The pre-restart server ran exactly one forward (one cache miss).
    let pre_snap = server_a.metrics_snapshot();
    assert_eq!(
        pre_snap.cache_misses, 1,
        "pre-restart full-context classify must be a cache miss (one forward)"
    );

    // Restart: drop the pre-restart server (its in-memory cache is lost) and
    // bind a fresh one — exactly a restarted process with an empty cache.
    drop(server_a);
    drop(client_a);
    let server_b = ClassifyServer::bind("127.0.0.1:0").expect("post-restart server must bind");
    let addr_b = server_b.local_addr();
    let mut client_b = llm_d_sc::grpc::classify::ClassifyClient::connect(addr_b)
        .expect("client must connect post-restart");

    let after = client_b
        .classify(full_context_request())
        .expect("post-restart classify must succeed");

    // The restarted server recomputed (its cache started empty), so it also ran
    // a fresh forward — the disposable cache was NOT carried across the restart.
    let post_snap = server_b.metrics_snapshot();
    assert_eq!(
        post_snap.cache_misses, 1,
        "AC-013: the restarted server must recompute (fresh forward), not serve a stale hit"
    );

    // The full-context recomputation must be equivalent to the pre-restart
    // result (deterministic pipeline, same ranked signals).
    assert_eq!(
        before.signals, after.signals,
        "AC-013: restart + complete context must recompute an equivalent result"
    );
}
