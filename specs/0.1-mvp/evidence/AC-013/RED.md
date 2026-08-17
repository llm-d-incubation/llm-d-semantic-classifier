# AC-013 RED evidence — restart + complete context recomputes correctly

## Status: NO FEATURE-LEVEL RED — escalated

This file records the honest RED investigation for AC-013. The maintainer's
turn instruction required proving RED for the expected reason before
implementing. The investigation found that **no feature-level RED exists on the
local deterministic path**: the deterministic pipeline already recomputes
correctly after a restart. No failure excerpt is fabricated below, because none
is genuine.

## Test ID
I-045 (test-plan.md maps AC-013 to I-045, S-020).

## Test file
`tests/restart.rs` — `i045_restart_full_context_recomputes_correctly` (a plain,
deterministic `#[test]`; integration over a real gRPC server).

## Command
```
cargo test --locked --test restart
```

## Worktree / SHA
- HEAD SHA: `752d5671d55f01f5bd90d957779fc84d7a1e0721`
- Working tree: `tests/restart.rs` added (untracked); `src/` unchanged.
- `git status`:
  ```
   ?? tests/restart.rs
  ```
  No commits/pushes.

## Result: GREEN on first baseline run (expected, feature pre-exists)
```
running 1 test
test i045_restart_full_context_recomputes_correctly ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
```

## Why there is no feature-level RED
AC-013 requires that after a restart (which drops the disposable in-memory
`SharedCache`), a request carrying COMPLETE context recomputes correctly — a
fresh forward that returns an equivalent result to the pre-restart one. The
restart is simulated by dropping the pre-restart `ClassifyServer` and binding a
fresh one (a fresh empty cache, exactly a restarted process). The pipeline
(`src/classify.rs`) is deterministic: tokenizer -> versioned cache ->
single-flight -> ranker over the committed synthetic fixtures
(`tests/fixtures/modelcar/tokenizer.json` + `synthetic-prototypes.json`). The
ranked signals derive solely from the normalized context and the pinned
revisions (`src/ranker.rs` is pure deterministic math, U-064/U-065), so a fresh
service recomputes an exactly equivalent result. The restarted server's cache
starts empty, so the recomputation is a genuine cache miss (fresh forward),
verified via `server.metrics_snapshot().cache_misses == 1`.

Because this behavior already exists, the proving test is non-vacuously GREEN
immediately. There is no missing feature to fail RED for, so recording a fake
failure would violate the engineering contract ("Never weaken/delete an
assertion merely to make CI pass"; no fabricated evidence). This is escalated
rather than silently reinterpreted (AGENTS.md: spec drift/ambiguity -> ESCALATE).

## Only observed failure (test-authoring, NOT the feature RED)
The first compile of `tests/restart.rs` failed because `ClassifyService::classify`
is a `ClassifierRuntime` trait method not imported at the call site
(`E0599 method not found`). This was a test-authoring import omission fixed
before the baseline run; it is not the expected feature RED for AC-013 and is
recorded here only for transparency.

## References
- `specs/0.1-mvp/test-plan.md` AC-013 -> I-045, S-020
- `tests/TEST_MATRIX.md` I-045 / S-020
- `specs/0.1-mvp/spec.md` AC-013; State: exact-result cache is disposable
- `specs/0.1-mvp/acceptance.md` "Restart/full-context correctness passes."
