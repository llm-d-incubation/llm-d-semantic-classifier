# AC-013 GREEN evidence — I-045 restart + full context recomputes correctly

## Test ID
I-045 (test-plan.md maps AC-013 to I-045, S-020).

## Test file
`tests/restart.rs` — `i045_restart_full_context_recomputes_correctly`
(a plain deterministic `#[test]`, integration over a real gRPC server).

## Command
```
cargo test --locked --test restart
```

## Result: GREEN
```
running 1 test
test i045_restart_full_context_recomputes_correctly ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

## What this slice proves
- A pre-restart `ClassifyServer` classifies a COMPLETE-context input over a real
  gRPC channel and returns ranked signals, not abstain; its metrics show exactly
  one cache miss (one forward).
- Dropping the pre-restart server and binding a fresh one simulates a restarted
  process: the disposable in-memory exact-result cache is gone.
- The restarted server recomputes the same full-context input: its metrics show
  exactly one cache miss (a genuine fresh forward, NOT a stale hit carried
  across restart) — `cache_misses == 1` on the post-restart server.
- The recomputed ranked signals are exactly equivalent to the pre-restart ones
  (deterministic tokenizer -> versioned cache -> single-flight -> ranker over
  the committed synthetic fixtures).

## Smallest implementation change
No `src/` change was required: restart + complete-context recompute already
behaves correctly on the deterministic path (see `RED.md` — no feature-level
RED exists; the pipeline recomputes a fresh forward that is exactly equivalent
after a restart). The smallest change is the addition of the proving/regression
test `tests/restart.rs` (untracked). It pins the behavior so a regression in
recompute-on-restart would fail RED going forward.

## Worktree / SHA
- HEAD SHA: `752d5671d55f01f5bd90d957779fc84d7a1e0721`
- Working tree: `tests/restart.rs` added (untracked); `src/` unchanged.
- `git status --short`:
  ```
   M .agent/state/current.md
   ?? specs/0.1-mvp/evidence/AC-013/
   ?? tests/restart.rs
  ```
- No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-013 -> I-045, S-020
- `tests/TEST_MATRIX.md` I-045 "restart + full context recomputes correctly"
- `specs/0.1-mvp/spec.md` AC-013; State: exact-result cache is disposable
- `src/classify.rs`, `src/cache.rs`, `src/ranker.rs`
