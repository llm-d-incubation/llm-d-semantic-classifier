# AC-009 RED evidence — I-001 (real tonic round trip)

## Criterion
AC-009 dummy gateway consumes response over persistent gRPC.

## Proving test (this slice)
- I-001 `i001_real_tonic_round_trip` (`tests/grpc.rs`, integration, async `#[tokio::test]`).

`specs/0.1-mvp/test-plan.md` maps AC-009 to I-001/I-002/I-005/I-006/I-008
(integration) and S-001/S-002 (Kubernetes system). This turn focuses on I-001
(the real tonic client/server round trip). I-002 (persistent channel) stays wired
and green in the same file; I-005/I-006/I-008 (dummy-the AI Gateway semantics) and
S-001/S-002 are deferred to later slices/phases within AC-009.

## RED state (prior compile failure cited as RED)
The RED for I-001 was recorded in the prior turn at
`specs/0.1-mvp/evidence/AC-009/RED.md`: the gRPC layer did not exist yet, so the
proving tests could not compile.

Command:
```
cargo test --locked --test grpc
```

Failure excerpt:
```
error[E0433]: cannot find `grpc` in `llm_d_sc`
  --> tests/grpc.rs:18:15
   |
18 | use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyResponse, ClassifyServer};
   |               ^^^^ could not find `grpc` in `llm_d_sc`
error: could not compile `llm-d-sc` (test "grpc") due to 1 previous error
```
Exit code: 101.

Why expected: AC-009 demands a persistent gRPC classify service that a dummy
the AI Gateway client can consume responses from. No such layer existed — no proto, no
`src/grpc`, no tonic dependency — so `llm_d_sc::grpc` was undefined and the
proving tests could not compile, let alone pass. The failure is deterministic and
non-vacuous: once a real tonic classify server + client exists, I-001 must pass.

## Worktree / SHA
- HEAD SHA: `a49dc474725486c24dc6764ec1d75f76e689e1e7` (uncommitted changes at RED time).
