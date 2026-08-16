# SWAP-tokenizers-crate.md

Corrective slice: replace the hand-rolled tokenizer with the official
HuggingFace `tokenizers` crate.

## Change

- `Cargo.toml`: `tokenizers = "0.23.1"` already present (pinned); verified it
  resolves and compiles standalone (`tokenizers` 0.23.1 in `Cargo.lock`).
- `src/tokenizer.rs`: rewritten from a ~324-line hand-rolled implementation
  (`BertNormalizer` clean-text/accent-strip, `BertPreTokenizer` punctuation
  split, `WordPiece` greedy match, `TemplateProcessing`, unicode tables,
  CJK spacing) to a thin resident wrapper:

```rust
pub struct Tokenizer {
    inner: tokenizers::Tokenizer,
}

impl Tokenizer {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Tokenizer, TokenizerError> {
        let inner = tokenizers::Tokenizer::from_file(path).map_err(TokenizerError::Tokenizers)?;
        Ok(Tokenizer { inner })
    }

    pub fn tokenize(&self, text: &str) -> Result<Vec<u32>, TokenizerError> {
        let encoding = self.inner.encode(text, true).map_err(TokenizerError::Tokenizers)?;
        Ok(encoding.get_ids().to_vec())
    }
}
```

Production body is now ~45 lines (was ~324). All normalization, pre-tokenization,
wordpiece, template processing, and truncation are delegated to the crate.

## Truncation via the crate

The fixture `tokenizer.json` ships `truncation.max_length` (256); `from_file`
deserializes the full tokenizer including that truncation config, so over-length
inputs are capped by the crate exactly as the pinned reference does. The
`tokenizers::Tokenizer` wrapper exposes no public `with_truncation`, so the
file-declared truncation is the crate path that matches the fixture.

## Public signatures preserved (callers compile unchanged)

- `Tokenizer::load<P: AsRef<Path>>(path) -> Result<Tokenizer, TokenizerError>`
  — used by `src/embedding.rs` (`EmbeddingError::Tokenizer`) and
  `src/runtime.rs` (`load_tokenizer_once`, `.to_string()`).
- `Tokenizer::tokenize(&self, text) -> Result<Vec<u32>, TokenizerError>`
  — used by `src/embedding.rs::embed`.
- `TokenizerError` implements `Display` + `Error` (was `Io/Json/MissingField`;
  now `Tokenizers(tokenizers::Error)`).

## Equivalence proof (parity tests unchanged, all GREEN)

`./hack/test-parity` -> GREEN (5 passed):

- `u060_tokenizer_golden_token_ids_match_trusted_reference`
- `u066_max_length_truncation_deterministic`
- `u063_embedding_normalization_matches_classifier_definition`
- `u061_pooling_output_matches_trusted_reference`
- `u067_golden_fixture_ranking_matches_reference`
- `u062_real_candle_forward_matches_embedding_dimension`

`cargo test -- u060 u066` -> GREEN (2 passed), proving golden token-ID and
truncation parity against the pinned reference.

## Line count

- Before: `src/tokenizer.rs` 456 lines total (~324 production).
- After: `src/tokenizer.rs` 190 lines total (~45 production + 145 test module).
