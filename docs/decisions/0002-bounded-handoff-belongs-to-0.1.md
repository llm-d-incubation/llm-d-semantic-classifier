# ADR-0002: 0.1 requires the bounded queue to be IN the request path

Date: 2026-08-17
Status: Accepted
Context: worker ESCALATE — "BOUNDED-SCHEDULER" was not a defined 0.1 artifact

## The contradiction

The reviewer (acting on an external review) asked for a bounded scheduler between
Tonic and inference during 0.1. The worker correctly objected: `docs/VERSIONS.md`
assigns "bounded scheduler, deadlines, cancellation, load shedding, graceful
drain" to **0.20**, and no 0.1 acceptance criterion required the queue to be
wired. It escalated rather than implement unspecified work. That was correct
behaviour under docs/SDD.md.

## Decision

Split the concern by what each phase actually needs:

**0.1 (this phase, via AC-008):** the queue must be IN the request path. The 0.1
spec's In-Scope list already says "bounded MVP queue", and I-035 ("saturation
rejects rather than runaway queueing") cannot be honestly satisfied by a queue
that no request touches. Concretely 0.1 requires only:
  - the model forward does NOT execute on a Tokio network worker
  - a bounded handoff (channel/queue) sits between the handler and inference
  - a dedicated inference executor performs the forward
  - queue-full returns an explicit resource-exhausted status
  - I-035 proves bound + explicit overload + recovery

**0.20 (unchanged):** per-job deadlines, queued-request cancellation, load
shedding policy, graceful drain, worker-failure isolation, structured error
taxonomy. Those remain out of 0.1.

## Rationale

The trigger is measurement validity, not feature completeness. AC-011's whole
purpose is latency evidence; if 0.1 is benchmarked with the model forward running
on a network worker, the resulting p95/p99 describe an architecture we have
already decided not to ship (see research: "the async networking runtime must not
become the model execution scheduler"). Measuring the wrong architecture is worse
than not measuring.

## Process notes

1. The worker escalated correctly. The reviewer's slice prompt named work with no
   spec artifact behind it — a prompt defect, not a worker defect.
2. On retry the worker expanded `specs/0.20-runtime-hardening/` into full SDD
   fields. That work is retained as an UNREVIEWED DRAFT: authoring spec artifacts
   is the maintainer/reviewer role per CONTRIBUTING.md, and it was invited by a
   vague reviewer prompt rather than chosen by the worker. It must be reviewed
   before 0.20 begins, and it does not authorise 0.20 implementation now.
