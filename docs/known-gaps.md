# Known gaps

Current limitations of `llm-d-sc`, stated plainly. Each is tracked as an issue;
this page exists so the list has one authoritative home rather than being spread
across the README, specs, and commit messages.

The machine-checked view is `./hack/spec-check 0.1-mvp`, which reports every
acceptance criterion, every required test ID, and its execution status.

## Project

| Gap | Impact | Phase |
| --- | --- | --- |
| Default artifacts are hosted in a personal namespace | the built-in classifiers resolve to `cnuland/llm-d-sc-{complexity,cost,sensitivity}` on Hugging Face, so the default every user pulls depends on one individual's account and cannot be updated by other maintainers. Revisions are digest-pinned, so a moved artifact fails loudly at fetch rather than silently serving different weights. Migrating to the `llm-d` organisation is a one-line change per definition in `classifiers/` plus `hack/fetch-model` | tracked |

## Contract

| Gap | Impact | Phase |
| --- | --- | --- |
| Insufficient context is not inferred automatically | `ABSTAIN` is emitted when a gateway explicitly marks context as delta-only. The service does not yet infer insufficient context from text, and it has no optional session feature cache to recover it | 0.22 |
| Scores are similarities, not calibrated probabilities | ranked scores are cosine similarities against labelled anchors. They are comparable within one response, which makes the margin between the top two meaningful, but not across models or taxonomies, and should not be read as statistical confidence | 0.21 |
| Result cache eviction is FIFO | the cache is bounded (50k entries) so memory is finite, but eviction is insertion-order rather than least-recently-used. FIFO was chosen because it bounds memory at one push and one pop per insert, while LRU needs recency bookkeeping on every hit and a hit is 632 nanoseconds | 0.22 |

## Capability

| Gap | Impact | Phase |
| --- | --- | --- |
| Cost accuracy trails the other signals | `cost` is now fine-tuned (0.833 held-out, up from 0.750 on a general-purpose embedder) but remains the weakest of the three. Cross-model label verification discarded 34% of its synthetic corpus for disagreement, against far less for complexity, which says the tiers are genuinely more ambiguous rather than merely under-trained | 0.2 |
| No generic domain classification | the ModernBERT domain classifier is a SEQUENCE-classification model and the 0.1 backend ranks embeddings against anchors, so it cannot serve one. It is deliberately not offered by `hack/fetch-model`: fetching an artifact the runtime cannot rank against produces confident nonsense rather than an error. Needs a sequence-classification runtime adapter | 0.25 |
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
| No metrics export | no Prometheus or OpenTelemetry endpoint yet. Per-stage percentiles are logged to stderr on an interval as an interim measure | 0.3 |

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

The retrained model was therefore NOT published. Shipping a second artifact whose advantage
is not established adds confusion rather than value, and the currently pinned model has the
marginally better overall number and is the one the golden-fixture tests (I-021 through
I-024) were validated against. The retrained weights and the pipeline that produced them are
reproducible from `training/`.

The implication is that the sensitivity ceiling is a taxonomy-design problem rather than a
training problem, and the lever is anchors rather than epochs. An adopter who encodes their
own disclosure policy in `anchors.json` should be expected to beat the shipped defaults,
which necessarily encode an invented policy. See [classifiers.md](classifiers.md).
