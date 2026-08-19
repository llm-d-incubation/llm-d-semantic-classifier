# Known gaps

Current limitations of `llm-d-sc`, stated plainly. Each is tracked as an issue;
this page exists so the list has one authoritative home rather than being spread
across the README, specs, and commit messages.

The machine-checked view is `./hack/spec-check 0.1-mvp`, which reports every
acceptance criterion, every required test ID, and its execution status.

## Capability

| Gap | Impact | Phase |
| --- | --- | --- |
| Cost classification is anchors-only | `cost` has no fine-tuned model of its own and runs on a general-purpose embedder; measured accuracy trails the fine-tuned classifiers. A dedicated fine-tune is the tracked fix | 0.2 |
| Default artifact and serving backend differ | `hack/fetch-model` pulls the generic ModernBERT sequence classifier by default, while the serving path currently implements the embedding-and-ranking backend. Sequence-classification serving, and regenerated parity fixtures and benchmarks for it, land before that artifact is served end to end | 0.1 fix |
| No BUILT-IN custom domain classifiers | routing by business function (sales, shipping, finance, support) requires supplying your own definition. The mechanism works and is documented in [classifiers.md](classifiers.md), but no business-domain taxonomy ships by default | 0.4 |
| Single classifier per service instance | no registry, no per-classifier queues or lanes | 0.23 |
| No classifier revision swap | changing a classifier revision requires a restart; no canary or rollback | 0.24, 0.41 |

## Runtime

| Gap | Impact | Phase |
| --- | --- | --- |
| Executor width is not auto-tuned | the executor now runs a real worker pool (default 4, `LLM_D_SC_INFERENCE_WORKERS`), but the best width for a given host and quantisation is not discovered automatically | 0.2 |
| No per-request deadlines or cancellation | a queued request cannot be abandoned when the caller has already given up | 0.2 |
| No graceful drain | shutdown does not stop admission and drain in-flight work in a defined order | 0.2 |
| No health-checking endpoint | readiness is internal state; an orchestrator cannot probe it over gRPC or HTTP | 0.3 |

## Observability

| Gap | Impact | Phase |
| --- | --- | --- |
| Stage percentiles are bucketed approximations | per-stage p50/p95/p99 are now recorded in fixed log-scale histograms; a reported quantile is within 12.5% of the true sample and buckets report their lower bound | 0.2 |
| No metrics export | no Prometheus or OpenTelemetry endpoint yet | 0.3 |

## Validation

| Gap | Impact | Phase |
| --- | --- | --- |
| No cluster evidence | sidecar and Service round trips, same-node versus cross-node placement, disconnected artifact start, restricted security context, and restart behaviour are unmeasured | 0.3 |
| Benchmarks are single-environment | all published numbers come from one contributor's homelab and have not been independently reproduced. See [performance.md](performance.md) | ongoing |
| Behaviour under pod CPU limits unknown | published numbers are from an unconstrained host and will not transfer directly | 0.21 |

## Closed

Listed so the history is legible rather than quietly rewritten.

| Was | Closed by |
| --- | --- |
| Model forward ran on a Tokio network worker | bounded handoff to a dedicated executor thread pool (`I-090`, with `I-091` as the serialisation control) |
| Concurrency 4 raised miss latency roughly fourfold with no throughput gain | measured after the pool fix (`P-020`/`P-021`): 24 misses take 189.5 ms at width 1 and 44.3 ms at width 4, a 4.27x throughput gain, while forward p50 goes 7.68 ms to 7.17 ms. Added concurrency no longer costs latency |
| Production path bypassed the result cache and metrics | the real path runs through the shared `ServiceCore` |
| Classifier ranked against synthetic prototypes and reported synthetic revisions | artifact-backed classifier definitions with real anchors and revisions (`I-072`, `I-073`, `I-074`) |
| Persistent-channel claim rested on a counter that was never incremented | the claim is now measured server-side by counting accepted TCP connections (`I-002`, with `I-092` as the control) |
| Only cumulative latency sums were recorded | per-stage histograms with p50/p95/p99 (`U-086` through `U-089`) |

## Reporting a gap

If you hit something not listed here, please open an issue. Reports that include
the commit SHA, the classifier artifact revision, and how to reproduce are the
most useful. Security issues follow [SECURITY.md](../SECURITY.md) instead.

## Why sensitivity plateaus around 0.88-0.89

Worth stating explicitly, because the obvious next step does not work.

The first sensitivity model was trained on defective data (the generator's reasoning
traces rather than prompts). Retraining on clean, cross-verified synthetic data did NOT
improve overall accuracy: 0.8933 to 0.8800 on the same held-out set, which at n=75 is a
one-sample difference and therefore a wash. Boundary-case accuracy did improve, 0.7600 to
0.8000.

The confusion matrix says why. `CONFIDENTIAL` and `NEVER_EGRESS` are classified perfectly.
Essentially every error is `INTERNAL` predicted as `CONFIDENTIAL`, or `PUBLIC` as
`INTERNAL`. Those boundaries are ORGANISATIONAL POLICY, not properties of the text:
whether "summarise the postmortem for last Tuesday's outage" is internal or confidential
depends on a disclosure rule, and no volume of synthetic data can learn a policy the data
does not encode. Complexity reaches 0.975 on the same method because SIMPLE versus
REASONING is a property of the text itself.

The implication is that the sensitivity ceiling is a taxonomy-design problem rather than a
training problem, and the lever is anchors rather than epochs. An adopter who encodes their
own disclosure policy in `anchors.json` should be expected to beat the shipped defaults,
which necessarily encode an invented policy. See [classifiers.md](classifiers.md).
