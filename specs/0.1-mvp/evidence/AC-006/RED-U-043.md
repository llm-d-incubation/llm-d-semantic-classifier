# AC-006 RED evidence — U-043 (slice)

## Proving test
- U-043 `u043_cache_key_changes_with_tokenizer_revision` (`src/cache.rs`, plain
  `#[test]`, offline — no model fetch required).
- Builds two `CacheKey`s from the SAME normalized text `"same normalized input"`
  with identical classifier/model/taxonomy revisions but DIFFERENT tokenizer
  revisions (`tok-rev-1` vs `tok-rev-2`). It asserts the two keys are NOT equal,
  and that classifying both into one `ExactCache` runs the forward closure twice
  (a miss — the stale cached classification under the old tokenizer revision is
  never served).

## Why this proves AC-006 (design.md key contract)
`specs/0.1-mvp/design.md` is NORMATIVE: "Do not use a raw prompt string as the
sole cache identity." The key must fingerprint the tokenizer revision, so
identical text under a different tokenizer revision must MISS (different key),
never serve a stale cached classification.

## RED state (buggy key equality)
For the RED slice the `CacheKey` struct carries all five fingerprint fields, but
`PartialEq`/`Hash` consider ONLY the normalized-text hash (the design violation:
revision changes do not affect key identity). The fix (fold every revision into
equality/hash) is NOT yet implemented this turn.

## Command
```
cargo test --locked u043
```

## Result
FAILED. Expected RED reason: `tok-rev-1` and `tok-rev-2` keys are considered
EQUAL (same text hash), so the tokenizer revision change does not change the key
— the design contract is violated.

Failure excerpt:
```
---- cache::tests::u043_cache_key_changes_with_tokenizer_revision stdout ----
thread 'cache::tests::u043_cache_key_changes_with_tokenizer_revision' (18832505) panicked at src/cache.rs:205:9:
assertion `left != right` failed: a tokenizer revision change must change the cache key
  left: CacheKey { classifier_id: "clf", model_revision: "model-rev", tokenizer_revision: "tok-rev-1", taxonomy_revision: "tax-rev", normalized_text_hash: 13336777793627534463 }
 right: CacheKey { classifier_id: "clf", model_revision: "model-rev", tokenizer_revision: "tok-rev-2", taxonomy_revision: "tax-rev", normalized_text_hash: 13336777793627534463 }
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 20 filtered out
```

## Why this is the expected failure
The proving test asserts the design.md key contract: a tokenizer revision change
must produce a different key. The RED key equality ignores the revision fields
(only the text hash participates), so both keys are equal and the primary
`assert_ne!` fails. This confirms U-043 is non-vacuous: a cache key that ignores
the tokenizer revision fails, while a correct versioned fingerprint that folds
the revision into equality/hash would keep the keys distinct.

## Worktree / SHA
- SHA: `df21d9a90c557237308fc82011e691d2cebf4944` (uncommitted changes).
- `git status`: `M src/lib.rs`, `?? src/cache.rs`, `?? specs/0.1-mvp/evidence/AC-006/`.
  No commits/pushes.
