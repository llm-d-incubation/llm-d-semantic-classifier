# AC-010 GREEN evidence — response contains signals, not final route

## Criterion
AC-010 response contains signals, not final route.

## Adjudication
Resolved by `docs/decisions/0001-no-route-field-in-response.md`: interpretation
(B) is authoritative — the `final_route` field is REMOVED from the schema
entirely. A route must be unrepresentable on the wire, not merely "never set";
U-010 is a SCHEMA invariant.

## Tests mapped in test-plan.md
`specs/0.1-mvp/test-plan.md` maps AC-010 to U-010 (schema) and I-007
(integration). Both pass at the worker (local) scope.

## Commands & results
```
cargo test --test schema --locked
```
PASSED — 2 passed; 0 failed (U-010 + generated-type surface).

```
cargo test --test grpc --locked
```
PASSED — 5 passed; 0 failed (incl. new I-007).

## Implementation (smallest change)
1. Removed `optional string final_route = 3;` from `ClassifyResponse` in
   `proto/classify.proto` (ADR-0001 (B)); the message now carries only
   `request_id` and `signals`.
2. Removed the `final_route: None,` field from the handler response construction
   in `src/grpc/classify.rs`.
3. Added `tests/schema.rs` — U-010 deterministic schema test that reads the
   committed `proto/classify.proto`, parses `ClassifyResponse`'s actual field
   declarations, and asserts none of `final_route`/`route`/`endpoint`/`target`
   is a field name.
4. Added `tests/grpc.rs` `i007_response_cannot_dictate_endpoint` — drives the
   dummy gateway against the real server and asserts the only route in the system
   is the dummy's own test-only mapping; the response type offers no route to
   consume.

## Privileged existing-test change (authorized by ADR-0001)
`tests/grpc.rs` `i001_real_tonic_round_trip` previously asserted
`response.final_route.is_none()` (AC-009 evidence). That assertion is replaced by
the U-010 schema invariant: `ClassifyResponse` has no route field, so a route is
unrepresentable on the wire (ADR-0001 "Consequences": AC-009's
`final_route.is_none()` assertion is replaced by the U-010 schema invariant).
No assertion was weakened — the property is now enforced by the type/schema
rather than runtime discipline.

## Evidence files
- `specs/0.1-mvp/evidence/AC-010/RED-U010.md`
- `specs/0.1-mvp/evidence/AC-010/GREEN-U010.md`
- `specs/0.1-mvp/evidence/AC-010/GREEN-I007.md`

## Worktree / SHA
- HEAD SHA: `e6361b73c4865d14fee6147a218463d9ec30099f` (working-tree changes).
- `git status --short`:
  ```
   M .agent/state/current.md
   M proto/classify.proto
   M src/grpc/classify.rs
   M tests/grpc.rs
   M tests/TEST_MATRIX.md
   ?? docs/decisions/
   ?? specs/0.1-mvp/evidence/AC-010/
   ?? tests/schema.rs
  ```
- No commits/pushes.
