# U-062 RED (real Candle forward)

## Test
`u062_real_candle_forward_matches_embedding_dimension` (`src/embedding.rs`, `#[ignore]`):
asserts the resident model's real Candle forward emits an embedding whose length
equals `word_embedding_dimension` from the ModelCar pooling config.

## Command
`cargo build --tests --locked`

## Worktree
HEAD `b08e335221fa5c69b089a1ef187003863923f250`; uncommitted changes:
Cargo.toml/Cargo.lock (add candle-transformers 0.11), src/embedding.rs (test
rewritten to reference `Embedder`).

## Failure excerpt
```
error[E0433]: cannot find type `Embedder` in this scope
  --> src/embedding.rs:97:24
error: could not compile `llm-d-sc` (lib test) due to 1 previous error
```

## Why this is the expected failure
The rewritten U-062 exercises a real forward through a new `Embedder` type
(`Embedder::load` + `embed`) that has not yet been implemented. The unresolved
type is the exact right-reason failure: the Candle backend does not exist yet,
so the real-forward test cannot compile. This is not a test-assertion failure —
it fails before the assertion can run, proving the test now depends on the real
forward backend rather than the prior contract-only dimension check.
