# AC-008 GREEN evidence — slice U-030

## Test ID
U-030 `u030_inference_queue_capacity_is_bounded` (`src/queue.rs`, plain `#[test]`, offline).

## Command
```
cargo test --locked u030
```

## Result
PASSED. 1 passed; 0 failed.

```
running 1 test
test queue::tests::u030_inference_queue_capacity_is_bounded ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s
```

## What changed (smallest implementation)
Added the `BoundedQueue<T>` abstraction in `src/queue.rs`:
- `BoundedQueue::new(capacity)` creates a queue that holds at most `capacity` jobs.
- `try_enqueue` rejects admission when the queue is at/over capacity, returning
  `QueueError::ResourceExhausted`, and never grows beyond capacity.
- `len()` returns the current queue length; `capacity()` returns the configured capacity.

U-030 constructs `BoundedQueue::new(3)`, fills it to capacity, asserts
`len() == capacity`, then asserts a further `try_enqueue` returns
`Err(QueueError::ResourceExhausted)`, that `len()` is preserved (no unbounded
growth), and that `capacity()` still reports the configured capacity. All pass.

## Worktree / SHA
- HEAD SHA: `2e7629cc5e4e2f37b7c04d5d17491502761a04c9` (uncommitted changes).
- `git status`:
  ```
   M .agent/state/current.md
   M src/lib.rs
  ?? specs/0.1-mvp/evidence/AC-008/
  ?? src/queue.rs
  ```
- `src/lib.rs` registers `pub mod queue;`; `src/queue.rs` now implements the
  bounded queue plus the U-030/U-031 proving tests. No commits/pushes.
