# ADR-0001: The classify response schema contains no route field

Date: 2026-08-16
Status: Accepted
Context: worker ESCALATE on AC-010 (spec vs committed code contradiction)

## Contradiction

`tests/TEST_MATRIX.md` U-010 requires: "classification response schema contains
no final route/endpoint". Commit 8306356 nonetheless shipped
`proto/classify.proto` with `optional string final_route = 3` in
`ClassifyResponse`, commented "llm-d-sc must NEVER set this field", and AC-009's
evidence proved the property by asserting `final_route.is_none()`.

Two mutually exclusive readings:
- (A) keep the field, forbid setting it → nothing to prove, U-010 vacuous
- (B) remove the field → U-010 is a real schema invariant

## Decision: (B). The field is removed.

Rationale:
1. U-010 is about the SCHEMA, not runtime behavior. A response type that *can*
   express a route fails the criterion as written.
2. "Must never set" makes a safety property depend on implementer discipline
   forever. Praxis owns routing (spec: State/Non-goals); the wire contract
   should make the alternative unrepresentable, not merely discouraged.
3. `src/classify.rs`'s `ClassificationResult` already has no route field — the
   proto was inconsistent with the typed core it serializes.
4. It is a latent hazard: any future handler could populate it, and any future
   Praxis could start reading it, silently migrating routing authority.

## Reviewer accountability

The AC-009 review (verdict c93a12b) asserted "the response type structurally
cannot carry a route" on the basis of `src/classify.rs` and did not inspect the
proto. That was a review miss; the worker caught it by refusing to fabricate a
RED. Review checklist gains: for wire-contract criteria, read the .proto, not
only the Rust types.

## Consequences

- Remove `final_route` from `ClassifyResponse`.
- AC-009's `final_route.is_none()` assertion is replaced by the U-010 schema
  invariant (privileged existing-test change, justified here).
- U-010 becomes a deterministic schema test that is RED while the field exists.
