//! Real served-path integration (AC-002/AC-003/AC-004).
//!
//! Proves the PRODUCTION binary's real model lifecycle end to end: read the
//! ModelCar dir, validate the required layout, load tokenizer + config +
//! safetensors, build the Candle classifier, run a warmup forward, and only then
//! serve classify requests whose ranked signals come from the ACTUAL embedding
//! (a real Candle forward) — never the deterministic synthetic pipeline.
//!
//! This requires the fetched local model weights (gitignored), so the test is
//! `#[ignore]`d and runs under `./hack/test-parity` (`cargo test -- --ignored`)
//! after `./hack/fetch-model`.

use llm_d_sc::classify::{load_and_warm_modelcar, WARMUP_INPUT};
use llm_d_sc::grpc::classify::generated;
use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyServer};
use llm_d_sc::runtime::Readiness;

fn model_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("models")
        .join("sensitivity")
}

/// Real path end to end: server with a real model dir -> classify -> ranked
/// signals from the actual embedding.
#[test]
#[ignore]
fn real_served_path_classifies_from_actual_embedding() {
    // Full real lifecycle: validate ModelCar -> load -> warmup forward -> READY.
    let classifier =
        load_and_warm_modelcar(model_dir()).expect("real model dir must load and warm");

    // The server serves the resident Candle classifier and reports READY.
    let server =
        ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier).expect("server must bind");
    assert_eq!(
        server.readiness(),
        Readiness::Ready,
        "a loaded+warmed server must report READY"
    );

    // Classify over the wire; the ranked signals come from the ACTUAL embedding.
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect");
    let response = client
        .classify(ClassifyRequest {
            request_id: "req-real-0001".to_string(),
            session_id: "sess-real".to_string(),
            context: WARMUP_INPUT.to_string(),
            signals: vec!["sensitivity".to_string()],
        })
        .expect("real classify must succeed");

    // Ranked signals from the real forward, each finite and labeled.
    assert!(
        !response.ranked.is_empty(),
        "real path must return ranked semantic signals from the actual embedding"
    );
    assert_eq!(
        response.status,
        generated::ClassificationStatus::Ok as i32,
        "real classify must return status OK"
    );
    for signal in &response.ranked {
        assert!(
            !signal.label.is_empty(),
            "every ranked signal must carry a label"
        );
        assert!(
            signal.score.is_finite(),
            "every ranked signal must carry a finite score"
        );
    }
    // The resident classifier's versioned fingerprint is non-empty (the real
    // revisions, not the synthetic ones).
    assert!(
        !response.classifier_id.is_empty(),
        "response must carry classifier_id"
    );
    assert!(
        !response.model_revision.is_empty(),
        "response must carry model_revision"
    );
}
