# Test plan — 0.20-runtime-hardening / BOUNDED-SCHEDULER slice

## Strategy (unit / integration / e2e / regression per criterion)

| AC | Required evidence |
|---|---|
| AC-001 | U-030, U-031, I-035, I-081, S-040, R-013 |
| AC-002 | U-032, I-036 |
| AC-003 | U-033, I-037 |
| AC-004 | U-034, U-036 |
| AC-005 | U-013 |
| AC-006 | U-035, I-013, S-042, R-015 |

## Impact analysis
Run `./hack/test-impact src/queue.rs src/classify.rs src/runtime.rs src/bin/server.rs`.
Expected Required: queue, grpc, restart, metrics. Recommended: schema (unchanged),
telemetry.

## Mocks
A mock worker (a test double returning a canned failure) is required for AC-BS-4 to
prove worker-failure isolation without a real model forward. Justification recorded in
the test file: the failing-forward path cannot be exercised deterministically with the
deterministic pipeline, so a stub worker is needed.

> **UNREVIEWED WORKER DRAFT** (ADR-0002): authored by the implementation
> worker in response to a vague reviewer prompt. Spec authoring is the
> maintainer/reviewer role. Must be reviewed before 0.20 begins; does NOT
> authorise 0.20 implementation.
