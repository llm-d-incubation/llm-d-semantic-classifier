# AC-006 GREEN evidence — U-040 (slice)

## Proving test
- U-040 `u040_exact_cache_hit_bypasses_tokenizer_and_runtime` (`src/cache.rs`,
  plain `#[test]`, offline — no model fetch required).
- Creates an `ExactCache`, classifies an identical key twice. The forward
  closure stands in for the tokenize + model-forward stage. The first call is a
  miss (forward runs once); the second identical call is a cache HIT. The test
  asserts the forward closure is invoked exactly ONCE total (`forward_count() ==
  1`), i.e. the hit bypasses the tokenizer and model forward entirely, and that
  the hit is counted (`hit_count() == 1`) and returns the exact cached result.

## Why this proves AC-006
`specs/0.1-mvp/test-plan.md` maps AC-006 to U-040 ("exact cache hit bypasses
tokenizer and runtime"). AC-006 requires a cache hit to bypass the tokenizer
and the model forward. The forward closure stands in for the tokenize +
model-forward stage; the test asserts it runs exactly ONCE (on the miss) and NOT
again on the hit.

## Change
`src/cache.rs` — smallest fix to `ExactCache::classify`, replacing the pre-fix
RED stub that ALWAYS invoked the forward closure:
- On a cache HIT (`entries.get(&key)` returns `Some`): increment `hit_count`
  and return the cached result WITHOUT invoking the forward closure (tokenizer +
  model forward bypassed).
- On a miss: invoke the forward closure exactly once, increment `forward_count`,
  store the result, and return it.

No other files changed.

## Command
```
cargo test --locked u040
```

## Result
GREEN.

```
running 1 test
test cache::tests::u040_exact_cache_hit_bypasses_tokenizer_and_runtime ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.00s
```

## Suites / worktree
- `./hack/test-impact src/cache.rs` -> `src/cache.rs` maps to `src/*` which is
  an unknown surface -> FULL SUITE required.
- `./hack/spec-check 0.1-mvp` -> OK (deterministic checks passed; reviewer judges
  scope).
- `./hack/verify` -> GREEN (fmt, clippy `-D warnings`, build, full test).
- Worktree: SHA `df21d9a` (uncommitted). `git status`:
  `M src/lib.rs` (module registration from the RED slice), `?? src/cache.rs`,
  `?? specs/0.1-mvp/evidence/AC-006/`. No commits/pushes.

## Note on the remaining AC-006 tests
I-030 (warmed result cache hit invokes zero model forwards) and P-001/P-002
(perf cache hit) are integration/perf-environment tests per the test plan and
out of scope for this local unit turn. They remain open for their environments.
