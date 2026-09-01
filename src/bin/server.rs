//! llm-d-sc service binary (Kubernetes container entrypoint).
//!
//! Binds the existing [`ClassifyServer`] on `LLM_D_SC_LISTEN` (default
//! `0.0.0.0:50051`) and reads the ModelCar mount directory from
//! `LLM_D_SC_MODEL_DIR` (default `/models`). The served pipeline is the RESIDENT
//! Candle classifier: the binary reads the ModelCar dir, validates its required
//! layout, loads tokenizer + config + safetensors, constructs the real
//! `CandleClassifier`, and runs a WARMUP FORWARD on a fixture input — only then
//! does it report READY. ANY failure leaves the service NOT ready with an
//! actionable typed error (a directory that merely exists never produces READY,
//! AC-002/AC-003). The deterministic synthetic pipeline is NOT used here; it is
//! reserved for weight-free tests.

use std::env;
use std::io;

use llm_d_sc::classify::load_and_warm_modelcar;
use llm_d_sc::grpc::classify::ClassifyServer;
use llm_d_sc::metrics::LatencyStage;

/// Default TCP listen address.
const DEFAULT_LISTEN: &str = "0.0.0.0:50051";
/// Default ModelCar mount directory.
const DEFAULT_MODEL_DIR: &str = "/models";

fn main() -> io::Result<()> {
    let listen = env::var("LLM_D_SC_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_string());
    let model_dir =
        env::var("LLM_D_SC_MODEL_DIR").unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
    if model_dir.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LLM_D_SC_MODEL_DIR must not be empty",
        ));
    }

    // Real model lifecycle: validate the ModelCar required-files layout, load
    // tokenizer + config + safetensors, build the Candle classifier, and run a
    // WARMUP FORWARD on a fixture input. ANY failure leaves the service NOT
    // ready with an actionable typed error — a directory that merely exists
    // must NOT produce READY (AC-002/AC-003).
    let classifier = load_and_warm_modelcar(&model_dir).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("llm-d-sc NOT ready: {model_dir}: {e}"),
        )
    })?;

    // Only a loaded+warmed classifier reaches here, so the server reports READY.
    let server = ClassifyServer::bind_with_classifier(&listen, classifier)?;
    eprintln!(
        "llm-d-sc: bound {listen} -> {}; ModelCar dir {model_dir}; READY (resident Candle classifier loaded and warmed)",
        server.local_addr()
    );

    // Periodically log the per-stage latency DECOMPOSITION.
    //
    // S-080 requires system evidence that distinguishes round-trip time from
    // queue and forward time. RTT is measurable from outside by any client; the
    // internal stages are not, and there is no metrics endpoint yet (tracked for
    // 0.3). Logging percentiles, not means, keeps this consistent with the rule
    // that a latency claim from an average is not evidence. Emitted only when
    // requests have actually been served, so an idle service stays quiet.
    let metrics = server.metrics();
    let interval = env::var("LLM_D_SC_METRICS_LOG_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    std::thread::Builder::new()
        .name("metrics-log".to_string())
        .spawn(move || {
            let mut last_total = 0u64;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(interval));
                let snap = metrics.snapshot();
                let served = snap.cache_hits + snap.cache_misses + snap.cache_coalesced;
                if served == last_total {
                    continue;
                }
                last_total = served;
                let stage = |s| metrics.stage_percentiles(s);
                let q = stage(LatencyStage::Queue);
                let t = stage(LatencyStage::Tokenize);
                let f = stage(LatencyStage::Forward);
                let tot = stage(LatencyStage::Total);
                eprintln!(
                    "llm-d-sc metrics: served={served} hits={} misses={} coalesced={} | \
                     queue p50={:?} p99={:?} | tokenize p50={:?} p99={:?} | \
                     forward p50={:?} p99={:?} | total p50={:?} p99={:?}",
                    snap.cache_hits,
                    snap.cache_misses,
                    snap.cache_coalesced,
                    q.p50,
                    q.p99,
                    t.p50,
                    t.p99,
                    f.p50,
                    f.p99,
                    tot.p50,
                    tot.p99
                );
            }
        })
        .expect("metrics log thread must spawn");

    // Keep the serving runtime alive for the process lifetime. The
    // `ClassifyServer` owns the Tokio runtime that serves gRPC; holding it
    // (and blocking on a channel that never receives) keeps the process up.
    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = rx.recv();
    Ok(())
}
