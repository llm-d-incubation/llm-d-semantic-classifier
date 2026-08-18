//! AC-011 benchmark-runner smoke test.
//!
//! Proves the `src/bin/bench-runner.rs` binary executes a TINY 0.1 matrix
//! against the REAL classifier and emits machine-readable JSON + a stdout
//! table. Requires the pinned sensitivity model (fetched by `./hack/fetch-model`
//! via `./hack/test-parity`), so it is `#[ignore]`d by default and runs under
//! `./hack/test-parity` (`cargo test --locked -- --ignored`).

use std::process::Command;

/// The compiled bench-runner binary (Cargo sets this for integration tests).
const BIN: &str = env!("CARGO_BIN_EXE_bench-runner");
/// Default model dir the runner reads unless overridden.
const DEFAULT_MODEL_DIR: &str = "artifacts/models/sensitivity";

/// Smoke test: the runner executes a tiny matrix against the REAL model and
/// writes a JSON result, exiting 0.
#[test]
#[ignore]
fn bench_runner_executes_a_tiny_matrix_against_the_real_model() {
    // The pinned model must be present (hack/test-parity fetches it first).
    let model_dir = std::path::Path::new(DEFAULT_MODEL_DIR);
    assert!(
        model_dir.join("model.safetensors").exists(),
        "pinned model weights must be fetched before this smoke test runs"
    );

    // Run the runner with a TINY matrix so it completes quickly. A tiny
    // measured count keeps the hit-mode warmup small (max(warmup, measure)).
    let out = Command::new(BIN)
        .env("LLM_D_SC_MODEL_DIR", model_dir)
        .env("BENCH_WARMUP", "2")
        .env("BENCH_MEASURE", "3")
        .output()
        .expect("bench-runner must launch");
    assert!(
        out.status.success(),
        "bench-runner must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // It must print a JSON path and a human-readable table on stdout.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("wrote JSON results to"),
        "runner must announce its JSON output; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("== llm-d-sc benchmark runner"),
        "runner must print a human-readable table; stdout:\n{stdout}"
    );

    // The announced JSON file must exist and be valid JSON.
    let json_path = stdout
        .lines()
        .find(|l| l.contains("wrote JSON results to"))
        .and_then(|l| l.trim().strip_prefix("wrote JSON results to "))
        .map(|p| p.trim())
        .expect("runner must print the JSON path");
    let raw = std::fs::read_to_string(json_path)
        .unwrap_or_else(|e| panic!("announced JSON {json_path} unreadable: {e}"));
    let v: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("announced JSON {json_path} invalid: {e}"));
    // Manifest + a non-empty scenario list must be present.
    assert!(v.get("manifest").is_some(), "report must carry a manifest");
    let scenarios = v.get("scenarios").and_then(serde_json::Value::as_array);
    assert!(
        scenarios.map(|a| !a.is_empty()).unwrap_or(false),
        "report must carry at least one scenario"
    );
    // The manifest must carry the HOMELAB.md fields available locally.
    let manifest = v.get("manifest").expect("manifest present");
    for field in [
        "git_sha",
        "model_dir",
        "model_revision",
        "tokenizer_revision",
        "backend",
        "topology",
        "cpu_model",
        "warmup_requests",
        "measured_requests",
    ] {
        assert!(
            manifest.get(field).is_some(),
            "manifest must carry '{field}'"
        );
    }
    assert_eq!(
        manifest.get("backend").and_then(serde_json::Value::as_str),
        Some("candle"),
        "backend must be candle"
    );
    assert_eq!(
        manifest.get("topology").and_then(serde_json::Value::as_str),
        Some("loopback"),
        "topology must be loopback"
    );

    // Clean up the smoke test's artifact so it doesn't accumulate in the
    // gitignored artifacts/bench dir.
    if let Some(path) = std::path::Path::new(json_path).parent() {
        let _ = std::fs::remove_file(json_path);
        let _ = std::fs::remove_dir(path);
    }
}
