//! ModelCar artifact evidence (I-060, I-062, I-063).
//!
//! The service must start from a resident `/models` mount with no runtime
//! Hugging Face fetch, must reject an incomplete mount, and must be able to tie
//! a served result to the exact bytes it loaded.
//!
//! Requires fetched weights; run with `cargo test --test modelcar -- --ignored`.

use llm_d_sc::runtime::{modelcar_digest, Readiness, Runtime, MODELCAR_REQUIRED_FILES};

fn model_dir(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("models")
        .join(name)
}

/// I-060: the ModelCar must contain every required file, and a mount missing any
/// of them must fail rather than start degraded.
#[test]
#[ignore]
fn i060_modelcar_contains_required_files() {
    let dir = model_dir("complexity");
    for f in MODELCAR_REQUIRED_FILES {
        let path = dir.join(f);
        let meta = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("required ModelCar file {} missing: {e}", path.display()));
        assert!(
            meta.len() > 0,
            "required file {} is zero-size",
            path.display()
        );
    }

    // Each required file, removed individually, must break warmup. Asserting
    // only on a wholly empty directory would pass even if the check looked at
    // just one of the three.
    for omit in MODELCAR_REQUIRED_FILES {
        let partial =
            std::env::temp_dir().join(format!("llm-d-sc-i060-{}", omit.replace('/', "_")));
        let _ = std::fs::remove_dir_all(&partial);
        std::fs::create_dir_all(&partial).unwrap();
        for f in MODELCAR_REQUIRED_FILES {
            if f == omit {
                continue;
            }
            let dest = partial.join(f);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(dir.join(f), &dest).unwrap();
        }
        let mut runtime = Runtime::new();
        let err = runtime
            .warmup_modelcar(&partial, MODELCAR_REQUIRED_FILES)
            .expect_err(&format!("a ModelCar missing {omit} must fail warmup"));
        assert!(
            err.contains(omit),
            "the error must name the missing file {omit}, got: {err}"
        );
        assert_eq!(runtime.readiness(), Readiness::NotReady);
        std::fs::remove_dir_all(&partial).ok();
    }
}

/// I-062: the artifact digest must be recorded, stable, and content-sensitive.
#[test]
#[ignore]
fn i062_artifact_model_digest_recorded() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime.artifact_digest(),
        None,
        "no digest may be claimed before warmup"
    );
    runtime
        .warmup_modelcar(model_dir("complexity"), MODELCAR_REQUIRED_FILES)
        .expect("complexity ModelCar must warm");
    let digest = runtime
        .artifact_digest()
        .expect("digest must be recorded")
        .to_string();
    assert!(
        digest.starts_with("blake3:"),
        "digest must name its algorithm: {digest}"
    );

    // Deterministic: the same bytes must always produce the same digest.
    let again = modelcar_digest(model_dir("complexity"), MODELCAR_REQUIRED_FILES).unwrap();
    assert_eq!(digest, again, "the digest must be deterministic");

    // Distinguishing: a DIFFERENT artifact must produce a different digest,
    // otherwise the digest would pass this test while identifying nothing.
    let other = modelcar_digest(model_dir("sensitivity"), MODELCAR_REQUIRED_FILES).unwrap();
    assert_ne!(
        digest, other,
        "two different ModelCars must not share a digest"
    );

    // Content-sensitive: flipping one byte of the weights must change it.
    let tampered = std::env::temp_dir().join("llm-d-sc-i062-tampered");
    let _ = std::fs::remove_dir_all(&tampered);
    std::fs::create_dir_all(&tampered).unwrap();
    for f in MODELCAR_REQUIRED_FILES {
        let dest = tampered.join(f);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(model_dir("complexity").join(f), &dest).unwrap();
    }
    let weights = tampered.join("model.safetensors");
    let mut bytes = std::fs::read(&weights).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&weights, &bytes).unwrap();
    let tampered_digest = modelcar_digest(&tampered, MODELCAR_REQUIRED_FILES).unwrap();
    assert_ne!(
        digest, tampered_digest,
        "flipping one byte of the weights must change the digest"
    );
    std::fs::remove_dir_all(&tampered).ok();
}

/// I-063: the service must start from the resident artifact with Hugging Face
/// egress disabled.
///
/// The property is that NOTHING in the load path reaches the network. Asserting
/// it by simply loading proves little on a machine that has connectivity, so
/// this sets the offline switches that would make any fetch fail loudly, and
/// points the cache at an empty directory so a warm HF cache cannot mask a fetch.
#[test]
#[ignore]
fn i063_service_starts_from_artifact_with_hf_egress_disabled() {
    use llm_d_sc::classify::load_and_warm_modelcar;

    let empty_cache = std::env::temp_dir().join("llm-d-sc-i063-empty-hf-cache");
    let _ = std::fs::remove_dir_all(&empty_cache);
    std::fs::create_dir_all(&empty_cache).unwrap();

    // SAFETY: single-threaded test setup before any load; no other thread reads
    // these variables concurrently.
    unsafe {
        std::env::set_var("HF_HUB_OFFLINE", "1");
        std::env::set_var("TRANSFORMERS_OFFLINE", "1");
        std::env::set_var("HF_HOME", &empty_cache);
        std::env::set_var("HF_HUB_DISABLE_TELEMETRY", "1");
    }

    let classifier = load_and_warm_modelcar(model_dir("complexity"))
        .expect("the service must load and warm entirely from /models with egress disabled");

    // And it must actually serve, not merely construct.
    use llm_d_sc::classify::{ClassificationInput, ClassifierRuntime};
    let result = classifier
        .classify(ClassificationInput {
            text: "What is the capital of Norway?".to_string(),
            requested_signals: vec!["sensitivity".to_string()],
            session_metadata: Default::default(),
        })
        .expect("offline classification must succeed");
    assert!(!result.ranked.is_empty());

    unsafe {
        std::env::remove_var("HF_HUB_OFFLINE");
        std::env::remove_var("TRANSFORMERS_OFFLINE");
        std::env::remove_var("HF_HOME");
        std::env::remove_var("HF_HUB_DISABLE_TELEMETRY");
    }
    std::fs::remove_dir_all(&empty_cache).ok();
}

/// I-061: the artifact must be readable by an ARBITRARY non-root UID.
///
/// OpenShift assigns a random UID from the namespace range and never root, and
/// that UID is in no group that owns the mount. So the only permission bits that
/// matter are the OTHER bits: files need o+r and every directory on the path
/// needs o+x to be traversable. An artifact that is readable only by its owner
/// works on a developer laptop and fails in the cluster with a permission error
/// at warmup, which is the worst place to discover it.
///
/// This checks the permission bits directly rather than dropping privileges,
/// because a test process cannot setuid to an arbitrary UID without root.
#[test]
#[ignore]
#[cfg(unix)]
fn i061_artifact_readable_by_arbitrary_non_root_uid() {
    use std::os::unix::fs::PermissionsExt;

    let dir = model_dir("complexity");

    // Every directory from the model dir down must be traversable by others.
    let mut dirs = vec![dir.clone()];
    for f in MODELCAR_REQUIRED_FILES {
        if let Some(parent) = dir.join(f).parent() {
            if parent != dir {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    for d in &dirs {
        let mode = std::fs::metadata(d).unwrap().permissions().mode();
        assert!(
            mode & 0o001 != 0,
            "{} has mode {:o}; an arbitrary UID cannot traverse it (needs o+x)",
            d.display(),
            mode & 0o777
        );
        assert!(
            mode & 0o004 != 0,
            "{} has mode {:o}; an arbitrary UID cannot list it (needs o+r)",
            d.display(),
            mode & 0o777
        );
    }

    for f in MODELCAR_REQUIRED_FILES {
        let path = dir.join(f);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert!(
            mode & 0o004 != 0,
            "{} has mode {:o}; an arbitrary non-root UID cannot read it (needs o+r)",
            path.display(),
            mode & 0o777
        );
        // Weights must never be writable by others: a mount that any UID can
        // rewrite makes the I-062 digest meaningless.
        assert!(
            mode & 0o002 == 0,
            "{} has mode {:o} and is world-WRITABLE; the artifact must be read-only \
             to arbitrary UIDs or its digest guarantees nothing",
            path.display(),
            mode & 0o777
        );
    }
}
