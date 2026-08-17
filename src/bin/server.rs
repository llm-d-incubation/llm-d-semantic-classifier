//! llm-d-sc service binary (OpenShift container entrypoint).
//!
//! Binds the existing [`ClassifyServer`] on `LLM_D_SC_LISTEN` (default
//! `0.0.0.0:50051`) and reads the ModelCar mount directory from
//! `LLM_D_SC_MODEL_DIR` (default `/models`). The served pipeline is the
//! deterministic synthetic one (tokenizer -> versioned cache -> single-flight ->
//! ranker), which needs no model forward — so no model weights are baked into
//! the image; the real model arrives via a ModelCar mount at `/models`.

use std::env;
use std::io;

use llm_d_sc::grpc::classify::ClassifyServer;

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

    // Bind and serve on a private Tokio runtime. The pipeline is the
    // deterministic synthetic one (no model forward), so no model weights are
    // loaded here; the ModelCar at `model_dir` is read by operators.
    let server = ClassifyServer::bind(&listen)?;
    eprintln!(
        "llm-d-sc: bound {listen} -> {}; ModelCar dir {model_dir}; deterministic pipeline, no model forward",
        server.local_addr()
    );

    // Keep the serving runtime alive for the process lifetime. The
    // `ClassifyServer` owns the Tokio runtime that serves gRPC; holding it
    // (and blocking on a channel that never receives) keeps the process up.
    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = rx.recv();
    Ok(())
}
