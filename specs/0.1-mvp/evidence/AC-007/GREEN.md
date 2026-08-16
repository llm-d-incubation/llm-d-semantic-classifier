# AC-007 GREEN evidence — identical concurrent misses do not create unbounded forwards

## Criterion
AC-007: identical concurrent misses do not create unbounded forwards.

`specs/0.1-mvp/test-plan.md` maps AC-007 to U-041 (unit), I-031 (integration,
"100 same-key simultaneous misses have bounded forward count"), and P-004 (perf,
"same-key burst miss coalescing").

## Unit-level proof
One offline plain `#[test]` in `src/cache.rs` (no model fetch):

- **U-041** `u041_identical_concurrent_misses_coalesce` — fires `CONCURRENCY = 8`
  simultaneous identical misses on an empty `SharedCache` (same key, all cache
  cold), synchronized so the misses genuinely overlap. Asserts the forward
  closure runs exactly ONCE total (`forward_count() == 1`), i.e. the 8 identical
  concurrent misses are coalesced into a SINGLE forward (bounded), and that every
  caller receives the same classification result.

### Command
```
cargo test --locked u041
```

### Result
GREEN — passes; `forward_count() == 1`. Stable across 8 consecutive runs (no
deadlock), and the full cache group passes:

```
test cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime ... ok
test cache::tests::u041_identical_concurrent_misses_coalesce ... ok
test cache::tests::u042_cache_key_changes_with_model_classifier_revision ... ok
test cache::tests::u043_cache_key_changes_with_tokenizer_revision ... ok
test cache::tests::u044_cache_key_changes_with_taxonomy_revision ... ok
test result: ok. 5 passed; 0 failed
```

## Implementation
`src/cache.rs` — `SharedCache::classify_concurrent` implements single-flight
coalescing:

1. **Fast path**: serve an already-cached result (no forward).
2. **Miss**: the FIRST caller for the key registers a per-key single-flight slot
   (`in_flight: Mutex<HashMap<CacheKey, Arc<InFlight>>>`); it runs the forward
   closure exactly once and stores the result.
3. **Coalescing**: every other identical concurrent miss finds the in-flight
   slot, blocks on its condvar, and reads the shared result when the designated
   forwarder publishes it (`notify_all`) and stores it.

This bounds N identical concurrent misses to exactly ONE forward per distinct
key, so identical concurrent misses cannot create unbounded forwards.

## Test-correction note (reviewer attention)
The previous-turn RED test placed the `Barrier` INSIDE the forward closure. That
is provably incompatible with a correct single-flight cache: only ONE thread
runs the forward, so an N-way barrier inside it would never reach N arrivals —
a deadlock. Forcing N threads into the forward (barrier) while asserting
`forward_count() == 1` is internally contradictory.

The test now synchronizes the N threads with a `Barrier` placed OUTSIDE the
forward and holds the forward stage open with a generous duration so the misses
genuinely overlap before the first stores. All assertions are unchanged
(`forward_count() == 1`, same result for every caller). The correction was
re-validated against the RED (no-coalescing) implementation: it fails RED
deterministically (`forward_count() == 8`, verified across 5 runs), so it is
non-vacuous.

## Suites / worktree
- `./hack/test-impact src/cache.rs` -> `src/cache.rs` maps to `src/*` which is
  an unknown surface -> FULL SUITE required.
- `./hack/spec-check 0.1-mvp` -> OK.
- `./hack/verify` -> GREEN (fmt, clippy `-D warnings`, build, full test).
- Worktree: SHA `c5ccc0e4` (uncommitted). No commits/pushes.

## Note on I-031 and P-004
I-031 (100 same-key simultaneous misses bounded forward count) and P-004
(same-key burst miss coalescing) are integration/perf-environment tests per the
test plan and out of scope for this local unit turn. They remain open for their
environments.
