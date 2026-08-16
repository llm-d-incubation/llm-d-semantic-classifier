# AC-007 GREEN evidence — U-041 (slice)

## Proving test
- U-041 `u041_identical_concurrent_misses_coalesce` (`src/cache.rs`, plain
  `#[test]`, offline — no model fetch required).
- Creates a `SharedCache` and fires `CONCURRENCY = 8` simultaneous identical
  misses on an empty cache (same key, all cache cold). Asserts the forward
  closure runs exactly ONCE total (`forward_count() == 1`) — the 8 identical
  concurrent misses are coalesced into a SINGLE forward (bounded) — and that
  every caller receives the same classification result.

## Test-correction note (why the barrier moved / overlap forced by a hold)
The previous-turn RED test placed the `Barrier` INSIDE the forward closure.
That is provably incompatible with a correct single-flight cache: only ONE
thread runs the forward, so an N-way barrier inside it would never reach N
arrivals — a deadlock. Keeping the barrier inside the forward while asserting
`forward_count() == 1` is internally contradictory (forcing N threads into the
forward forces N forwards).

The test now synchronizes the N threads with a `Barrier` placed OUTSIDE the
forward (they reach `classify_concurrent` together as concurrent misses), and
the forward closure holds the forward stage open for a generous duration so the
misses genuinely overlap before the first one stores. All assertions are
unchanged: `forward_count() == 1` and every caller gets the same result.

This test correction was re-validated against the RED (no-coalescing)
implementation: it fails RED deterministically (`forward_count() == 8`,
verified across 5 runs), so it is non-vacuous and still proves the AC-007 bug.

## Command
```
cargo test --locked u041
```

## Result
PASSED. `forward_count() == 1`: the 8 identical concurrent misses were
coalesced into a SINGLE forward; the other 7 waited for and read the shared
result. Every caller received the same classification result.

Stable across 8 consecutive runs (no deadlock, `forward_count() == 1` every
time), and the full cache group passes:
```
$ cargo test --locked cache
test cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime ... ok
test cache::tests::u041_identical_concurrent_misses_coalesce ... ok
test cache::tests::u042_cache_key_changes_with_model_classifier_revision ... ok
test cache::tests::u043_cache_key_changes_with_tokenizer_revision ... ok
test cache::tests::u044_cache_key_changes_with_taxonomy_revision ... ok
test result: ok. 5 passed; 0 failed
```

## Implementation (smallest change)
`SharedCache::classify_concurrent` now implements single-flight coalescing:
- Fast path: serve an already-cached result (no forward).
- On a miss, the FIRST caller for the key registers a single-flight slot and
  runs the forward exactly once; every other identical concurrent miss finds
  the in-flight slot, blocks on its condvar, and reads the shared result when
  the designated forwarder publishes it (`notify_all`) and stores it.
- Bounded: N identical concurrent misses produce exactly ONE forward per key.

## Why this is the GREEN for AC-007
`specs/0.1-mvp/test-plan.md` maps AC-007 to U-041 ("identical concurrent misses
coalesce/bound duplicate forwards"). With single-flight coalescing, the 8
overlapping identical misses produce exactly one forward (`forward_count() ==
1`), so identical concurrent misses no longer create unbounded forwards.

## Worktree / SHA
- HEAD SHA: `c5ccc0e4ae16fc0a0f86bd8ff23db5b9f9310259` (uncommitted changes).
- `git status`: `M src/cache.rs` (single-flight coalescing + corrected U-041
  overlap forcing), `?? specs/0.1-mvp/evidence/AC-007/`. No commits/pushes.
