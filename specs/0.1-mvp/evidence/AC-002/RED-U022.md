# AC-002 RED evidence — U-022

## Acceptance criterion
AC-002 not-ready before model load/warmup (failure contract: missing/corrupt
model -> not ready).

## Proving test
- U-022 warmup failure keeps not-ready (this iteration)

## Command
```
cargo test --locked u022
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `da15ab9b68af324e63ea58be404625f83265adca` (unchanged — implementation uncommitted)
- Test-only change: `src/runtime.rs` adds `u022_warmup_failure_keeps_not_ready`.
- `git status`:
  ```
   M .agent/state/current.md
   M src/lib.rs
   ?? src/runtime.rs
   ?? specs/0.1-mvp/evidence/AC-002/
  ```

## Failure excerpt
```
$ cargo test --locked u022
test runtime::tests::u022_warmup_failure_keeps_not_ready ... FAILED

---- runtime::tests::u022_warmup_failure_keeps_not_ready stdout ----
thread '...' panicked at src/runtime.rs:83:14:
warmup must reject a missing model path: ()
...
test result: FAILED. 0 passed; 1 failed; ...
```

## Why this is the expected failure
AC-002's failure contract requires that a missing/corrupt model leaves the
runtime NOT ready. U-022 calls `warmup("/nonexistent/model/path")` and expects
`Err`, then asserts readiness stays not-ready. Today `Runtime::warmup` ignores
its `path` argument and unconditionally returns `Ok(())` while flipping `ready`
to true, so `expect_err` panics ("warmup must reject a missing model path: ()").
This is precisely the expected RED: the failure path that must keep the runtime
not-ready does not exist yet.
