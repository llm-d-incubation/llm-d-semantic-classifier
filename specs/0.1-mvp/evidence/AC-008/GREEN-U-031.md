# AC-008 GREEN evidence — slice U-031

## Test ID
U-031 `u031_full_queue_returns_overload_resource_exhausted` (`src/queue.rs`, plain `#[test]`, offline).

## Command
```
cargo test --locked u031
```

## Result
PASSED. 1 passed; 0 failed.

```
running 1 test
test queue::tests::u031_full_queue_returns_overload_resource_exhausted ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s
```

## What changed (smallest implementation)
The same minimal `BoundedQueue<T>` implementation in `src/queue.rs` satisfies
this criterion: `try_enqueue` on a full queue returns
`QueueError::ResourceExhausted` rather than silently buffering.

U-031 constructs `BoundedQueue::new(1)`, admits one job, and asserts the second
`try_enqueue` returns `Err(QueueError::ResourceExhausted)` — the explicit
overload contract of AC-008.

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
