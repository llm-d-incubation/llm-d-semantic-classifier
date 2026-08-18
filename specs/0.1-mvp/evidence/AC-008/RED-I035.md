# AC-008 RED evidence — I-035 (bounded queue IN the request path)

## Proving test
- I-035 `i035_saturation_rejects_rather_than_runaway_queueing` (`tests/queue.rs`,
  `#[tokio::test]`, offline — a slow classifier + small queue bound, no model
  fetch).

`specs/0.1-mvp/test-plan.md` maps AC-008 to U-030/U-031 (unit, already GREEN),
I-035 (integration: queue IN the request path per ADR-0002), and P-020/P-021
(perf, later). This slice selects I-035 as the proving test for the request-path
handoff: it drives a real tonic client/server, saturates the service with a slow
forward behind a small bound, and asserts:
1. explicit resource-exhausted responses appear under saturation;
2. in-flight + queued work never exceeds the configured bound;
3. queue wait is recorded through the existing `Metrics` Queue stage;
4. the service recovers after load stops (a fresh request succeeds).

## Why these are the proving test for AC-008
AC-008 (per ADR-0002) requires the bounded queue to be IN the request path: the
model forward must NOT run on a Tokio network worker; a bounded handoff sits
between the gRPC handler and a dedicated inference executor; queue-full returns
an explicit resource-exhausted status; and I-035 proves bound + explicit
overload + recovery. The unit tests U-030/U-031 only prove the `BoundedQueue`
abstraction in isolation; I-035 proves the queue is actually wired into the
request path, which U-030/U-031 cannot.

## RED state (feature does not exist)
`src/grpc/classify.rs` currently runs `self.service.classify(input)` DIRECTLY in
the async tonic handler — the model forward executes on a Tokio network worker.
There is no bounded handoff, no dedicated inference executor, no `with_executor`
constructor, and no `max_admitted`/`queue_bound` observability. The proving test
cannot compile.

## Command
```
cargo test --locked --test queue i035
```

## Result
FAILED. Expected RED reason: the request-path bounded handoff (AC-008 / ADR-0002)
does not exist, so the proving test cannot compile, let alone pass.

Failure excerpt:
```
error[E0599]: no associated function or constant named `with_executor` found for struct `ClassifyServiceImpl<R>` in the current scope
  --> tests/queue.rs:88:40
   |
88 |     let service = ClassifyServiceImpl::with_executor(
   |                                        ^^^^^^^^^^^^^ associated function or constant not found in `ClassifyServiceImpl<_>`
```
Exit code: 101.

## Why this is the expected failure
AC-008 (as split by ADR-0002) demands a bounded handoff between the gRPC handler
and a dedicated inference executor, with queue-full returning resource-exhausted.
No such wiring exists: `ClassifyServiceImpl` has no `with_executor` constructor
and no `max_admitted`/`queue_bound` accessors, and the forward still runs on the
network worker. Because the feature is absent, `cargo test --locked --test queue
i035` fails at exit 101 with "no associated function or constant named
`with_executor`". This is precisely the expected RED: the feature (bounded queue
wired into the request path) does not exist, so the proving test cannot run. The
failure is deterministic and confirms the test is non-vacuous — once a bounded
handoff + dedicated executor with `with_executor`/`max_admitted` is implemented,
I-035 becomes selectable and must pass.

## Worktree / SHA
- HEAD SHA: `37225cb5348356d059fc9687af0ca3655c23084e` (uncommitted changes).
- `git status`:
  ```
   M hack/spec-check
  ?? hack/test-report
  ?? tests/queue.rs
  ```
  (`tests/queue.rs` holds the I-035 proving test; the bounded handoff is
  intentionally not yet implemented.) No commits/pushes.

## Note
Per AGENTS.md steps 1-4 only the RED proof and evidence are required before
implementation; `./hack/verify` is not run this iteration. The GREEN/implementation
step adds the minimal bounded handoff (`InferenceExecutor` in `src/handoff.rs`)
wired between the gRPC handler and a dedicated executor thread, maps queue-full
to tonic `resource_exhausted`, records queue wait through the existing `Metrics`
Queue stage, and exposes `max_admitted`/`queue_bound` so I-035 can prove the
bound.
