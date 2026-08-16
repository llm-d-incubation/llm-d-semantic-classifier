# AC-004 GREEN evidence — U-063

## Proving test
- U-063 `u063_embedding_normalization_matches_classifier_definition` (`src/embedding.rs`, `#[ignore]`)
- Asserts `embed()` emits an L2-normalized embedding (norm ~1.0) per the
  documented classifier definition.

## modules.json finding
- Fetched `modules.json` from pinned repo `cnuland/semantic-routing-sensitivity`
  @ rev `43f21d21ac48134464f8510a9ac9c95bdac7ba86` — present (not 404). It
  declares module idx 2 `2_Normalize` of type `sentence_transformers.models.Normalize`.
  Therefore the documented normalization contract is NORMALIZED embeddings.
  `2_Normalize/config.json` is 404 (stateless Normalize carries no config).
- Recorded in `RED-U063.md`.

## Smallest change
`src/embedding.rs` `Embedder::embed`: after masked mean pooling, L2-normalize the
output vector to unit norm (the Normalize stage the classifier definition
declares). Used `Tensor::norm` + `broadcast_div` (candle 0.11; `norm_l2` is not
in this version).

## Command
```
cargo test --locked -- --ignored u063
```

## Result
```
test embedding::tests::u063_embedding_normalization_matches_classifier_definition ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 12 filtered out
```

## Worktree / SHA
- SHA: `75fa0a4` (test added then implementation change; both unstaged `M src/embedding.rs`)
- No commits/pushes made.

## Note (scope boundary)
- U-061 parity (RED/escalated per `.agent/state/current.md`) is untouched and out
  of scope for this U-063 slice. U-063 proves the normalization contract against
  the classifier definition; it does not modify U-061's assertions.
