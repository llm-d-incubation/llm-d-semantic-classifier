# AC-008 RED evidence — U-030 / U-031

## Proving tests
- U-030 `u030_inference_queue_capacity_is_bounded` (`src/queue.rs`, plain
  `#[test]`, offline — no model fetch required).
- U-031 `u031_full_queue_returns_overload_resource_exhausted` (`src/queue.rs`,
  plain `#[test]`, offline).

`specs/0.1-mvp/test-plan.md` maps AC-008 to U-030/U-031 (unit), I-035
(integration), and P-023 (perf, later expanded). This slice selects the
unit-level proving tests U-030 and U-031; I-035 and P-023 are deferred to
their integration/perf phases.

U-030 constructs a `BoundedQueue::new(3)`, fills it to capacity, and asserts
that admission at/over capacity is rejected (`Err(QueueError::ResourceExhausted)`)
and that `len()` never exceeds the configured capacity (no unbounded growth).

U-031 constructs a `BoundedQueue::new(1)`, admits one job, and asserts that a
full queue rejects further work with an explicit
`QueueError::ResourceExhausted` (overload explicit, never silently buffered).

## Why these are the proving tests for AC-008
AC-008 requires the inference queue to be bounded and overload to be explicit.
U-030 guards the boundedness contract (admission beyond capacity is rejected,
the queue never grows without limit); U-031 guards the explicit-overload
contract (a full queue returns a resource-exhausted error rather than
unboundedly buffering work). Together they pin the failure-contract line
"full queue -> explicit resource exhausted" from `specs/0.1-mvp/spec.md`.

## RED state (feature does not exist)
There is no bounded-queue abstraction in the crate yet: `src/lib.rs` ships
`cache`, `config`, `embedding`, `ranker`, `runtime`, `tokenizer` — no `queue`
module and no `BoundedQueue` type. This slice adds the test-only `src/queue.rs`
(module registered in `src/lib.rs`) holding U-030/U-031, which reference
`super::BoundedQueue`. Because `BoundedQueue` is undefined, neither proving test
can be selected or compiled.

## Command
```
cargo test --locked u030
```
(run separately per test; `cargo test --locked u031` yields the identical
compile error — `cargo test` accepts a single TESTNAME filter.)

## Result
FAILED. Expected RED reason: the bounded-queue feature (AC-008) does not exist
yet, so the proving tests cannot compile, let alone pass.

Failure excerpt (U-030; U-031 identical):
```
error[E0432]: unresolved import `super::BoundedQueue`
  --> src/queue.rs:26:17
   |
26 |     use super::{BoundedQueue, QueueError};
   |                 ^^^^^^^^^^^^ no `BoundedQueue` in `queue`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (lib test) due to 1 previous error
```
Exit code: 101.

## Why this is the expected failure
AC-008 demands a bounded inference queue whose capacity is bounded (U-030) and
which surfaces overload explicitly (U-031). No such abstraction exists: the crate
only implements cache/runtime/config from earlier criteria. Because
`BoundedQueue` is undefined, `cargo test --locked u030` / `u031` fails at exit
101 with "no `BoundedQueue` in `queue`". This is precisely the expected RED: the
feature (bounded queue with explicit overload) does not exist, so the proving
tests cannot run, let alone pass. The failure is deterministic and confirms the
tests are non-vacuous — once a `BoundedQueue` with a bounded-capacity
`try_enqueue` returning `QueueError::ResourceExhausted` is implemented, U-030 and
U-031 become selectable and must pass.

Note: `./hack/verify` is NOT run this iteration — per AGENTS.md steps 1-4 only
the RED proof and evidence are required before implementation. The
GREEN/implementation step will add the minimal `BoundedQueue`/`QueueError`
abstraction so U-030/U-031 compile and pass, then I-035 / P-023 are exercised in
their integration/perf phases.

## Worktree / SHA
- HEAD SHA: `2e7629cc5e4e2f37b7c04d5d17491502761a04c9` (uncommitted changes).
- `git status`:
  ```
   M src/lib.rs
  ?? src/queue.rs
  ```
  `src/lib.rs` registers the new `queue` module; `src/queue.rs` holds the
  U-030/U-031 proving tests (test-only; `BoundedQueue` intentionally not yet
  implemented). No commits/pushes.
