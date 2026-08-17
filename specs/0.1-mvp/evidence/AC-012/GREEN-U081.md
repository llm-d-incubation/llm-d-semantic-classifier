# AC-012 GREEN evidence — U-081 cache hit/miss counters correct

## Criterion
AC-012 requires queue/tokenize/forward/total latency to be visible. U-081 (from
`specs/0.1-mvp/test-plan.md` -> AC-012) asserts the cache hit/miss counters are
counted independently and exactly, partitioning every request.

## Proving test
`u081_cache_hit_miss_counters_correct` in `tests/metrics.rs`.

## Implementation (smallest change)
`src/metrics.rs` (new): `Metrics::record_cache_hit()` increments the hit counter,
`Metrics::record_cache_miss()` increments the miss counter, and the snapshot
exposes both independently. `ClassifyService::classify` distinguishes a hit from
a miss (the forward closure runs only on a miss — AC-006) and records exactly one
of the two counters per request.

## Command
```
cargo test --locked --test metrics -- u081_cache_hit_miss_counters_correct
```

## Result
PASSED.

## Why this proves U-081
The test records two hits and one miss and asserts the hit counter is exactly 2,
the miss counter exactly 1, and their sum equals the request count (3). Counters
that leak across each other, or that do not partition every request, cannot
satisfy these assertions.

## Worktree / SHA
- HEAD SHA: `e27ccd39a938670d9e9c5858151dd3e5b964b573`
- Working tree (uncommitted): `src/metrics.rs` (new), `src/classify.rs`,
  `src/grpc/classify.rs`, `src/lib.rs` modified; `tests/metrics.rs` untracked.
  No commits/pushes.
