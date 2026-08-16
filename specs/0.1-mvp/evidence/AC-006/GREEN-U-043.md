# AC-006 GREEN evidence — U-043 (slice)

## Proving test
- U-043 `u043_cache_key_changes_with_tokenizer_revision` (`src/cache.rs`, plain
  `#[test]`, offline — no model fetch required).
- Builds two `CacheKey`s from the SAME normalized text with identical
  classifier/model/taxonomy revisions but DIFFERENT tokenizer revisions
  (`tok-rev-1` vs `tok-rev-2`). Asserts the keys are NOT equal and that
  classifying both into one `ExactCache` runs the forward closure twice (a miss —
  the stale cached classification under the old tokenizer revision is never
  served).

## Why this proves AC-006 (design.md key contract)
`specs/0.1-mvp/design.md` is NORMATIVE: the cache key must be a versioned
fingerprint of classifier/model/tokenizer/taxonomy revision plus the hash of the
normalized context; a raw prompt string must never be the sole identity. U-043
proves a tokenizer revision change produces a different key -> a cache miss, so
no stale cached classification is served under the new revision.

## Change
`src/cache.rs` — `CacheKey` is now a versioned fingerprint struct
`{ classifier_id, model_revision, tokenizer_revision, taxonomy_revision,
normalized_text_hash }` (derive Debug/Clone; hand-implemented `PartialEq`/`Eq`/
`Hash` that fold EVERY revision plus the normalized-text hash into key identity).
The `normalized_text` is hashed via `std DefaultHasher`; the raw string is never
retained as key identity.

## Command
```
cargo test --locked u043
```

## Result
GREEN.

```
running 1 test
test cache::tests::u043_cache_key_changes_with_tokenizer_revision ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s
```

## Suites / worktree
- `./hack/test-impact src/cache.rs` -> `src/*` unknown surface -> FULL SUITE
  required.
- `./hack/spec-check 0.1-mvp` -> OK.
- `./hack/verify` -> GREEN.
- Worktree: SHA `df21d9a` (uncommitted). No commits/pushes.
