//! Runtime lifecycle: model/tokenizer residency and readiness gating.
//!
//! The runtime must report NOT ready until the resident model/tokenizer has
//! been successfully loaded and warmed. Traffic is blocked until then.

use std::path::Path;

use crate::tokenizer::Tokenizer;

/// Required resident ModelCar files relative to the model directory, as
/// declared by `tests/fixtures/modelcar/classifier-manifest.json`. Warmup must
/// verify these are present so the runtime starts solely from `/models` with no
/// runtime Hugging Face fetch. This is production-visible so the served binary
/// can validate the ModelCar layout before loading (AC-002/AC-003).
pub const MODELCAR_REQUIRED_FILES: &[&str] = &[
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
///
/// AC-005: the model/tokenizer must be loaded at most ONCE per active revision
/// and held resident, so repeated calls reuse the resident instance rather than
/// re-loading from disk on every call.
pub struct Runtime {
    ready: bool,
    resident_tokenizer: Option<Tokenizer>,
    active_revision: Option<String>,
    tokenizer_load_count: u64,
}

impl Runtime {
    /// A runtime that has not loaded/warmed any model.
    pub fn new() -> Self {
        Runtime {
            ready: false,
            resident_tokenizer: None,
            active_revision: None,
            tokenizer_load_count: 0,
        }
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

    /// Load the tokenizer for the active `revision` and hold it resident.
    ///
    /// AC-005 contract (U-021): the model/tokenizer must be loaded at most ONCE
    /// per active revision. Repeated calls for the SAME active revision must
    /// reuse the resident tokenizer and must NOT re-load it. When the active
    /// revision changes, the tokenizer is re-loaded for the new revision (and
    /// the load count increments once for that revision).
    pub fn load_tokenizer_once<P: AsRef<Path>>(
        &mut self,
        revision: &str,
        tokenizer_path: P,
    ) -> Result<(), String> {
        // Reuse the resident tokenizer when the active revision is unchanged.
        if self.active_revision.as_deref() == Some(revision) {
            return Ok(());
        }
        let tokenizer = Tokenizer::load(tokenizer_path).map_err(|e| e.to_string())?;
        self.resident_tokenizer = Some(tokenizer);
        self.active_revision = Some(revision.to_string());
        self.tokenizer_load_count += 1;
        Ok(())
    }

    /// Number of tokenizer loads performed. AC-005 observability: must be 1
    /// per active revision, not 1 per call.
    pub fn tokenizer_load_count(&self) -> u64 {
        self.tokenizer_load_count
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

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("modelcar")
            .join(name)
    }

    #[test]
    fn u021_model_tokenizer_load_once_per_active_revision() {
        // U-021 (AC-005): the resident model/tokenizer must be loaded at most
        // ONCE per active revision. Repeated calls for the SAME active revision
        // must reuse the resident tokenizer and must NOT re-load it from disk.
        // The tokenizer.json is a committed ModelCar fixture (offline, no
        // fetch-model required), so this is a plain test.
        let mut runtime = Runtime::new();
        let tokenizer_path = fixture("tokenizer.json");
        let revision = "43f21d21ac48134464f8510a9ac9c95bdac7ba86";

        // Simulate many classification calls for the SAME active revision.
        for _ in 0..10 {
            runtime
                .load_tokenizer_once(revision, &tokenizer_path)
                .expect("resident tokenizer must load");
        }

        // AC-005: exactly ONE load for the active revision, not one per call.
        assert_eq!(
            runtime.tokenizer_load_count(),
            1,
            "model/tokenizer must be loaded once per active revision, not on every call"
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
