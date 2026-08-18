# AC-013 LOCAL-GREEN evidence — restart + complete context recomputes correctly

## Criterion
AC-013 restart + complete context recomputes correctly. After a restart the
disposable in-memory exact-result cache is lost; a request carrying COMPLETE
context must then RECOMPUTE correctly — a genuine fresh forward returning an
equivalent result to the pre-restart one, not abstain and not a stale hit.

## Tests mapped in test-plan.md
`specs/0.1-mvp/test-plan.md` maps AC-013 to I-045 (integration) and S-020
(Kubernetes system). No unit-level (U-*) tests are mapped to this criterion.
I-045 passes locally, so the whole-criterion LOCAL-GREEN.md for the local
(worker) scope is written here. PROMOTION-GREEN.md is reserved for when the
integration/system/perf tiers (S-020) also pass and is never written by the
worker.

## Command & result
```
cargo test --locked --test restart
```
PASSED — 1 passed; 0 failed:
```
running 1 test
test i045_restart_full_context_recomputes_correctly ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

## What the local evidence proves
- Pre-restart server classifies a complete-context input and returns ranked
  signals, not abstain (`cache_misses == 1`, one forward).
- Restart simulated by dropping the pre-restart `ClassifyServer` (in-memory
  cache lost) and binding a fresh server (empty cache).
- Post-restart server recomputes the same full-context input: `cache_misses ==
  1` on the fresh server — a genuine fresh forward, proving the disposable
  cache was NOT carried across the restart.
- Recomputed ranked signals are exactly equivalent to the pre-restart ones
  (deterministic pipeline over committed synthetic fixtures).

## RED note (no feature-level RED)
`RED.md` documents that no feature-level RED exists on the local deterministic
path: restart + complete-context recompute pre-exists, so I-045 is
non-vacuously GREEN immediately. The smallest change is the proving test
`tests/restart.rs`, which pins the behavior as a regression guard.

## Evidence files
- `specs/0.1-mvp/evidence/AC-013/RED.md` (honest RED investigation: no feature
  RED, escalated)
- `specs/0.1-mvp/evidence/AC-013/GREEN-I045.md`
- `specs/0.1-mvp/evidence/AC-013/GREEN.md` (prior slice summary, superseded by
  GREEN-I045.md)

## Deferred to their phase
- S-020 (Kubernetes kill/restart then full-context recompute) — system tier, not
  run by the worker; required for PROMOTION-GREEN only. Consistent with how
  AC-009/AC-011/AC-012 defer the Kubernetes E2E.

## Worktree / SHA
- HEAD SHA: `752d5671d55f01f5bd90d957779fc84d7a1e0721`
- Working tree: `tests/restart.rs` added (untracked); `src/` unchanged.
- No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-013 -> I-045, S-020
- `tests/TEST_MATRIX.md` I-045 / S-020
- `specs/0.1-mvp/spec.md` AC-013; State: exact-result cache is disposable
- `specs/0.1-mvp/acceptance.md` "Restart/full-context correctness passes."
