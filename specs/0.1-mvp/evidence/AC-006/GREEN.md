# AC-006 GREEN evidence — cache hit bypasses tokenizer/model forward

## Criterion
AC-006: cache hit bypasses tokenizer/model forward.

`specs/0.1-mvp/test-plan.md` maps AC-006 to U-040 (unit), I-030 (integration),
and P-001/P-002 (perf). The unit tier additionally requires the design.md key
contract (U-042/U-043/U-044): the cache key is a versioned fingerprint of the
classifier/model/tokenizer/taxonomy revisions plus a hash of the normalized
context, never the raw prompt string as sole identity.

## Unit-level proof
Four offline plain `#[test]`s in `src/cache.rs` (no model fetch):

- **U-040** `u040_exact_cache_hit_bypasses_tokenizer_and_runtime` — classifies an
  identical key twice; the forward closure stands in for the tokenize +
  model-forward stage. Asserts the forward closure runs exactly ONCE total
  (`forward_count() == 1`): once on the miss and NOT again on the hit, i.e. the
  cache hit bypasses the tokenizer and model forward. Also asserts the hit is
  counted (`hit_count() == 1`) and returns the exact cached result.
- **U-042** `u042_cache_key_changes_with_model_classifier_revision` — identical
  normalized text under a different model/classifier revision yields a different
  key -> a cache miss (stale cached classification under the old revision is
  never served).
- **U-043** `u043_cache_key_changes_with_tokenizer_revision` — identical text
  under a different tokenizer revision yields a different key -> a miss.
- **U-044** `u044_cache_key_changes_with_taxonomy_revision` — identical text
  under a different taxonomy/prototype revision yields a different key -> a miss.

### Commands
```
cargo test --locked u040
cargo test --locked u042
cargo test --locked u043
cargo test --locked u044
```

### Result
GREEN — all four pass (each `1 passed; 0 failed`).

```
running 1 test
test cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime ... ok
test cache::tests::u042_cache_key_changes_with_model_classifier_revision ... ok
test cache::tests::u043_cache_key_changes_with_tokenizer_revision ... ok
test cache::tests::u044_cache_key_changes_with_taxonomy_revision ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s
```

## Implementation
`src/cache.rs`:

1. **Versioned fingerprint `CacheKey`** (design.md): struct
   `{ classifier_id, model_revision, tokenizer_revision, taxonomy_revision,
   normalized_text_hash }`, built via `CacheKey::new(classifier_id,
   model_revision, tokenizer_revision, taxonomy_revision, normalized_text)`.
   `normalized_text` is hashed with `std DefaultHasher`; the raw prompt string is
   never retained as key identity. `PartialEq`/`Eq`/`Hash` fold EVERY revision
   plus the normalized-text hash into key identity, so a revision change with
   identical text yields a different key (miss).
2. **Bypass-correct `ExactCache::classify`** (U-040, unchanged): on a cache HIT
   increment `hit_count` and return the cached result WITHOUT invoking the
   forward closure (tokenizer + model forward bypassed); on a miss invoke the
   forward closure exactly once, increment `forward_count`, store, and return.

This resolves the reviewer's blocking finding: the old `CacheKey(String)` used
the raw prompt as the entire key, so a model/tokenizer/taxonomy revision change
would have served stale cached classifications.

## Suites / worktree
- `./hack/test-impact src/cache.rs` -> `src/cache.rs` maps to `src/*` which is
  an unknown surface -> FULL SUITE required.
- `./hack/spec-check 0.1-mvp` -> OK.
- `./hack/verify` -> GREEN (fmt, clippy `-D warnings`, build, full test).
- Worktree: SHA `df21d9a` (uncommitted). No commits/pushes.

## Note on I-030 and P-001/P-002
I-030 (warmed result cache hit invokes zero model forwards) and P-001/P-002
(perf cache hit) are integration/perf-environment tests per the test plan and
out of scope for this local unit turn. They remain open for their environments.
