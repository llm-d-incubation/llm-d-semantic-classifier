# Pre-1.0 Phase and Version Roadmap

The project remains intentionally pre-1.0. Phase names describe maturity; versions remain `0.x`.

| Phase | Version | Focus | Promotion evidence |
|---|---:|---|---|
| Phase 1 — MVP | **0.1** | Rust service + runtime trait + Candle + real ModelCar + dummy Praxis | functional E2E + initial latency evidence |
| Phase 2 — Runtime hardening | **0.20** | scheduler/deadlines/load shedding/readiness/shutdown/metrics | deterministic failure/concurrency tests |
| Phase 2.1 — Performance characterization | **0.21** | CPU/GPU, cache, input length, concurrency, topology | repeatable homelab benchmark report |
| Phase 2.2 — Cache/session optimization | **0.22** | exact-result + optional feature cache + recovery/abstention | crash/cache-loss correctness |
| Phase 2.3 — Multi-signal runtime | **0.23** | multiple classifiers, partial failure, per-classifier lanes | domain/complexity/sensitivity contract suite |
| Phase 2.4 — Runtime pluggability | **0.24** | backend conformance, atomic model swaps, library-ready core | backend/lifecycle conformance |
| Phase 3 — OpenShift production-like | **0.30** | registry/disconnected/scaling/rollout/security | full system suite |
| Phase 3.1 — Praxis integration | **0.31** | replace dummy boundary with real Praxis integration | real gateway E2E |
| Phase 3.2 — Targeted inference optimization | **0.32** | measured bottlenecks only | equivalent accuracy + before/after p99 |
| Phase 4 — Feedback ecosystem | **0.40** | telemetry/artifact hooks for SDG/Training/Eval | external-loop contract, not training in service |

## 0.1 MVP

Prove the shape of the service:
- Rust server and protobuf contract;
- `ClassifierRuntime` abstraction;
- Candle first backend;
- model/tokenizer loaded once and resident;
- readiness after successful warmup;
- one real sensitivity embedding classifier fixture;
- external model delivered as OCI ModelCar;
- exact-result cache with versioned key;
- basic bounded inference queue;
- dummy Praxis integration;
- OpenShift same-pod sidecar and ClusterIP RTT measurements;
- queue/tokenize/forward/total timing separation;
- restart with complete context recomputes correctly.

Not required: distributed cache, custom kernels, vLLM backend, multiple signals, RL/training, production control plane, hard universal 20 ms SLA.

## 0.20 Runtime hardening

Make the service trustworthy before making it clever:
- bounded queue;
- per-job deadline;
- queued cancellation;
- load shedding;
- liveness/readiness distinction;
- graceful shutdown/drain;
- structured errors;
- metric-cardinality bounds;
- prompt redaction;
- deterministic concurrency configuration.

## 0.21 Performance characterization

Establish named hardware profiles and benchmark:
- 0/50/90/100% cache hit where useful;
- 32/64/128/256-token inputs;
- concurrency 1/2/4/8/16/32;
- CPU worker/math/tokenizer thread configurations;
- GPU if available;
- localhost sidecar;
- same-node ClusterIP;
- cross-node when possible.

Only after this phase should an absolute p99 threshold become a hard gate for a named hardware profile.

## 0.22 Cache/session optimization

Keep three concepts distinct:
1. resident model/tokenizer runtime state;
2. exact-result cache;
3. optional session/prefix/feature cache.

Routing/session authority remains Praxis. Complete cache loss must not silently turn `continue` into a confident downgrade; insufficient context yields abstention.

## 0.23 Multi-signal runtime

- registry of multiple classifiers;
- independent signal status;
- partial success;
- classifier-specific queue/concurrency limits;
- serial vs parallel measurement;
- failed sensitivity never becomes a benign low-sensitivity result.

## 0.24 Runtime pluggability

- backend conformance suite;
- Candle passes it;
- test/mock backend passes it;
- candidate load/warm -> atomic active-handle switch;
- old in-flight work drains on old immutable handle;
- `runtime-core` does not depend on network server so future library embedding is possible.

## 0.30 OpenShift production-like

- private OCI registry;
- digest pinning;
- no runtime Hugging Face download;
- egress-denied/disconnected start;
- random UID/read-only model data;
- NetworkPolicy;
- 1->N scaling;
- rolling service/model revision;
- pod/node disruption;
- resource pressure;
- metrics/provenance evidence.

## 0.32 Optimization

Only optimize measured bottlenecks: tokenizer/threading, copies/allocations, mask reuse, sequence buckets, dtype, CPU affinity/NUMA, true local/flash attention, custom kernels/ops, alternative runtimes. Every change needs accuracy parity and comparable latency evidence.
