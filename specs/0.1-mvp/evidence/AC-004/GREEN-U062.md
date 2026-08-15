# U-062 GREEN (real Candle forward)

## Test
`u062_real_candle_forward_matches_embedding_dimension` (`src/embedding.rs`, `#[ignore]`):
loads the fetched sensitivity model via `Embedder::load`, embeds the golden input,
and asserts the real forward's embedding length equals `word_embedding_dimension`
from the ModelCar pooling config.

## Command
`cargo test --locked -- --ignored u062`

## Worktree
HEAD `b08e335221fa5c69b089a1ef187003863923f250`; uncommitted changes implement
the real Candle forward (`src/embedding.rs` + candle-transformers dep).

## Result
```
running 1 test
test embedding::tests::u062_real_candle_forward_matches_embedding_dimension ... ok
test result: ok. 1 passed; 0 failed; 0 ignored
```
`cargo build --tests --locked` clean (no warnings).

## What the implementation does
- Parses `artifacts/models/sensitivity/config.json` into `bert::Config` (serde).
- `unsafe VarBuilder::from_mmaped_safetensors(&[model.safetensors], F32, Cpu)`
  memory-maps the weights, then `BertModel::load`.
- `embed(text)` tokenizes with the resident `Tokenizer` -> `input_ids`,
  zero `token_type_ids`, all-ones `attention_mask`; runs `BertModel::forward`;
  masked mean-pools the sequence dim; returns the `[384]` `Vec<f32>`.
- Validates `word_embedding_dimension` (pooling config) == `hidden_size` (bert
  config) so the forward emits the contracted dim.
