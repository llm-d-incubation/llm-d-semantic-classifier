# AC-008 GREEN evidence — queue bounded; overload explicit; queue IN the request path

## Criterion
AC-008 queue bounded; overload explicit. Per ADR-0002, the bounded queue must be
IN the request path: the model forward does NOT run on a Tokio network worker; a
bounded handoff sits between the gRPC handler and a dedicated inference executor;
queue-full returns an explicit resource-exhausted status; and I-035 proves bound +
explicit overload + recovery.

## Tests mapped in test-plan.md
`specs/0.1-mvp/test-plan.md` maps AC-008 to U-030/U-031 (unit), I-035
(integration — queue IN the request path per ADR-0002), and P-020/P-021 (perf;
P-023 saturation is 0.21). All three proving tests pass.

## Commands & results
```
cargo test --locked u030
```
PASSED — 1 passed; 0 failed.

```
cargo test --locked u031
```
PASSED — 1 passed; 0 failed.

```
cargo test --locked --test queue i035
```
PASSED — 1 passed; 0 failed.

## Implementation
Two layers implement AC-008:
- `src/queue.rs`: `BoundedQueue<T>` + `QueueError` (the bounded FIFO abstraction).
  - `BoundedQueue::new(capacity)` bounds the queue to at most `capacity` jobs.
  - `try_enqueue` returns `Err(QueueError::ResourceExhausted)` at/over capacity
    and never grows the queue beyond capacity (no unbounded buffering).
  - `len()` / `capacity()` report current length and configured capacity.
  - `src/lib.rs` registers `pub mod queue;`.
- `src/handoff.rs`: `InferenceExecutor<R>` (the request-path bounded handoff).
  - A bounded `mpsc` channel + owned semaphore of `bound` permits between the
    gRPC handler and a dedicated executor thread.
  - The dedicated executor thread performs the model forward (NOT on a Tokio
    network worker) and returns the result via a `oneshot`.
  - Admission beyond `bound` returns `QueueFull`; in-flight + queued work never
    exceeds `bound` (`max_admitted <= bound`).
  - Queue wait is recorded through the existing `Metrics` Queue stage.
- `src/grpc/classify.rs`: `ClassifyServiceImpl::with_executor`/`new` spawn the
  dedicated executor; `queue_bound()`/`max_admitted()` expose the bound; the
  `classify` handler hands each request to `executor.try_enqueue` and maps
  `QueueFull` to tonic `resource_exhausted`.
- `src/lib.rs` registers `pub mod handoff;`.

## Evidence files
- `specs/0.1-mvp/evidence/AC-008/GREEN-U-030.md`
- `specs/0.1-mvp/evidence/AC-008/GREEN-U-031.md`
- `specs/0.1-mvp/evidence/AC-008/GREEN-I035.md`
- `specs/0.1-mvp/evidence/AC-008/RED-I035.md`

## Deferred to their phases
- P-020/P-021 (perf: concurrency 1 / 4 measurement), P-023 (0.21: saturation).

## Worktree / SHA
- HEAD SHA: `37225cb5348356d059fc9687af0ca3655c23084e` (uncommitted changes).
- `git status`:
  ```
   M src/grpc/classify.rs
   M src/lib.rs
  ?? specs/0.1-mvp/evidence/AC-008/RED-I035.md
  ?? src/handoff.rs
  ?? tests/queue.rs
  ```
- No commits/pushes.

## SUPERSEDED FACTS
The earlier LOCAL-GREEN.md (HEAD SHA `2e7629cc...`, unit scope only) stated that
I-035 was deferred to the integration phase and that AC-008's implementation was
solely the `BoundedQueue`/`QueueError` abstraction in `src/queue.rs`. That is
superseded: per ADR-0002, AC-008 now also requires the bounded queue to be IN the
request path. I-035 is implemented and GREEN, and the request-path bounded handoff
(`InferenceExecutor` in `src/handoff.rs`, wired through
`ClassifyServiceImpl::with_executor`) is part of AC-008's implementation.
