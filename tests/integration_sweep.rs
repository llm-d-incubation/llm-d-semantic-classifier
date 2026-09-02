//! Local integration sweep: the 0.1 integration IDs that need real weights.
//!
//! These were pending because the classifier ranked against synthetic prototypes,
//! so a "regulated-like golden fixture" had nothing meaningful to assert. With
//! artifact-backed taxonomies they are now real behavioural tests.
//!
//! Requires fetched weights (gitignored):
//!   ./hack/fetch-model --classifier complexity
//!   ./hack/fetch-model --classifier sensitivity
//! then `cargo test --test integration_sweep -- --ignored`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use llm_d_sc::classify::{
    load_and_warm_modelcar, CandleClassifier, ClassificationInput, ClassifierRuntime, ServiceCore,
};
use llm_d_sc::runtime::{Readiness, Runtime, MODELCAR_REQUIRED_FILES};
use llm_d_sc::taxonomy::ClassifierDefinition;

fn model_dir(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("models")
        .join(name)
}

fn input(text: &str) -> ClassificationInput {
    ClassificationInput {
        text: text.to_string(),
        requested_signals: Vec::new(),
        session_metadata: Default::default(),
        context_completeness: Default::default(),
    }
}

fn sensitivity_classifier() -> CandleClassifier {
    let def = ClassifierDefinition::built_in("sensitivity")
        .expect("built in")
        .expect("validates");
    CandleClassifier::from_modelcar_with(&model_dir("sensitivity"), def)
        .expect("sensitivity artifact must load")
}

/// Rank `text` and return the labels in ranked order.
fn ranked(clf: &CandleClassifier, text: &str) -> Vec<String> {
    clf.classify(input(text))
        .expect("classify must succeed")
        .ranked
        .into_iter()
        .map(|s| s.id)
        .collect()
}

// ---------------------------------------------------------------- readiness

/// I-010: the runtime must NOT report ready before the artifact is warmed.
#[test]
#[ignore]
fn i010_server_not_ready_before_artifact_warmup() {
    let runtime = Runtime::new();
    assert_eq!(
        runtime.readiness(),
        Readiness::NotReady,
        "a fresh runtime must not claim readiness"
    );

    // A directory that exists but lacks the ModelCar files must also stay not-ready.
    let empty = std::env::temp_dir().join("llm-d-sc-i010-empty");
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    let mut runtime = Runtime::new();
    runtime
        .warmup_modelcar(&empty, MODELCAR_REQUIRED_FILES)
        .expect_err("an empty directory must not warm up");
    assert_eq!(runtime.readiness(), Readiness::NotReady);
    std::fs::remove_dir_all(&empty).ok();
}

/// I-011: readiness flips to true only after a real load and warmup forward.
#[test]
#[ignore]
fn i011_readiness_true_after_warmup() {
    let mut runtime = Runtime::new();
    runtime
        .warmup_modelcar(model_dir("complexity"), MODELCAR_REQUIRED_FILES)
        .expect("a complete ModelCar must warm up");
    assert_eq!(runtime.readiness(), Readiness::Ready);

    // And the full production lifecycle (load + warmup forward) must succeed.
    load_and_warm_modelcar(model_dir("complexity")).expect("real lifecycle must reach READY");
}

/// I-012: repeated classify calls must not reload the model or tokenizer.
#[test]
#[ignore]
fn i012_repeated_calls_do_not_reload_model_or_tokenizer() {
    let clf = sensitivity_classifier();
    let tokenizer_calls = clf.tokenizer_call_counter();
    let forwards = clf.forward_call_counter();

    for i in 0..8 {
        clf.classify(input(&format!("distinct prompt number {i}")))
            .expect("classify must succeed");
    }

    // One tokenize and one forward PER REQUEST is correct. The property under
    // test is that the resident model is reused: if the artifact were reloaded
    // per call, load time would dominate and the counters would still read 8,
    // so the counters alone cannot prove it. Residency is proven structurally
    // (the Embedder is owned by the classifier and `from_modelcar_*` is the only
    // constructor that touches disk) and behaviourally by the latency below.
    assert_eq!(tokenizer_calls.load(Ordering::SeqCst), 8);
    assert_eq!(forwards.load(Ordering::SeqCst), 8);

    let start = std::time::Instant::now();
    clf.classify(input("one more distinct prompt")).unwrap();
    let per_call = start.elapsed();
    assert!(
        per_call < std::time::Duration::from_millis(400),
        "a warm call took {per_call:?}; a per-call model reload would cost far more"
    );
}

// ------------------------------------------------------- artifact + fixtures

/// I-020: the real sensitivity artifact loads and reports its own identity.
#[test]
#[ignore]
fn i020_real_sensitivity_artifact_loads() {
    let clf = sensitivity_classifier();
    let result = clf
        .classify(input("what is the capital of Denmark"))
        .unwrap();
    assert_eq!(result.classifier_id, "sensitivity");
    assert_eq!(result.ranked.len(), 5, "all five tiers must be ranked");
    assert!(
        result.ranked.iter().all(|s| s.score.is_finite()),
        "every score must be finite"
    );
    // Ranking must be sorted descending: callers rely on ranked[0] being the top.
    for w in result.ranked.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "ranking must be descending: {:?}",
            result.ranked
        );
    }
}

/// I-021: a public-like prompt ranks PUBLIC first.
#[test]
#[ignore]
fn i021_public_like_golden_fixture() {
    let clf = sensitivity_classifier();
    for text in [
        "What causes the northern lights?",
        "Explain how a bicycle gear ratio works.",
    ] {
        assert_eq!(
            ranked(&clf, text)[0],
            "PUBLIC",
            "\"{text}\" must rank PUBLIC first"
        );
    }
}

/// I-022: a regulated-like prompt ranks REGULATED first.
#[test]
#[ignore]
fn i022_regulated_like_golden_fixture() {
    let clf = sensitivity_classifier();
    for text in [
        "Summarise this patient's medical record including diagnosis and prescriptions.",
        "Extract the national insurance numbers from this employee spreadsheet.",
    ] {
        assert_eq!(
            ranked(&clf, text)[0],
            "REGULATED",
            "\"{text}\" must rank REGULATED first"
        );
    }
}

/// I-023: a credential-bearing prompt ranks NEVER_EGRESS first.
#[test]
#[ignore]
fn i023_never_egress_like_golden_fixture() {
    let clf = sensitivity_classifier();
    for text in [
        "Here is our production database password, build the connection string.",
        "This is the private signing key for our releases, explain the format.",
    ] {
        assert_eq!(
            ranked(&clf, text)[0],
            "NEVER_EGRESS",
            "\"{text}\" must rank NEVER_EGRESS first"
        );
    }
}

/// I-024: adversarial prompts must not be dragged to a tier by vocabulary alone.
///
/// "What makes a password strong?" contains credential vocabulary but discloses
/// nothing. The failure mode this guards against is a classifier that keys on
/// the WORD rather than the disclosure, which would over-restrict routing.
#[test]
#[ignore]
fn i024_adversarial_borderline_fixture_expected_ordering() {
    let clf = sensitivity_classifier();
    for text in [
        "What makes a password strong, in general terms?",
        "What is the purpose of a private key in public key cryptography?",
    ] {
        let order = ranked(&clf, text);
        assert_ne!(
            order[0], "NEVER_EGRESS",
            "\"{text}\" discloses no secret and must not rank NEVER_EGRESS first (got {order:?})"
        );
    }
}

// -------------------------------------------------------------------- cache

/// I-030: a warmed cache hit must invoke ZERO tokenizer calls and ZERO forwards.
#[test]
#[ignore]
fn i030_warm_cache_hit_invokes_zero_model_forwards() {
    let clf = sensitivity_classifier();
    // Clone the counters BEFORE the classifier moves into the core.
    let tokenizer_calls = clf.tokenizer_call_counter();
    let forwards = clf.forward_call_counter();
    let core = ServiceCore::new(clf);

    core.classify(input("warm this key"))
        .expect("miss must succeed");
    let after_miss_t = tokenizer_calls.load(Ordering::SeqCst);
    let after_miss_f = forwards.load(Ordering::SeqCst);
    assert_eq!(after_miss_f, 1, "the first call must be a real forward");

    for _ in 0..5 {
        core.classify(input("warm this key"))
            .expect("hit must succeed");
    }
    assert_eq!(
        tokenizer_calls.load(Ordering::SeqCst),
        after_miss_t,
        "a cache hit must not tokenize"
    );
    assert_eq!(
        forwards.load(Ordering::SeqCst),
        after_miss_f,
        "a cache hit must not run a model forward"
    );
}

/// I-031: many simultaneous identical misses must coalesce to a bounded number
/// of forwards, not one per caller.
#[test]
#[ignore]
fn i031_simultaneous_same_key_misses_have_bounded_forward_count() {
    const CALLERS: usize = 100;
    let clf = sensitivity_classifier();
    let forwards = clf.forward_call_counter();
    let core = Arc::new(ServiceCore::new(clf));

    // Real threads on the same key, released together, so the misses genuinely
    // race. Driving them sequentially would be answered by the cache and would
    // prove nothing about single-flight.
    let barrier = Arc::new(std::sync::Barrier::new(CALLERS));
    let handles: Vec<_> = (0..CALLERS)
        .map(|_| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.classify(input("stampede key")).is_ok()
            })
        })
        .collect();
    let ok = handles
        .into_iter()
        .filter(|h| !h.is_finished() || true)
        .map(|h| h.join().expect("caller thread must not panic"))
        .filter(|b| *b)
        .count();
    assert_eq!(ok, CALLERS, "every caller must receive a result");

    let n = forwards.load(Ordering::SeqCst);
    assert!(
        n >= 1 && n < CALLERS as u64 / 10,
        "{CALLERS} simultaneous identical misses ran {n} forwards; single-flight \
         coalescing must keep this far below one per caller"
    );
}
