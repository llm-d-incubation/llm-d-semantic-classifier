# AC-008 GREEN evidence — queue bounded; overload explicit

## Criterion
AC-008 queue bounded; overload explicit.

## Unit-level tests mapped in test-plan.md
`specs/0.1-mvp/test-plan.md` maps AC-008 to U-030/U-031 (unit), I-035
(integration), and P-023 (perf, later expanded). The unit-level proving tests
U-030 and U-031 both pass, so the whole-criterion GREEN.md for the unit scope is
written here.

## Commands & results
```
cargo test --locked u030
```
PASSED — 1 passed; 0 failed.

```
cargo test --locked u031
```
PASSED — 1 passed; 0 failed.

## Implementation
Smallest change: added `BoundedQueue<T>` + `QueueError` to `src/queue.rs`.
- `BoundedQueue::new(capacity)` bounds the queue to at most `capacity` jobs.
- `try_enqueue` returns `Err(QueueError::ResourceExhausted)` at/over capacity and
  never grows the queue beyond capacity (no unbounded buffering).
- `len()` / `capacity()` report current length and configured capacity.
- `src/lib.rs` registers `pub mod queue;`.

## Evidence files
- `specs/0.1-mvp/evidence/AC-008/GREEN-U-030.md`
- `specs/0.1-mvp/evidence/AC-008/GREEN-U-031.md`

## Deferred to their phases
- I-035 (integration: saturation rejects rather than runaway queueing).
- P-023 (perf: concurrency 32 / saturation, later expanded).

## Worktree / SHA
- HEAD SHA: `2e7629cc5e4e2f37b7c04d5d17491502761a04c9` (uncommitted changes).
- `git status`:
  ```
   M .agent/state/current.md
   M src/lib.rs
  ?? specs/0.1-mvp/evidence/AC-008/
  ?? src/queue.rs
  ```
- No commits/pushes.
