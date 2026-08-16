# AC-004 RED evidence — U-063

## Proving test
- U-063 `u063_embedding_normalization_matches_classifier_definition` (`src/embedding.rs`, `#[ignore]`)
- Asserts the DOCUMENTED normalization contract: because the pinned model's
  `modules.json` declares a `sentence_transformers.models.Normalize` module
  (idx 2), `embed()` must emit an L2-NORMALIZED embedding (norm ~1.0), not the
  raw masked-mean-pooled vector.

## modules.json finding (recorded per task)
- `artifacts/models/sensitivity/modules.json` was ABSENT locally.
- Fetched it from the pinned repo `cnuland/semantic-routing-sensitivity` @ rev
  `43f21d21ac48134464f8510a9ac9c95bdac7ba86` via `hf_hub_download` (the
  `./hack/fetch-model`-style fetch). It resolved successfully (NOT 404):
  ```json
  [
    {"idx":0,"name":"0","path":"","type":"sentence_transformers.models.Transformer"},
    {"idx":1,"name":"1","path":"1_Pooling","type":"sentence_transformers.models.Pooling"},
    {"idx":2,"name":"2","path":"2_Normalize","type":"sentence_transformers.models.Normalize"}
  ]
  ```
- `2_Normalize/config.json` returned 404 (`RemoteEntryNotFoundError`) — expected:
  the sentence-transformers `Normalize` module is stateless and carries no config.
- CONCLUSION: the task's fallback branch ("no Normalize module -> unnormalized
  embeddings + cosine similarity") does NOT apply. A Normalize module EXISTS, so
  the documented normalization contract is NORMALIZED (L2-normalized) embeddings.

## Command
```
cargo test --locked -- --ignored u063
```

## Result
FAILED (0 passed; 1 failed). Expected RED reason: `embed()` returns the raw
masked-mean-pooled vector (norm 5.7587), which is NOT the classifier-defined
normalized embedding.

Failure excerpt:
```
thread 'embedding::tests::u063_embedding_normalization_matches_classifier_definition' panicked at src/embedding.rs:317:9:
embed() must emit an L2-normalized embedding per the classifier's Normalize module (got norm 5.758741102092094, want ~1.0)
test result: FAILED. 0 passed; 1 failed; 0 ignored; 12 filtered out
```

## Worktree / SHA
- SHA: `75fa0a4`
- `git status`: `M src/embedding.rs` (the new U-063 test only)
- `modules.json` is gitignored (weights/model dir never committed).

## Why this is the expected failure
The resident `Embedder::embed` performs tokenization + forward + masked mean
pooling only (src/embedding.rs mean_pool). It does NOT apply the Normalize stage
the classifier definition declares. L2 normalization always yields norm 1.0, so
the current output (5.7587) violates the documented contract by ~4.76.
