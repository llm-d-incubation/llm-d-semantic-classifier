# AC-010 RED evidence — U-010 response schema must contain no route field

## Test ID
U-010 (test-plan.md maps AC-010 to U-010, I-007).

## Test file
`tests/schema.rs` — `u010_response_schema_has_no_route_field` (a plain,
deterministic `#[test]`; no network, no async).

## Command
```
cargo test --test schema --locked
```

## Worktree / SHA
- HEAD SHA: `e6361b73c4865d14fee6147a218463d9ec30099f`
- Working tree: `tests/schema.rs` added (uncommitted); `proto/classify.proto`
  still contains `optional string final_route = 3`.

## Result: RED (expected)
```
running 2 tests
test u010_generated_response_type_exists ... ok
test u010_response_schema_has_no_route_field ... FAILED

---- u010_response_schema_has_no_route_field stdout ----
thread 'u010_response_schema_has_no_route_field' panicked at tests/schema.rs:55:9:
U-010: ClassifyResponse schema must not contain a `final_route` field (ADR-0001)

test result: FAILED. 1 passed; 1 failed
```

## Why this is the expected failure
The committed schema (`proto/classify.proto`) still declares
`optional string final_route = 3;` inside `message ClassifyResponse`. ADR-0001
(interpretation (B)) requires the field be REMOVED entirely, so the schema
invariant U-010 is RED while the field exists. The failure is caused by exactly
the forbidden `final_route` field, matching the criterion.

## References
- `docs/decisions/0001-no-route-field-in-response.md` (decision (B))
- `tests/TEST_MATRIX.md` U-010
