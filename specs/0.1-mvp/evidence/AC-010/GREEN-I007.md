# AC-010 GREEN evidence — I-007 llm-d-sc response cannot dictate endpoint

## Test ID
I-007 (test-plan.md maps AC-010 to U-010, I-007).

## Test file
`tests/grpc.rs` — `i007_response_cannot_dictate_endpoint`.

## Command
```
cargo test --test grpc --locked
```

## Result: GREEN
```
running 5 tests
test i001_real_tonic_round_trip ... ok
test i005_dummy_gateway_preserves_session_metadata ... ok
test i006_dummy_gateway_routes_outside_llm_d_sc ... ok
test i007_response_cannot_dictate_endpoint ... ok
test i002_persistent_http2_channel_reused ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## What it proves
The dummy gateway receives a response over the real persistent channel and routes
outside llm-d-sc. The ONLY route in the system is the one the dummy computes
itself: the outcome's `route` is exactly one of the dummy's fixed test-only
mappings, chosen purely from the consumed ranked signal. The response type
offers no route to consume — `ClassifyResponse` exposes no
`route`/`endpoint`/`final_route` field (ADR-0001), enforced by the U-010 schema
invariant in `tests/schema.rs`; referencing such a field would not compile.

## References
- `docs/decisions/0001-no-route-field-in-response.md` (decision (B))
- `tests/TEST_MATRIX.md` I-007
