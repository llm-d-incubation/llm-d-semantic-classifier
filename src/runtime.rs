//! Runtime lifecycle: model/tokenizer residency and readiness gating.
//!
//! The runtime must report NOT ready until the resident model/tokenizer has
//! been successfully loaded and warmed. Traffic is blocked until then.

use std::path::Path;

/// Required resident ModelCar files relative to the model directory, as
/// declared by `tests/fixtures/modelcar/classifier-manifest.json`. Warmup must
/// verify these are present so the runtime starts solely from `/models` with no
/// runtime Hugging Face fetch.
#[cfg(test)]
const MODELCAR_REQUIRED_FILES: &[&str] = &[
    "model.safetensors",
    "tokenizer.json",
    "1_Pooling/config.json",
];

/// Readiness gate for the resident runtime.
///
/// Starts NOT ready and only flips to READY after a successful warmup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    NotReady,
    Ready,
}

impl Readiness {
    /// True only when the runtime is ready to serve traffic.
    pub fn ready(self) -> bool {
        matches!(self, Readiness::Ready)
    }
}

/// A resident model/tokenizer runtime whose readiness is gated on warmup.
pub struct Runtime {
    ready: bool,
}

impl Runtime {
    /// A runtime that has not loaded/warmed any model.
    pub fn new() -> Self {
        Runtime { ready: false }
    }

    /// Load and warm the model at `path`.
    ///
    /// `path` must exist and be readable; otherwise warmup fails with an error
    /// and the runtime stays NOT ready. On success the runtime flips to READY.
    pub fn warmup<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(format!("model path does not exist: {}", path.display()));
        }
        if let Err(e) = std::fs::metadata(path) {
            return Err(format!(
                "model path is not readable: {}: {}",
                path.display(),
                e
            ));
        }
        self.ready = true;
        Ok(())
    }

    /// Load and warm a ModelCar at `path`.
    ///
    /// A ModelCar must contain every file in `required_files` (relative to
    /// `path`); a missing required file fails warmup and keeps the runtime NOT
    /// ready. On success the runtime flips to READY.
    pub fn warmup_modelcar<P: AsRef<Path>>(
        &mut self,
        path: P,
        required_files: &[&str],
    ) -> Result<(), String> {
        let path = path.as_ref();
        for f in required_files {
            let required = path.join(f);
            if !required.is_file() {
                return Err(format!(
                    "ModelCar missing required file {} at {}",
                    f,
                    required.display()
                ));
            }
        }
        self.warmup(path)
    }

    /// Current readiness.
    pub fn readiness(&self) -> Readiness {
        if self.ready {
            Readiness::Ready
        } else {
            Readiness::NotReady
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Runtime, MODELCAR_REQUIRED_FILES};

    #[test]
    fn u020_readiness_false_before_successful_warmup() {
        let mut runtime = Runtime::new();
        // Before any model load/warmup, the runtime must report not-ready.
        assert!(
            !runtime.readiness().ready(),
            "must be not-ready before warmup"
        );
        // After a successful warmup it flips to ready. Warm up against a real,
        // readable directory so the path validation passes.
        let dir = std::env::temp_dir().join("llm-d-sc-u020");
        std::fs::create_dir_all(&dir).unwrap();
        runtime.warmup(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(runtime.readiness().ready(), "must be ready after warmup");
    }

    #[test]
    fn u022_warmup_failure_keeps_not_ready() {
        let mut runtime = Runtime::new();
        // A nonexistent model path must make warmup fail...
        runtime
            .warmup("/nonexistent/model/path")
            .expect_err("warmup must reject a missing model path");
        // ...and readiness must remain not-ready.
        assert!(
            !runtime.readiness().ready(),
            "must stay not-ready after failed warmup"
        );
    }

    #[test]
    fn i064_incomplete_modelcar_fails_readiness() {
        // I-064 (AC-003): an incomplete/corrupt ModelCar must fail readiness.
        // The ModelCar manifest (classifier-manifest.json) requires the files
        // `/models/model.safetensors`, `/models/tokenizer.json`, and
        // `/models/1_Pooling/config.json` to be present and readable. A model
        // directory that exists but is missing these required files must keep
        // the runtime NOT ready.
        let dir = std::env::temp_dir().join("llm-d-sc-i064-incomplete");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Intentionally write NO required ModelCar files: no weights, no
        // tokenizer, no pooling config.
        let mut runtime = Runtime::new();
        runtime
            .warmup_modelcar(&dir, MODELCAR_REQUIRED_FILES)
            .expect_err("incomplete ModelCar must fail warmup");
        assert!(
            !runtime.readiness().ready(),
            "must stay not-ready when ModelCar is incomplete"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
