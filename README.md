# llm-d-semantic-classifier

**Semantic classification for inference routing.** A low-latency Rust service
that turns an incoming request into calibrated semantic signals so an AI Gateway
can route it well. Release `0.1` classifies request **domain**; additional signal
types are planned (see [Project status](#project-status)).

[![fast-ci](https://github.com/llm-d-incubation/llm-d-semantic-classifier/actions/workflows/fast-ci.yml/badge.svg)](https://github.com/llm-d-incubation/llm-d-semantic-classifier/actions/workflows/fast-ci.yml)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![llm-d incubation](https://img.shields.io/badge/llm--d-incubation-5B2C8D.svg)](https://github.com/llm-d-incubation)
[![Slack](https://img.shields.io/badge/Slack-%23sig--semantic--classifier-4A154B.svg?logo=slack)](https://llm-d.ai/slack)

> **Incubating, pre-1.0.** llm-d-semantic-classifier (`llm-d-sc`) lives in the
> [llm-d-incubation](https://github.com/llm-d-incubation) organization, where new
> llm-d components are developed before graduation. APIs, configuration, and the
> wire contract may change between `0.x` releases. See
> [Project status](#project-status) for what is proven today and what is tracked
> as open work.

---

## Contents

- [What it is](#what-it-is)
- [Why it exists](#why-it-exists)
- [Quick start](#quick-start)
- [Architecture](#architecture)
- [Repository layout](#repository-layout)
- [Project status](#project-status)
- [Community](#community)
- [Contributing](#contributing)
- [License](#license)

## What it is

`llm-d-sc` executes semantic classifiers in the inference request path and
returns ranked, versioned evidence about a request. It is a **signal producer,
not a decision maker**.

![AI Gateway and llm-d-sc: the gateway performs routing and enforcement while llm-d-sc returns semantic classification signals. An incoming request enters the AI Gateway, which sends a classification request to llm-d-sc; llm-d-sc returns ranked semantic signals with confidence scores (the diagram shows the target set of domain, sensitivity, and complexity; release 0.1 serves domain), and the gateway then applies policy, session stickiness, guardrails, and fallback to select the final model.](docs/images/llm-d-sc-semantic-routing.png)

> The payload above shows the target shape once several signal types are served
> together. Release `0.1` serves a **generic domain classifier** and returns
> ranked labels with the classifier, model, tokenizer, and taxonomy revisions
> that produced them. Additional signal types, and several signals in one
> response, arrive in later phases.

![llm-d-sc: what it IS and what it isn't. llm-d-sc IS a semantic classifier runtime service that executes classifiers to produce signals about a request; built for speed with a long-lived Rust service, resident models, caching and warmup; a pluggable runtime architecture with Candle as the first backend and others such as ONNX or vLLM possible later; a signal producer rather than a decision maker, returning ranked signals with confidence while routing decisions are made by the AI Gateway; safe by design, able to abstain when context is insufficient rather than guess; and observable, exposing metrics for latency, confidence, abstention and cache behaviour. llm-d-sc is NOT a router (it does not choose models or endpoints or apply policy; that is the AI Gateway), not a policy or guardrail engine, not a session or state authority (routing state, stickiness and fallback belong to the AI Gateway), not a general-purpose LLM platform, not a training platform, and not a model lifecycle or management system.](docs/images/llm-d-sc-is-and-isnt.png)

Routing, policy, session authority, and organisation-wide model lifecycle belong
to the gateway and its control plane. The wire contract enforces the boundary:
the response type has **no route or endpoint field**, by design
([ADR-0001](docs/decisions/0001-no-route-field-in-response.md)).

## Why it exists

Routing an inference request well requires knowing something about it. Performing
that classification inside a gateway couples model residency, tokenizer
versioning, and CPU/GPU scheduling to the data plane.

| Concern | Without a classifier runtime | With `llm-d-sc` |
| --- | --- | --- |
| Gateway complexity | model loading and warmup in the routing hot path | gateway stays a gateway |
| Classification cost | per-request model work | resident models, versioned result cache |
| Overload behaviour | unbounded queueing inside the data plane | bounded admission, explicit rejection |
| Signal trust | labels with no provenance | every result carries classifier, model, tokenizer, and taxonomy revisions |
| Backend choice | baked in | `ClassifierRuntime` abstraction; Candle is the first backend, not the architecture |

## Quick start

Requires a Rust toolchain and `protoc`.

**1. Materialise a classifier artifact** (ModelCar layout, mounted at `/models`
in a container). The default is a generic multilingual ModernBERT domain
classifier:

```bash
./hack/fetch-model            # default: generic domain classifier
./hack/fetch-model --list     # other classifiers, including custom ones
```

**2. Run the service:**

```bash
LLM_D_SC_MODEL_DIR=./artifacts/models/intent \
LLM_D_SC_LISTEN=0.0.0.0:50051 \
  cargo run --release --bin llm-d-sc-server
```

```text
llm-d-sc: bound 0.0.0.0:50051; ModelCar dir ./artifacts/models/intent;
READY (resident classifier loaded and warmed)
```

The service reports **not ready** until the artifact is validated, the model and
tokenizer are loaded, and a warmup forward has succeeded, an orchestrator never
routes traffic to a cold instance.

**3. Classify a request.** The bundled gateway stand-in issues a real gRPC call
over a persistent channel, consumes the signals, and applies its own test-only
policy afterwards, demonstrating that routing authority stays outside this
service:

```bash
cargo test --release --test grpc -- --nocapture
```

A response carries ranked signals and the revisions that produced them:

```text
request_id:         "req-1"
classifier_id:      "domain"
model_revision:     "bf8d3833707d1bb8f9237260c271ca0d5982462d"
tokenizer_revision: "bf8d3833707d1bb8f9237260c271ca0d5982462d"
taxonomy_revision:  "mmbert-intent-14"
status:             OK
ranked:             [ { label: "computer science", score: 0.71 },
                      { label: "engineering",      score: 0.12 },
                      { label: "math",             score: 0.05 } ]
```

Verification and benchmarking:

```bash
./hack/verify              # fmt, clippy -D warnings, build, unit + local tests
./hack/test-parity         # model-dependent tests against the pinned artifact
./hack/spec-check 0.1-mvp   # evidence ledger: every criterion and test ID
cargo run --release --bin bench-runner   # latency matrix
```

Deployment manifests live in [`deploy/`](deploy/). The container build
([`Containerfile`](Containerfile)) ships **no model**. The classifier arrives as
a separate OCI artifact.

## Architecture

```text
  AI Gateway
      │  gRPC (persistent HTTP/2)
      ▼
 ┌─────────────────────────────────────────────────────────┐
 │ tonic handler                     (Tokio: I/O only)     │
 └───────────────┬─────────────────────────────────────────┘
                 ▼
        exact-result cache ──── hit ──────────────► ranked signals
                 │ miss
                 ▼
        bounded admission ───── over capacity ────► RESOURCE_EXHAUSTED
                 │
                 ▼
     dedicated inference executor       (never a network worker)
                 │
                 ▼
        ClassifierRuntime  ──►  Candle backend
                 │
                 ▼
   tokenize → transformer → pooling → normalize → rank
```

Three properties are load-bearing:

1. **The networking runtime is not the model scheduler.** Model forwards execute
   on dedicated inference workers; the request handler only admits work and
   awaits a result.
2. **Overload is explicit.** Beyond the configured admission bound, requests are
   rejected with `RESOURCE_EXHAUSTED` rather than queued without limit.
3. **Cache identity is versioned.** Keys are BLAKE3 fingerprints over the
   classifier, model, tokenizer, and taxonomy revisions plus a hash of the
   normalized input. It is never the raw prompt, and never reusable across a
   revision change.

Further reading: [`docs/architecture.md`](docs/architecture.md) ·
[`docs/research/runtime-performance.md`](docs/research/runtime-performance.md) ·
[`docs/decisions/`](docs/decisions/)

## Repository layout

```text
src/                 library crate: config, runtime, cache, handoff, gRPC, classify
src/bin/             llm-d-sc-server, bench-runner
proto/               the gRPC wire contract
tests/               integration, parity, and benchmark-harness suites
deploy/              Kubernetes manifests, ModelCar build
hack/                verify, test-report, test-parity, spec-check, fetch-model
docs/                architecture, decisions (ADRs), condensed research,
                     benchmark results and methodology
specs/               the design record, see below
```

`specs/` is the project's **design record**, not scaffolding. Each version owns a
directory containing its problem statement, observable behaviour, non-goals,
acceptance criteria mapped to stable test IDs, failure contract, rollback path,
and per-criterion RED/GREEN evidence. It exists so that what was intended, what
was proven, and what was deliberately deferred are all auditable rather than
folkloric. See
[`docs/research/development-method.md`](docs/research/development-method.md).

## Project status

Pre-1.0 and phased; see [`docs/VERSIONS.md`](docs/VERSIONS.md).

| Version | Focus | State |
| --- | --- | --- |
| **0.1** | service shape: gRPC contract, runtime abstraction, ModernBERT backend, artifact delivery, result cache, bounded admission, gateway integration, generic domain classification | local evidence complete; cluster evidence open |
| 0.2 | runtime hardening: deadlines, cancellation, load shedding, graceful drain, structured errors | open |
| 0.21 | performance characterisation, named hardware profiles, SLO proposal | open |
| 0.22 | cache and session optimisation, abstention on context loss | open |
| 0.23 | multi-signal runtime: complexity, sensitivity, and cost signals returned together with independent status | open |
| 0.24 | runtime pluggability: backend conformance suite, atomic classifier revision swap | open |
| 0.3 | production-like Kubernetes validation: topology, disconnected artifact start, restricted security context, scaling, rollout | open |
| 0.31 | integration with a real AI Gateway, replacing the in-repo stand-in | open |
| 0.32 | targeted inference optimisation, driven only by measured bottlenecks | open |
| 0.4 | custom domain classifiers: bring your own labels for business routing such as sales, shipping, finance, or support | open |
| 0.41 | classifier lifecycle: revision promotion, canary comparison, rollback | open |
| 0.5 | feedback loop: export classification outcomes for offline evaluation and retraining, without training inside the serving runtime | open |

Detailed phase definitions live in [`docs/VERSIONS.md`](docs/VERSIONS.md); open
work is tracked as issues.

The evidence ledger is machine-checked, not asserted:

```bash
./hack/spec-check 0.1-mvp             # per-criterion status and pending test IDs
./hack/spec-check 0.1-mvp --promotion # refuses while any required gate is unmet
```

This project treats *"many passing unit tests"* and *"the system is proven"* as
different claims, and the tooling enforces the difference. Current limitations
are recorded in [`docs/known-gaps.md`](docs/known-gaps.md) and tracked as issues.

## Community

Everyone is welcome: contributors, operators, researchers, and the merely
curious. llm-d-sc is developed in the open as part of the
[llm-d community](https://llm-d.ai/community).

| Channel | Where |
| --- | --- |
| Slack | [`#sig-semantic-classifier`](https://llm-d.ai/slack) in the llm-d workspace, for design discussion, questions, and help getting started ([get an invite](https://llm-d.ai/slack)) |
| Issues | [this repository](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues); newcomer-friendly work is labelled `good-first-issue` |
| Weekly standup | Wednesdays, 12:30pm ET, open to the public ([community calendar](https://calendar.google.com/calendar/u/0?cid=NzA4ZWNlZDY0NDBjYjBkYzA3NjdlZTNhZTk2NWQ2ZTc1Y2U5NTZlMzA5MzhmYTAyZmQ3ZmU1MDJjMDBhNTRiNEBncm91cC5jYWxlbmRhci5nb29nbGUuY29t)) |
| Mailing list | [llm-d-contributors](https://groups.google.com/g/llm-d-contributors) |
| Code | [llm-d](https://github.com/llm-d) and [llm-d-incubation](https://github.com/llm-d-incubation) |
| Conduct | [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) |

All llm-d meetings are open. Join to participate, ask questions, or just listen.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). In summary:

- commits require a [DCO](https://developercertificate.org/) `Signed-off-by` line
  (`git commit -s`)
- `./hack/verify` must be green
- behaviour claims need tests; performance claims need comparable p50/p95/p99
  evidence with the measurement conditions recorded
- existing test assertions are protected. Weakening one requires an explicit
  argument that the previous contract was wrong
- substantial changes are specification-first (`specs/`)

Maintainers are listed in [MAINTAINERS.md](MAINTAINERS.md). Security reports go
through [SECURITY.md](SECURITY.md), not public issues.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
