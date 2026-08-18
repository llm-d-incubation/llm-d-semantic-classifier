# 0.20 Runtime Hardening — Phase Specification

## Problem
The 0.1 MVP proves service shape but leaves the inference path untrustworthy under
real load: admission beyond the bounded queue is only an enum variant, there is
no per-job deadline or queued cancellation, no load shedding, no worker-failure
isolation, and no graceful drain. Under sustained load the service must degrade
explicitly, never unboundedly, and must shut down without dropping in-flight work.

## Upstream context
Phase 0.20 (VERSIONS.md) makes the service trustworthy before making it clever.
`specs/0.1-mvp/spec.md` AC-008 already declares the failure contract (full queue ->
explicit resource exhausted; expired queued request -> do not infer), but the 0.1
slice only proved the bounded FIFO (U-030/U-031). This phase implements the rest of
the scheduler contract and the drain/structured-error behavior it implies.

## Existing behavior
- `src/queue.rs` `BoundedQueue` is a capacity-bounded FIFO with only
  `QueueError::ResourceExhausted` (U-030/U-031 proven).
- `src/classify.rs` declares `ClassifyError::{ResourceExhausted, RequestExpired}` but
  nothing in the pipeline enforces deadlines or cancellation; the enum variants are
  not exercised by the queue path.
- No load-shedding (oversize rejection before model work).
- No worker abstraction; a runtime error is not isolated per request.
- No graceful drain on shutdown; readiness is a single `Runtime::Readiness` gate.

## Desired behavior (BOUNDED-SCHEDULER slice)
The bounded scheduler must, in one coherent slice:

1. Keep admission bounded: saturation rejects explicitly with resource-exhausted and
   never buffers without limit; an overload counter increments; bounded overload is
   preserved under CPU pressure.
2. Enforce a per-job deadline: a queued job whose deadline has expired is discarded
   and NEVER executes forward; the caller receives an explicit request-expired error;
   stale queued jobs are discarded on deadline expiry.
3. Honor queued cancellation: caller cancellation removes the queued job/waiter so it
   never executes forward and does not leak a waiter/job.
4. Isolate worker failures: a worker failure resolves its waiter with an explicit
   error and one runtime error never poisons unrelated requests.
5. Shed load before model work: oversize requests are rejected before tokenization or
   any model forward.
6. Drain gracefully: shutdown stops admission and drains configured in-flight work;
   repeated start/shutdown sequences do not deadlock.

## Non-goals
- Routing policy, stickiness, or endpoint selection (llm-d-sc never routes; AC-010).
- Distributed scheduler or cross-replica coordination.
- Liveness/readiness *distinction* beyond the existing readiness gate (separate slice).
- Metric-cardinality bounds, prompt-redaction, and structured-error taxonomy beyond
  the scheduler's own errors (separate slices).
- Unrestricted model forward from Tokio request workers: the bounded scheduler still
  owns concurrency; Tokio never forwards the model directly.
- No unbounded inference queues.

## Compatibility
- The existing `BoundedQueue` API and `ClassifyError` variants remain; the slice adds
  the deadline/cancellation/drain semantics that the contract already promises.
- The gRPC status taxonomy (I-004) must map the new error outcomes without changing
  the wire contract for already-successful calls.

## Security impact
- Oversize rejection before tokenization bounds memory and CPU before any model work
  (R-009/R-010).
- No raw prompt enters default logs/metrics (unchanged, AC-014).

## Rollback
- The scheduler is internal; disabling the slice means reverting to the 0.1 bounded
  FIFO behavior, which still satisfies the capacity contract (U-030/U-031). No
  configuration surface is added for the scheduler in this slice.

## Acceptance criteria (machine-verifiable; one worker turn each)
- [ ] AC-001 Bounded admission and saturation rejection: queue capacity is bounded;
      admission beyond capacity returns resource-exhausted; overload counter increments;
      bounded overload holds under CPU pressure; randomized load never exceeds the bound.
- [ ] AC-002 Per-job deadline: an expired queued job never executes forward; the caller
      gets an explicit request-expired error; stale queued jobs are discarded on expiry.
- [ ] AC-003 Queued cancellation: a cancelled queued job never executes forward; caller
      cancellation does not leak a waiter/job.
- [ ] AC-004 Worker-failure isolation: a worker failure resolves its waiter with an
      explicit error; one runtime error cannot poison unrelated requests.
- [ ] AC-005 Load shedding: oversize requests are rejected before tokenization/model work.
- [ ] AC-006 Graceful drain: shutdown stops admission and drains configured in-flight
      work; SIGTERM flips readiness before drain; graceful termination under active calls;
      repeated start/shutdown does not deadlock.

## Negative cases (must continue to fail / remain unchanged)
- N-1 A full queue must continue to reject, never buffer unboundedly (U-030/U-031).
- N-2 No fabricated label on any scheduler error path (failure contract).
- N-3 The response must never contain a route/endpoint field (AC-010).

## Open questions
None blocking. Deadline/cancellation use the existing `std::time` and Tokio
cancellation primitives already in the dependency graph; no new dependency is
introduced without a design note.

> **UNREVIEWED WORKER DRAFT** (ADR-0002): authored by the implementation
> worker in response to a vague reviewer prompt. Spec authoring is the
> maintainer/reviewer role. Must be reviewed before 0.20 begins; does NOT
> authorise 0.20 implementation.
