//! The runtime must describe itself, and everything that needs its identity must
//! ask (U-110..U-113, I-095).
//!
//! Three separate defects shared one root cause: the gRPC surface hardcoded a
//! signal name, the result cache keyed on module constants, and results reported
//! fixture revisions. Each was invisible to a large green suite because every
//! test happened to pass the one string the hardcoded check expected. These
//! tests fail if any of that is reintroduced.
//!
//! Requires fetched weights; run with `cargo test --test runtime_metadata -- --ignored`.

use llm_d_sc::cache::CacheKey;
use llm_d_sc::classify::{CandleClassifier, ClassifierRuntime, ServiceCore};
use llm_d_sc::taxonomy::ClassifierDefinition;

fn classifier(name: &str) -> CandleClassifier {
    let def = ClassifierDefinition::built_in(name).unwrap().unwrap();
    CandleClassifier::from_modelcar_with(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts/models")
            .join(name),
        def,
    )
    .unwrap_or_else(|e| panic!("{name} must load: {e}"))
}

/// U-110: metadata reports the REAL identity, never fixture placeholders.
#[test]
#[ignore]
fn u110_metadata_reports_real_identity_not_fixtures() {
    let meta = classifier("complexity").metadata();
    assert_eq!(meta.classifier_id, "complexity");
    assert_eq!(meta.signal, "complexity");

    // The specific strings that used to be reported on the real path.
    for (field, value) in [
        ("classifier_id", &meta.classifier_id),
        ("model_revision", &meta.model_revision),
        ("tokenizer_revision", &meta.tokenizer_revision),
        ("taxonomy_revision", &meta.taxonomy_revision),
    ] {
        for bad in [
            "sensitivity-synthetic",
            "synthetic-for-mechanics-only",
            "tokenizer-fixture",
            "synthetic-prototypes",
        ] {
            assert_ne!(
                value, bad,
                "{field} must not report the fixture value '{bad}' on the real path"
            );
        }
    }

    // The digest must be present and content-derived, so a result is tied to
    // bytes rather than to a revision a stale mount may not match.
    let digest = meta
        .artifact_digest
        .as_deref()
        .expect("a classifier loaded from a ModelCar must carry its artifact digest");
    assert!(
        digest.starts_with("blake3:"),
        "digest must name its algorithm: {digest}"
    );
}

/// U-111: two different taxonomies must not share a cache namespace.
///
/// The cache key was built from module constants, so every backend in a process
/// keyed into ONE namespace. It did not corrupt results only because there is
/// one classifier per process today, which is exactly the kind of assumption
/// that breaks silently later.
#[test]
#[ignore]
fn u111_different_classifiers_produce_different_cache_keys() {
    let a = classifier("complexity").metadata();
    let b = classifier("sensitivity").metadata();
    assert_ne!(a.cache_identity(), b.cache_identity());

    let (a1, a2, a3, a4) = a.cache_identity();
    let (b1, b2, b3, b4) = b.cache_identity();
    let text = "an identical prompt sent to both classifiers";
    assert_ne!(
        CacheKey::new(a1, a2, a3, a4, text),
        CacheKey::new(b1, b2, b3, b4, text),
        "the same text under two taxonomies must not collide in the cache"
    );
}

/// U-112: a change in ANY identity component changes the key.
#[test]
#[ignore]
fn u112_every_identity_component_participates_in_the_cache_key() {
    let m = classifier("complexity").metadata();
    let (c, mo, tk, tx) = m.cache_identity();
    let text = "a stable prompt";
    let base = CacheKey::new(c, mo, tk, tx, text);

    for (label, key) in [
        ("classifier_id", CacheKey::new("other", mo, tk, tx, text)),
        ("model/digest", CacheKey::new(c, "other", tk, tx, text)),
        (
            "tokenizer_revision",
            CacheKey::new(c, mo, "other", tx, text),
        ),
        ("taxonomy_revision", CacheKey::new(c, mo, tk, "other", text)),
    ] {
        assert_ne!(base, key, "changing {label} must change the cache key");
    }
}

/// U-113: ServiceCore reports the WRAPPED backend's identity, not its own.
#[test]
#[ignore]
fn u113_service_core_delegates_identity_to_the_backend() {
    let direct = classifier("complexity").metadata();
    let wrapped = ServiceCore::new(classifier("complexity")).metadata();
    assert_eq!(direct, wrapped, "wrapping must not change identity");
}

/// I-095: the gRPC surface validates a requested signal against the LOADED
/// runtime, accepting the one it serves and rejecting one it does not.
#[test]
#[ignore]
fn i095_grpc_validates_requested_signal_against_the_loaded_runtime() {
    use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyServer};

    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier("complexity"))
        .expect("server must bind");
    let mut client = ClassifyClient::connect(server.local_addr()).expect("client must connect");

    let req = |signals: Vec<String>| ClassifyRequest {
        request_id: "sig-check".into(),
        session_id: "sess".into(),
        context: "What is the capital of Peru?".into(),
        signals,
    };

    // The signal this instance actually serves is accepted.
    client
        .classify(req(vec!["complexity".into()]))
        .expect("the served signal must be accepted");

    // No constraint is accepted: a remote caller need not know the taxonomy.
    client
        .classify(req(Vec::new()))
        .expect("an empty signal list must be accepted");

    // A signal this instance does NOT serve is rejected explicitly. Before the
    // runtime described itself this was inverted: 'sensitivity' was accepted by
    // a complexity service and 'complexity' was rejected.
    let err = client
        .classify(req(vec!["sensitivity".into()]))
        .expect_err("a signal this instance does not serve must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("complexity"),
        "the error must name the signal actually served, got: {}",
        err.message()
    );
}
