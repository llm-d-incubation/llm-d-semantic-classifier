//! The SERVED path must classify against a real taxonomy (AC-004 / P0-3).
//!
//! Until now `CandleClassifier` loaded the real model but ranked against four
//! synthetic orthogonal vectors from `tests/fixtures/`, and reported synthetic
//! revision metadata. The model was real; the answer was not. These tests prove
//! the gap is closed end to end: a client on the wire receives the taxonomy's
//! OWN labels and the revisions needed to reproduce the result.
//!
//! Requires fetched weights (gitignored), so these are `#[ignore]`d and run via
//! `cargo test -- --ignored` after `./hack/fetch-model --classifier complexity`.

use llm_d_sc::classify::CandleClassifier;
use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyServer};
use llm_d_sc::taxonomy::ClassifierDefinition;

fn model_dir(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("models")
        .join(name)
}

fn request(id: &str, text: &str) -> ClassifyRequest {
    ClassifyRequest {
        request_id: id.into(),
        session_id: "sess-taxonomy".into(),
        context: text.into(),
        signals: vec!["sensitivity".into()],
    }
}

/// I-072: the wire response carries the taxonomy's real labels and revisions.
#[test]
#[ignore]
fn i072_served_response_carries_real_taxonomy_labels_and_revisions() {
    let definition = ClassifierDefinition::built_in("complexity")
        .expect("complexity must be built in")
        .expect("complexity must validate");
    let expected_labels = definition.labels.clone();
    let expected_taxonomy = definition.taxonomy_revision.clone();
    let expected_model = definition.model_revision.clone();

    let classifier =
        CandleClassifier::from_modelcar_with(&model_dir("complexity"), definition)
            .expect("real model + taxonomy must load");
    let server =
        ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier).expect("server must bind");
    let mut client = ClassifyClient::connect(server.local_addr()).expect("client must connect");

    let response = client
        .classify(request("tax-0001", "Prove that the square root of two is irrational."))
        .expect("classify must succeed");

    // Every returned label must be a declared label of the taxonomy, and every
    // declared label must be ranked: a partial ranking would silently hide a
    // class from the caller.
    let mut got: Vec<String> = response.ranked.iter().map(|r| r.label.clone()).collect();
    let mut want = expected_labels.clone();
    got.sort();
    want.sort();
    assert_eq!(
        got, want,
        "the served ranking must cover exactly the taxonomy's labels, not synthetic prototypes"
    );
    assert!(
        !response.ranked.iter().any(|r| r.label.starts_with("proto-")),
        "synthetic prototype ids must never reach the wire"
    );

    // The revisions must identify the real artifact, so a result is reproducible.
    assert_eq!(response.classifier_id, "complexity");
    assert_eq!(response.taxonomy_revision, expected_taxonomy);
    assert_eq!(response.model_revision, expected_model);
    assert_ne!(
        response.taxonomy_revision, "synthetic-prototypes",
        "the served path must not report synthetic taxonomy metadata"
    );
}

/// I-073: the ranking is semantically correct on the wire, not merely populated.
///
/// A response can carry the right label set and still be noise. This drives
/// prompts whose tier is unambiguous and asserts the TOP label, so the test
/// fails if the anchors are embedded incorrectly (for example wrong pooling)
/// even though the schema still looks right.
#[test]
#[ignore]
fn i073_served_ranking_is_semantically_correct() {
    let definition = ClassifierDefinition::built_in("complexity").unwrap().unwrap();
    let classifier =
        CandleClassifier::from_modelcar_with(&model_dir("complexity"), definition).unwrap();
    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier).unwrap();
    let mut client = ClassifyClient::connect(server.local_addr()).unwrap();

    let cases = [
        ("What is the capital of Portugal?", "SIMPLE"),
        (
            "Prove by induction that the sum of the first n odd numbers is n squared.",
            "REASONING",
        ),
        (
            "Design a multi region inventory service with reconciliation, idempotent writes, and failover.",
            "COMPLEX",
        ),
    ];

    for (i, (text, expected)) in cases.iter().enumerate() {
        let response = client
            .classify(request(&format!("sem-{i:04}"), text))
            .expect("classify must succeed");
        let top = &response.ranked[0];
        assert_eq!(
            &top.label, expected,
            "\"{text}\" must rank {expected} first, got {} ({:.3})",
            top.label, top.score
        );
    }
}

/// I-074: a custom definition supplied by PATH is served, not just built-ins.
///
/// v0.1 ships three built-in taxonomies, but the anchor mechanism is the
/// product. An operator must be able to supply their own definition file
/// without rebuilding the binary; this proves that path is wired end to end.
#[test]
#[ignore]
fn i074_custom_definition_from_path_is_served() {
    let custom = std::env::temp_dir().join("llm-d-sc-custom-taxonomy.json");
    std::fs::write(
        &custom,
        r#"{
          "classifier_id":"support-desk","signal":"domain",
          "taxonomy_revision":"custom-test-v1",
          "model_repo":"cnuland/llm-d-sc-complexity","model_revision":"custom",
          "top_k":2,
          "labels":["BILLING","OUTAGE"],
          "anchors":{
            "BILLING":["I was charged twice on my card","please refund my last invoice","update my billing address"],
            "OUTAGE":["the site is down for everyone","we are seeing 500 errors in production","the service is unreachable"]
          }
        }"#,
    )
    .expect("write custom definition");

    let definition = ClassifierDefinition::resolve(custom.to_str().unwrap())
        .expect("a definition supplied by path must resolve");
    assert_eq!(definition.classifier_id, "support-desk");

    let classifier =
        CandleClassifier::from_modelcar_with(&model_dir("complexity"), definition).unwrap();
    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier).unwrap();
    let mut client = ClassifyClient::connect(server.local_addr()).unwrap();

    let response = client
        .classify(request("cust-0001", "my invoice was billed twice this month"))
        .expect("classify must succeed");
    assert_eq!(response.classifier_id, "support-desk");
    assert_eq!(response.taxonomy_revision, "custom-test-v1");
    assert_eq!(
        response.ranked[0].label, "BILLING",
        "a custom taxonomy must rank its own labels correctly"
    );

    std::fs::remove_file(&custom).ok();
}
