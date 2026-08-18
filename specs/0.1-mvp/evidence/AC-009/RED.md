# AC-009 RED evidence — I-005 / I-006 (dummy gateway semantics)

## Criterion
AC-009 dummy gateway consumes response over persistent gRPC.

## Proving tests (this slice)
- I-005 `i005_dummy_gateway_preserves_session_metadata` (`tests/grpc.rs`, integration).
- I-006 `i006_dummy_gateway_routes_outside_llm_d_sc` (`tests/grpc.rs`, integration).

`specs/0.1-mvp/test-plan.md` maps AC-009 to I-001/I-002/I-005/I-006/I-008
(integration) and S-001/S-002 (Kubernetes system). I-001/I-002 (real tonic round
trip + persistent HTTP/2 channel) are already RED/GREEN in this same file and
are committed. I-008 (multi-turn requests do not reconnect per call) is asserted
by I-002 via `channel_reconnect_count() == 0`.

This slice selects the dummy-the AI Gateway semantics: I-005 (the dummy gateway preserves
the session metadata it propagates: request_id/session_id/context/signals/deadline
are passed through and kept intact for its own routing decision) and I-006 (the
dummy gateway consumes the ranked signal, then routes OUTSIDE llm-d-sc via its
fixed test-only mapping NEVER_EGRESS -> local-model / otherwise -> general-model,
recording route + classifier RTT). Both prove the responsibility split that
routing/session authority stays the AI Gateway (AC-010), which is the essence of AC-009.

## RED state (feature does not exist)
There is no `dummy_gateway` module in the crate. `src/lib.rs` exposes only
`cache`, `classify`, `config`, `embedding`, `grpc`, `queue`, `ranker`,
`runtime`, `tokenizer`. The proving tests drive a
`llm_d_sc::dummy_gateway::DummyGateway` client (connect over the persistent
channel, classify_and_route, DummyRequest/DummyOutcome) against the real
classify server. Because `llm_d_sc::dummy_gateway` is undefined, neither proving
test can be selected or compiled.

## Command
```
cargo test --locked --test grpc
```

## Result
FAILED. Expected RED reason: the dummy-the AI Gateway layer (AC-009 I-005/I-006) does
not exist yet, so the proving tests cannot compile, let alone pass.

Failure excerpt:
```
error[E0432]: unresolved import `llm_d_sc::dummy_gateway`
   --> tests/grpc.rs:136:19
    |
136 |     use llm_d_sc::dummy_gateway::{DummyGateway, DummyRequest};
    |                   ^^^^^^^^^^^^ could not find `dummy_gateway` in `llm_d_sc`

error[E0432]: unresolved import `llm_d_sc::dummy_gateway`
   --> tests/grpc.rs:173:19
    |
173 |     use llm_d_sc::dummy_gateway::{DummyGateway, DummyRequest};
    |                   ^^^^^^^^^^^^ could not find `dummy_gateway` in `llm_d_sc`

error: could not compile `llm-d-sc` (test "grpc") due to 2 previous errors
```
Exit code: 101.

## Why this is the expected failure
AC-009 demands the dummy gateway consume the classification response over a
persistent gRPC channel and then apply its own routing outside llm-d-sc. The
persistent gRPC layer already exists (I-001/I-002 green), but the dummy-the AI Gateway
client that receives a synthetic request, propagates session metadata, consumes
the ranked signal, and routes outside llm-d-sc does not yet exist. Because
`llm_d_sc::dummy_gateway` is undefined, `cargo test --locked --test grpc` fails at
exit 101 with "could not find `dummy_gateway` in `llm_d_sc`". This is precisely
the expected RED: the feature (dummy-the AI Gateway semantics) does not exist, so the
proving tests cannot run, let alone pass. The failure is deterministic and
non-vacuous — once a `src/dummy_gateway.rs` exposing `DummyGateway::connect`,
`DummyGateway::classify_and_route`, `DummyRequest`, and `DummyOutcome` is
implemented, I-005 and I-006 become selectable and must pass.

## Worktree / SHA
- HEAD SHA: `83063564edd0eddade63d7de7b399c7015fe8ee8` (uncommitted change).
- `git status`:
  ```
   M tests/grpc.rs
  ```
  `tests/grpc.rs` holds the I-005/I-006 proving tests (test-only; the dummy
  module intentionally not yet implemented). No commits/pushes.
