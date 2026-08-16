//! Tokenizer residency and golden token-ID parity.
//!
//! AC-004 requires the pinned sensitivity model to match trusted reference
//! embedding/ranking fixtures. Tokenization is the first deterministic stage:
//! the resident tokenizer must reproduce the exact token IDs of the pinned
//! Python reference for the golden inputs.
//!
//! [`Tokenizer`] is a thin resident wrapper around the official HuggingFace
//! `tokenizers` crate. It loads a `tokenizer.json` in the HF `tokenizers`
//! library format and delegates all of normalization, pre-tokenization,
//! wordpiece, template processing, and truncation to that crate, so parity with
//! the pinned reference is exact by construction. It is resident: it reads only
//! the committed ModelCar fixture and makes no network call.

use std::path::Path;

/// A resident BERT tokenizer, backed by the official HuggingFace `tokenizers`
/// crate.
///
/// Tokenization follows the pinned reference (HuggingFace `tokenizers` library)
/// so downstream pooling and ranking parity can be proven against the same
/// reference fixtures.
pub struct Tokenizer {
    inner: tokenizers::Tokenizer,
}

/// Errors produced while loading or tokenizing.
#[derive(Debug)]
pub enum TokenizerError {
    Tokenizers(tokenizers::Error),
}

impl std::fmt::Display for TokenizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenizerError::Tokenizers(e) => write!(f, "tokenizer error: {e}"),
        }
    }
}

impl std::error::Error for TokenizerError {}

impl Tokenizer {
    /// Load a tokenizer from a `tokenizer.json` (tokenizers library format).
    ///
    /// The fixture's `truncation.max_length` (matching the pinned reference) is
    /// applied by the crate when the file is deserialized, so over-length
    /// inputs are capped deterministically exactly as the reference does.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Tokenizer, TokenizerError> {
        let inner = tokenizers::Tokenizer::from_file(path).map_err(TokenizerError::Tokenizers)?;
        Ok(Tokenizer { inner })
    }

    /// Tokenize a single sequence, returning token IDs including `[CLS]` and
    /// `[SEP]` (matching the reference `TemplateProcessing`). Over-length inputs
    /// are truncated to the fixture's `max_length` by the crate.
    pub fn tokenize(&self, text: &str) -> Result<Vec<u32>, TokenizerError> {
        let encoding = self
            .inner
            .encode(text, true)
            .map_err(TokenizerError::Tokenizers)?;
        Ok(encoding.get_ids().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const GOLDEN_INPUT: &str = "this is a golden sensitivity input";

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("modelcar")
            .join(name)
    }

    fn load_golden_token_ids() -> Vec<u32> {
        let raw = std::fs::read_to_string(fixture("golden-token-ids.json"))
            .expect("golden fixture readable");
        let v: Value = serde_json::from_str(&raw).expect("golden fixture valid json");
        v["token_ids"]
            .as_array()
            .expect("golden token_ids array")
            .iter()
            .map(|x| x.as_u64().expect("token id numeric") as u32)
            .collect()
    }

    #[test]
    fn u060_tokenizer_golden_token_ids_match_trusted_reference() {
        // U-060 (AC-004): the resident tokenizer must reproduce the trusted
        // reference token IDs for the golden input so downstream pooling and
        // ranking parity can be proven against the same pinned reference. The
        // tokenizer is loaded from the committed ModelCar fixture (no network),
        // and the expected IDs are the committed golden fixture produced by the
        // pinned Python reference (HuggingFace `tokenizers` library).
        let tokenizer = Tokenizer::load(fixture("tokenizer.json"))
            .expect("tokenizer must load from the committed ModelCar fixture");
        let ids = tokenizer
            .tokenize(GOLDEN_INPUT)
            .expect("golden input must tokenize");
        assert_eq!(
            ids,
            load_golden_token_ids(),
            "tokenizer token IDs must match the trusted reference"
        );
    }

    #[test]
    fn u066_max_length_truncation_deterministic() {
        // U-066 (AC-004): when the input exceeds the fixture's configured max
        // length, the resident tokenizer must truncate deterministically and
        // exactly as the pinned reference does: total length (including [CLS]
        // and [SEP]) is capped at `truncation.max_length`, the [CLS] and [SEP]
        // special tokens are preserved, and excess tokens are dropped from the
        // right (the tail), not the head.
        let tokenizer = Tokenizer::load(fixture("tokenizer.json"))
            .expect("tokenizer must load from the committed ModelCar fixture");

        // The fixture pins truncation.max_length; the resident tokenizer must
        // honor that same value so parity with the reference is maintained.
        let raw =
            std::fs::read_to_string(fixture("tokenizer.json")).expect("tokenizer fixture readable");
        let root: Value = serde_json::from_str(&raw).expect("tokenizer fixture valid json");
        let max_length = root["truncation"]["max_length"]
            .as_u64()
            .expect("fixture truncation.max_length present") as usize;

        // Far more than max_length tokens: 400 "golden" words plus a trailing
        // marker that must be dropped by right truncation.
        let mut long_input = "golden ".repeat(400);
        long_input.push_str("tailmarker");

        let first = tokenizer
            .tokenize(&long_input)
            .expect("long input must tokenize");
        let second = tokenizer
            .tokenize(&long_input)
            .expect("long input must tokenize deterministically");

        // Determinism: repeated tokenization of the same input is identical.
        assert_eq!(first, second, "max-length truncation must be deterministic");

        // Total length is capped at the fixture's max_length.
        assert_eq!(
            first.len(),
            max_length,
            "truncated output length must equal the fixture max_length"
        );

        // [CLS] and [SEP] are preserved at the boundaries.
        assert_eq!(
            first.first(),
            Some(&101),
            "truncated output must keep [CLS] first"
        );
        assert_eq!(
            first.last(),
            Some(&102),
            "truncated output must keep [SEP] last"
        );

        // Content budget is max_length minus the two special tokens, matching
        // the pinned reference.
        let content_len = first.len() - 2;
        let golden_id = load_golden_token_ids()[4]; // id of "golden" = 3585
        assert_eq!(
            first.iter().filter(|&&i| i == golden_id).count(),
            content_len,
            "content tokens must be kept up to max_length - 2"
        );

        // Right truncation: the trailing marker must be dropped, the head kept.
        let marker_id = tokenizer.tokenize("tailmarker").expect("marker tokenizes");
        let marker_content = marker_id
            .iter()
            .skip(1)
            .take_while(|&&i| i != 102)
            .copied()
            .collect::<Vec<u32>>();
        assert!(
            !marker_content.is_empty(),
            "marker must tokenize to a real content token"
        );
        let marker_content_id = marker_content[0];
        assert!(
            !first.contains(&marker_content_id),
            "right truncation must drop the tail marker token"
        );
        assert!(
            first.contains(&golden_id),
            "right truncation must keep the head content tokens"
        );
    }
}
