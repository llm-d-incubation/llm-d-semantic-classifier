# AC-009 RED evidence — I-001 / I-002 (slice)

## Criterion
AC-009 dummy Praxis consumes response over persistent gRPC.

## Proving tests (this slice)
- I-001 `i001_real_tonic_client_server_round_trip` (`tests/grpc.rs`, integration).
- I-002 `i002_persistent_http2_channel_reused` (`tests/grpc.rs`, integration).

`specs/0.1-mvp/test-plan.md` maps AC-009 to I-001/I-002/I-005/I-006/I-008
(integration) and S-001/S-002 (OpenShift system). This slice selects the
persistent-gRPC round-trip contract (I-001: a real tonic client/server round
trip returns ranked signals; I-002: the HTTP/2 channel is persistent and reused
across turns, so multi-turn requests do not reconnect per call — I-008). The
dummy-Praxis semantic tests (I-005/I-006/I-008) and OpenShift system tests
(S-001/S-002) are deferred to later slices/phases within AC-009.

## Why these are the proving tests for AC-009
AC-009 requires the dummy Praxis (client) to consume the classification response
over a persistent gRPC channel. I-001 pins the round-trip contract: a real
classify server + client exchange a request and a response carrying ranked
semantic signals (and never a final route, per AC-010). I-002 pins the
PERSISTENT channel: several turns must reuse the same HTTP/2 channel and must
not reconnect per call. Together they capture the essence of AC-009 ("consumes
response over persistent gRPC") at the integration seam.

## RED state (feature does not exist)
There is no gRPC layer in the crate yet: no `.proto` contract, no `src/grpc`
module, no tonic dependency, and no `tests/grpc` integration target. The crate
only ships `cache`, `config`, `embedding`, `queue`, `ranker`, `runtime`,
`tokenizer` (`src/lib.rs`). This slice adds the test-only `tests/grpc.rs`
holding I-001/I-002, which reference
`llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyResponse, ClassifyServer}`.
Because `llm_d_sc::grpc` is undefined, neither proving test can be selected or
compiled.

## Command
```
cargo test --locked --test grpc
```

## Result
FAILED. Expected RED reason: the persistent-gRPC feature (AC-009) does not exist
yet, so the proving tests cannot compile, let alone pass.

Failure excerpt:
```
error[E0433]: cannot find `grpc` in `llm_d_sc`
  --> tests/grpc.rs:18:15
   |
18 | use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyResponse, ClassifyServer};
   |               ^^^^ could not find `grpc` in `llm_d_sc`

For more information about this error, try `rustc --explain E0433`.
error: could not compile `llm-d-sc` (test "grpc") due to 1 previous error
```
Exit code: 101.

## Why this is the expected failure
AC-009 demands a persistent gRPC classify service that a dummy Praxis client can
consume responses from. No such layer exists: the crate implements only
cache/runtime/config/ranking from earlier criteria. Because `llm_d_sc::grpc` is
undefined, `cargo test --locked --test grpc` fails at exit 101 with "cannot find
`grpc` in `llm_d_sc`". This is precisely the expected RED: the feature
(persistent gRPC round trip) does not exist, so the proving tests cannot run,
let alone pass. The failure is deterministic and confirms the tests are
non-vacuous — once a `src/grpc` layer exposing `classify::{ClassifyServer,
ClassifyClient, ClassifyRequest, ClassifyResponse}` is implemented, I-001 and
I-002 become selectable and must pass.

Note: per AGENTS.md steps 1-4 only the RED proof and evidence are required before
implementation. `./hack/verify` is NOT run this iteration; the GREEN step will
add the minimal gRPC layer (proto contract + server + client + persistent
channel) so I-001/I-002 compile and pass, then I-005/I-006/I-008 and S-001/S-002
are exercised in their later slices/phases.

## Worktree / SHA
- HEAD SHA: `a49dc474725486c24dc6764ec1d75f76e689e1e7` (uncommitted changes).
- `git status`:
  ```
   ?? tests/grpc.rs
  ```
  `tests/grpc.rs` holds the I-001/I-002 proving tests (test-only; the gRPC layer
  intentionally not yet implemented). No commits/pushes.
