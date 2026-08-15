//! Real Candle forward for the resident sensitivity model.
//!
//! AC-004 requires the pinned sensitivity model to match trusted reference
//! embedding/ranking fixtures. U-062 proves the first real-forward contract:
//! running the resident BERT model through Candle must emit an embedding whose
//! length equals the dimension declared by the ModelCar's pooling config
//! (`1_Pooling/config.json`, `word_embedding_dimension`).
//!
//! [`Embedder`] loads the bert config and weights via
//! `VarBuilder::from_mmaped_safetensors` (unsafe: memory-maps the safetensors),
//! builds `BertModel`, tokenizes with the resident [`Tokenizer`], runs a forward
//! pass, and mean-pools the sequence dimension (masked by the attention mask)
//! into the final embedding vector.

use std::fs;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert;
use serde_json::Value;

use crate::tokenizer::Tokenizer;

/// Errors produced while loading the embedder or embedding text.
#[derive(Debug)]
pub enum EmbeddingError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Candle(candle_core::Error),
    Tokenizer(crate::tokenizer::TokenizerError),
    MissingField(&'static str),
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingError::Io(e) => write!(f, "embedding config io error: {e}"),
            EmbeddingError::Json(e) => write!(f, "embedding config json error: {e}"),
            EmbeddingError::Candle(e) => write!(f, "embedding candle error: {e}"),
            EmbeddingError::Tokenizer(e) => write!(f, "embedding tokenizer error: {e}"),
            EmbeddingError::MissingField(name) => {
                write!(f, "embedding config missing field: {name}")
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

/// The embedding dimension contract declared by the resident pooling config.
///
/// Loads `word_embedding_dimension` from a ModelCar `1_Pooling/config.json` so
/// the resident model's embedding dimension is pinned to the model contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingContract {
    word_embedding_dimension: usize,
}

impl EmbeddingContract {
    /// Load the embedding dimension contract from a pooling config file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<EmbeddingContract, EmbeddingError> {
        let raw = fs::read_to_string(path).map_err(EmbeddingError::Io)?;
        let root: Value = serde_json::from_str(&raw).map_err(EmbeddingError::Json)?;
        let word_embedding_dimension = root
            .get("word_embedding_dimension")
            .and_then(Value::as_u64)
            .ok_or(EmbeddingError::MissingField("word_embedding_dimension"))?
            as usize;
        Ok(EmbeddingContract {
            word_embedding_dimension,
        })
    }

    /// The resident model's embedding dimension.
    pub fn dimension(&self) -> usize {
        self.word_embedding_dimension
    }
}

/// A resident embedder: the real Candle forward over the pinned sensitivity
/// BERT model, tokenized by the resident [`Tokenizer`].
///
/// The model weights are memory-mapped from a local `model.safetensors` (never
/// fetched at runtime) and run on the CPU in eval mode (dropout disabled), so
/// repeated embeddings of the same input are deterministic.
pub struct Embedder {
    model: bert::BertModel,
    tokenizer: Tokenizer,
}

impl Embedder {
    /// Load the embedder from the bert `config.json`, the safetensors weights,
    /// the `tokenizer.json`, and the pooling `config.json`.
    pub fn load<P: AsRef<Path>>(
        model_config: P,
        weights: P,
        tokenizer_path: P,
        pooling_config: P,
    ) -> Result<Embedder, EmbeddingError> {
        let raw = fs::read_to_string(model_config).map_err(EmbeddingError::Io)?;
        let config: bert::Config = serde_json::from_str(&raw).map_err(EmbeddingError::Json)?;

        let device = Device::Cpu;
        // SAFETY: `from_mmaped_safetensors` memory-maps the safetensors file.
        // The mapped region is owned by the returned VarBuilder/backend and stays
        // alive for the lifetime of the tensors built from it, so the mmap is
        // valid for the whole model load.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights.as_ref()], DType::F32, &device)
        }
        .map_err(EmbeddingError::Candle)?;
        let model = bert::BertModel::load(vb, &config).map_err(EmbeddingError::Candle)?;

        let tokenizer = Tokenizer::load(tokenizer_path).map_err(EmbeddingError::Tokenizer)?;
        // The pooling config pins the expected embedding dimension; validate it
        // matches the bert hidden_size so the forward emits the contracted dim.
        let contract = EmbeddingContract::load(pooling_config)?;
        if contract.dimension() != config.hidden_size {
            return Err(EmbeddingError::MissingField(
                "word_embedding_dimension != hidden_size",
            ));
        }

        Ok(Embedder { model, tokenizer })
    }

    /// Embed `text`: tokenize, run the model forward, and mean-pool the
    /// sequence dimension (masked by the attention mask) into the final
    /// embedding vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let ids = self
            .tokenizer
            .tokenize(text)
            .map_err(EmbeddingError::Tokenizer)?;
        let seq_len = ids.len();
        let device = &self.model.device;

        let input_ids =
            Tensor::from_vec(ids, (1, seq_len), device).map_err(EmbeddingError::Candle)?;
        let token_type_ids =
            Tensor::zeros((1, seq_len), DType::U32, device).map_err(EmbeddingError::Candle)?;
        let attention_mask =
            Tensor::ones((1, seq_len), DType::U32, device).map_err(EmbeddingError::Candle)?;

        let sequence = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(EmbeddingError::Candle)?;

        let pooled = mean_pool(&sequence, &attention_mask).map_err(EmbeddingError::Candle)?;
        let flat = pooled.squeeze(0).map_err(EmbeddingError::Candle)?;
        flat.to_vec1::<f32>().map_err(EmbeddingError::Candle)
    }
}

/// Masked mean-pool over the sequence dimension (dim 1), matching the
/// sentence-transformers `MeanPooling` operator: sum of non-pad token
/// embeddings divided by the number of non-pad tokens.
fn mean_pool(sequence: &Tensor, attention_mask: &Tensor) -> candle_core::Result<Tensor> {
    let mask = attention_mask.unsqueeze(2)?.to_dtype(DType::F32)?; // [1, seq_len, 1]
    let masked = sequence.broadcast_mul(&mask)?; // [1, seq_len, hidden]
    let sum = masked.sum(1)?; // [1, hidden]
    let denom = mask.sum(1)?; // [1, 1]
    sum.broadcast_div(&denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN_INPUT: &str = "this is a golden sensitivity input";

    fn artifact(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join("models")
            .join("sensitivity")
            .join(name)
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("modelcar")
            .join(name)
    }

    #[test]
    #[ignore]
    fn u062_real_candle_forward_matches_embedding_dimension() {
        // U-062 (AC-004): the resident model's real Candle forward must emit an
        // embedding of exactly the dimension declared by the ModelCar pooling
        // config (`word_embedding_dimension`). This requires the local model
        // weights (gitignored), so the test is #[ignore]d and run explicitly
        // with `-- --ignored` after `./hack/fetch-model`.
        let embedder = Embedder::load(
            artifact("config.json"),
            artifact("model.safetensors"),
            fixture("tokenizer.json"),
            artifact("1_Pooling/config.json"),
        )
        .expect("embedder must load from the fetched sensitivity model");
        let contract = EmbeddingContract::load(artifact("1_Pooling/config.json"))
            .expect("pooling config must load");
        let vec = embedder
            .embed(GOLDEN_INPUT)
            .expect("golden input must embed");
        assert_eq!(
            vec.len(),
            contract.dimension(),
            "real forward embedding length must match word_embedding_dimension"
        );
    }
}
