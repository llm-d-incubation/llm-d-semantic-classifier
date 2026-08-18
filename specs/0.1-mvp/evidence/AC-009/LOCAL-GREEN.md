# AC-009 GREEN evidence — dummy gateway consumes response over persistent gRPC

## Criterion
AC-009 dummy gateway consumes response over persistent gRPC.

## Tests mapped in test-plan.md
`specs/0.1-mvp/test-plan.md` maps AC-009 to I-001/I-002/I-005/I-006/I-008
(integration) and S-001/S-002 (Kubernetes system). All integration-level proving
tests pass, so the whole-criterion GREEN.md for the local (worker) scope is
written here. PROMOTION-GREEN.md is reserved for when the Kubernetes system tier
(S-001/S-002) also passes and is never written by the worker.

## Commands & results
```
cargo test --locked --test grpc
```
PASSED — 4 passed; 0 failed:
```
running 4 tests
test i001_real_tonic_round_trip ... ok
test i006_dummy_gateway_routes_outside_llm_d_sc ... ok
test i005_dummy_gateway_preserves_session_metadata ... ok
test i002_persistent_http2_channel_reused ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Full suite (`./hack/test-all`): 22 passed; 0 failed; 5 ignored (Candle model
tests, run explicitly after `./hack/fetch-model`); grpc suite 4/4.

## Implementation
- I-001/I-002 (committed in prior slices): real tonic round trip over a
  persistent HTTP/2 channel; I-008 (multi-turn no reconnect per call) is asserted
  by I-002 via `channel_reconnect_count() == 0`.
- This slice (smallest change): added `src/dummy_gateway.rs` and registered
  `pub mod dummy_gateway;` in `src/lib.rs`.
  - `DummyGateway::connect` reuses the existing persistent `ClassifyClient`.
  - `DummyGateway::classify_and_route` propagates session metadata verbatim
    (request_id/session_id/context/signals/deadline), consumes the top ranked
    signal, and applies a fixed test-only mapping (`NEVER_EGRESS_SIGNAL "proto-a"`
    -> `"local-model"`, otherwise -> `"general-model"`), recording route +
    classifier RTT.
  - Routing/session authority stays outside llm-d-sc (AC-010): the response never
    carries a final route.

## Evidence files
- `specs/0.1-mvp/evidence/AC-009/RED.md` (I-005/I-006 RED)
- `specs/0.1-mvp/evidence/AC-009/GREEN-I005-I006.md`

## Deferred to their phase
- S-001/S-002 (Kubernetes sidecar + ClusterIP benchmark) — system tier, not run by
  the worker; required for PROMOTION-GREEN only.

## Worktree / SHA
- HEAD SHA: `83063564edd0eddade63d7de7b399c7015fe8ee8` (uncommitted changes).
- `git status --short`:
  ```
   M .agent/state/current.md
   M specs/0.1-mvp/evidence/AC-009/RED.md
   M src/lib.rs
   M tests/grpc.rs
  ?? specs/0.1-mvp/evidence/AC-009/GREEN-I005-I006.md
  ?? specs/0.1-mvp/evidence/AC-009/LOCAL-GREEN.md
  ?? src/dummy_gateway.rs
  ```
- No commits/pushes.

## SUPERSEDED FACTS (reviewer, 2026-08-17)
This summary states that dummy gateway propagates a **deadline**. It does not: `deadline` is
absent from `ClassifyRequest` in the protobuf and `DummyGateway::classify_and_route()` drops
it. Deadline propagation is I-003, a **0.20** hardening test — the code is correct for 0.1;
this evidence text was wrong. Also note the served path uses the deterministic synthetic
pipeline, not `CandleClassifier`; wiring the real runtime is tracked as the integration
convergence work.

## SUPERSEDED FACTS (convergence slice 1, 2026-08-17)
The prior whole-criterion summaries (e.g. `GREEN-I001.md`) describe the response as carrying
"request_id + ranked signals only" with `signals: repeated string` (signal names only). That
is superseded by Convergence Slice 1: `ClassifyResponse` now carries `classifier_id`,
`model_revision`, `tokenizer_revision`, `taxonomy_revision`, a `ClassificationStatus`, and
`repeated RankedSignal ranked` (each with `label` + `score`). Scores/revisions/status are now
on the wire; `requested_signals` is validated (U-011). See `CONTRACT-lock.md` in this
directory.
