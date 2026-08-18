# AC-008 GREEN evidence — I-035 (bounded queue IN the request path)

## Test ID
I-035 `i035_saturation_rejects_rather_than_runaway_queueing` (`tests/queue.rs`,
`#[tokio::test]`, offline — a slow classifier + small queue bound, no model fetch).

## Command
```
cargo test --locked --test queue i035
```

## Result
PASSED. 1 passed; 0 failed.

```
running 1 test
test i035_saturation_rejects_rather_than_runaway_queueing ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
```

## What changed (smallest implementation, per ADR-0002)
The bounded queue is now wired INTO the request path — the model forward does NOT
run on a Tokio network worker.

- `src/handoff.rs` (new): `InferenceExecutor<R>` — a bounded handoff
  (`tokio::sync::mpsc::channel(bound)` + an owned semaphore of `bound` permits)
  between the gRPC handler and a DEDICATED executor thread. The executor thread
  performs `service.classify(...)` (the forward) off Tokio network workers and
  returns the result via a `oneshot`. Admission beyond the bound returns
  `QueueFull`; the total of in-flight + queued work never exceeds `bound`.
- `src/grpc/classify.rs`: `ClassifyServiceImpl` gained a `with_executor`/`new`
  constructor that spawns the dedicated executor, `queue_bound()` and
  `max_admitted()` accessors. The `classify` handler no longer runs the forward
  inline on the network worker; it calls `executor.try_enqueue(input)` and maps
  `QueueFull` to `tonic::Status::resource_exhausted("inference queue is full")`,
  then awaits the oneshot result.
- Queue wait is recorded through the existing `Metrics` Queue stage: the
  executor thread calls `metrics.record_stage(LatencyStage::Queue, queued_at.elapsed())`
  when a job's forward begins.

I-035 drives a real tonic client/server with a slow forward
(`SlowClassifier`, 50 ms sleep) behind a small bound (`QUEUE_BOUND = 3`) and
fires `SATURATION_LOAD = 20` concurrent requests. It asserts:
1. explicit resource-exhausted responses appear under saturation;
2. `max_admitted <= QUEUE_BOUND` (in-flight + queued never exceed the bound);
3. `snapshot().queue > Duration::ZERO` (queue wait recorded through the Queue
   metrics stage);
4. a fresh request succeeds after the saturation load drains (recovery).

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
- `src/lib.rs` registers `pub mod handoff;`. No commits/pushes.
