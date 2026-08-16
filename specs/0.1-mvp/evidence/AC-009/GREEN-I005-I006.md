# AC-009 GREEN evidence — I-005 / I-006 (dummy Praxis semantics)

## Criterion
AC-009 dummy Praxis consumes response over persistent gRPC.

## Proving tests (this slice)
- I-005 `i005_dummy_praxis_preserves_session_metadata` (`tests/grpc.rs`, integration).
- I-006 `i006_dummy_praxis_routes_outside_llm_d_sc` (`tests/grpc.rs`, integration).

## Implementation (smallest change)
Added `src/dummy_praxis.rs` and wired `pub mod dummy_praxis;` in `src/lib.rs`.
`DummyPraxis` connects over the existing persistent `ClassifyClient`, propagates
session metadata verbatim, consumes the top ranked signal, and applies a fixed
test-only mapping (`NEVER_EGRESS_SIGNAL "proto-a" -> "local-model"`, otherwise ->
`"general-model"`), recording route + classifier RTT. Routing authority stays
outside llm-d-sc (AC-010). No changes to the existing gRPC layer.

## Command
```
cargo test --locked --test grpc i005
cargo test --locked --test grpc i006
```

## Result
PASSED (GREEN).

I-005:
```
running 1 test
test i005_dummy_praxis_preserves_session_metadata ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
```

I-006:
```
running 1 test
test i006_dummy_praxis_routes_outside_llm_d_sc ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
```

Full suite `cargo test --locked --test grpc` (4/4, no regression):
```
running 4 tests
test i001_real_tonic_round_trip ... ok
test i006_dummy_praxis_routes_outside_llm_d_sc ... ok
test i005_dummy_praxis_preserves_session_metadata ... ok
test i002_persistent_http2_channel_reused ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Worktree / SHA
- HEAD SHA: `83063564edd0eddade63d7de7b399c7015fe8ee8` (uncommitted changes).
- `git status --short`:
  ```
   M .agent/state/current.md
   M specs/0.1-mvp/evidence/AC-009/RED.md
   M src/lib.rs
   M tests/grpc.rs
  ?? src/dummy_praxis.rs
  ```
  No commits/pushes.
