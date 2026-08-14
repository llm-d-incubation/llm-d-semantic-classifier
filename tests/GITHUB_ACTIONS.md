# GitHub Actions Validation

GitHub Actions validates clean-room reproducibility and deterministic behavior. Shared hosted runners should not be the authority for a strict 20 ms p99 service SLO.

## Every push / PR: fast CI

Required:
- exact SHA checkout;
- pinned Rust toolchain;
- `cargo fmt --check`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- clean workspace build;
- unit tests;
- impact-selected integration tests;
- protobuf/API generation consistency;
- spec-check and acceptance/test-ID mapping;
- detection/surfacing of existing test deletion/assertion changes;
- service container build;
- small local ModelCar-layout fixture test;
- dependency/license/source policy checks;
- secret scanning according to repo policy.

Use read-only repository permissions by default. Jobs executing untrusted code do not receive reviewer/model/homelab credentials.

## Promotion CI: trusted/review-ready SHA

Run:
- complete deterministic suite;
- download real sensitivity model at pinned immutable revision;
- build OCI ModelCar;
- push/pull through ephemeral/local OCI registry;
- verify `/models` layout and non-root readability;
- run llm-d-sc against materialized artifact;
- dummy Praxis gRPC E2E;
- golden embedding/classification parity;
- cache-hit/cache-miss functional tests;
- overload/deadline integration tests;
- readiness and graceful shutdown;
- artifact digest/evidence report;
- SBOM/provenance when project policy matures.

The real model should not have to download on every tiny WIP push if this slows iteration. Fast CI can use a tiny structural fixture; promotion/nightly CI uses the pinned real artifact.

## What GitHub may hard-gate about performance

Hard deterministic invariants:
- cache hit performs zero forwards;
- queue/in-flight count never exceeds configured bounds;
- expired queued jobs do not run;
- no reconnect-per-request behavior in integration test;
- gross hang/timeout guard.

Non-blocking trend artifacts on shared runners:
- cache lookup microbenchmark;
- tokenizer microbenchmark;
- protobuf encode/decode;
- model-forward smoke benchmark.

Do **not** hard-gate a 20 ms p99 on a shared hosted runner.

## Nightly/scheduled

- full dependency/advisory checks;
- broader fuzz/property suite;
- real ModelCar rebuild from pinned source;
- concurrency/stress/soak;
- deadlock/leak checks;
- optional benchmark trends;
- supported platform/toolchain matrix.

## Promotion/merge discipline

The exact SHA that passed independent review is the SHA tested. If target `main` materially advances, recreate the validation ref and rerun required prod-like tests before merge.

## Public repository safety

Do not execute arbitrary fork PR workflows on a trusted homelab/LAN runner. If specialized hardware is automated later, use an isolated disposable runner pool with explicit trust gating.
