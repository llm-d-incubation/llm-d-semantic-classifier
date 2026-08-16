# AC-010 GREEN evidence — U-010 response schema contains no route field

## Test ID
U-010 (test-plan.md maps AC-010 to U-010, I-007).

## Test file
`tests/schema.rs` — `u010_response_schema_has_no_route_field` (plain,
deterministic `#[test]`; no network, no async).

## Command
```
cargo test --test schema --locked
```

## Result: GREEN
```
running 2 tests
test u010_generated_response_type_exists ... ok
test u010_response_schema_has_no_route_field ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Implementation (smallest change)
- Removed `optional string final_route = 3;` from `message ClassifyResponse` in
  `proto/classify.proto` (ADR-0001 interpretation (B)). The schema now carries
  only `request_id` and `signals`; a route is unrepresentable on the wire.
- Removed the `final_route: None,` field from the handler response construction
  in `src/grpc/classify.rs`.
- The U-010 test parses the message's actual field declarations and asserts none
  of `final_route`/`route`/`endpoint`/`target` is a field name.

## Worktree / SHA
- HEAD SHA: `e6361b73c4865d14fee6147a218463d9ec30099f` (working-tree changes).

## References
- `docs/decisions/0001-no-route-field-in-response.md` (decision (B))
- `tests/TEST_MATRIX.md` U-010
