# AC-006 RED evidence — U-040

## Proving test
- U-040 `u040_exact_cache_hit_bypasses_tokenizer_and_runtime` (`src/cache.rs`,
  plain `#[test]`, offline — no model fetch required).
- Creates an `ExactCache`, classifies an identical key twice. The forward
  closure stands in for the tokenize + model-forward stage. The first call is a
  miss (forward runs once); the second identical call is a cache HIT. The test
  asserts the forward closure is invoked exactly ONCE total (`forward_count() ==
  1`), i.e. the hit bypasses the tokenizer and model forward entirely, and that
  the hit is counted (`hit_count() == 1`) and returns the exact cached result.

## Why this is the proving test for AC-006
`specs/0.1-mvp/test-plan.md` maps AC-006 to U-040 ("exact cache hit bypasses
tokenizer and runtime"). AC-006 requires a cache hit to bypass the tokenizer
and the model forward. To run the test I added a minimal `ExactCache` stub in
`src/cache.rs` whose behavior is the pre-fix wrong behavior: it ALWAYS invokes
the forward closure, even on a cache hit, so the tokenizer/model forward is
never bypassed (the forward count increments on the hit too). The fix (check
the cache first and return the cached result without invoking the forward on a
hit) is NOT yet implemented this turn.

## Command
```
cargo test --locked u040
```

## Result
FAILED. Expected RED reason: the current cache forwards on every call, so the
forward count is 2 (one for the miss, one for the hit) instead of 1 (miss only;
the hit must bypass).

Failure excerpt:
```
running 1 test
test cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime ... FAILED

---- cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime stdout ----
thread 'cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime' (18781651) panicked at src/cache.rs:97:9:
assertion `left == right` failed: cache hit must bypass the tokenizer and model forward
  left: 2
 right: 1

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out
```

## Why this is the expected failure
The proving test asserts the AC-006 bypass contract: a cache hit must not
re-run the tokenizer/model forward. The stub's pre-fix behavior forwards on
every call (miss AND hit), so the forward count is 2 rather than 1. The failure
is at the primary bypass assertion (`left: 2, right: 1`), confirming the test
guards the bypass contract and is non-vacuous: a cache that re-forwards on a
hit fails, while a correct cache that serves the hit from its entries without
forwarding would keep the count at 1.

## Worktree / SHA
- SHA: `df21d9a90c557237308fc82011e691d2cebf4944` (uncommitted changes).
- `git status`: `M src/lib.rs` (register `cache` module), `?? src/cache.rs`
  (RED stub + U-040 test). No commits/pushes.
