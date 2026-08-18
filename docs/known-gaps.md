# Known gaps

Current limitations of `llm-d-sc`, stated plainly. Each is tracked as an issue;
this page exists so the list has one authoritative home rather than being spread
across the README, specs, and commit messages.

The machine-checked view is `./hack/spec-check 0.1-mvp`, which reports every
acceptance criterion, every required test ID, and its execution status.

## Capability

| Gap | Impact | Phase |
| --- | --- | --- |
| Only one signal type (domain) is served | complexity, sensitivity, and cost signals are not yet available | 0.23 |
| Default artifact and serving backend differ | `hack/fetch-model` pulls the generic ModernBERT sequence classifier by default, while the serving path currently implements the embedding-and-ranking backend. Sequence-classification serving, and regenerated parity fixtures and benchmarks for it, land before that artifact is served end to end | 0.1 fix |
| No custom domain classifiers | routing by business function (sales, shipping, finance, support) requires bringing your own labels, which has no documented path yet | 0.4 |
| Single classifier per service instance | no registry, no per-classifier queues or lanes | 0.23 |
| No classifier revision swap | changing a classifier revision requires a restart; no canary or rollback | 0.24, 0.41 |

## Runtime

| Gap | Impact | Phase |
| --- | --- | --- |
| Inference executor runs a single worker | concurrent cache misses serialise: on the measured host, concurrency 4 raised miss latency roughly fourfold with no throughput gain. Admission bounding works as designed; executor width is not yet configurable | 0.1 fix |
| No per-request deadlines or cancellation | a queued request cannot be abandoned when the caller has already given up | 0.2 |
| No graceful drain | shutdown does not stop admission and drain in-flight work in a defined order | 0.2 |
| No health-checking endpoint | readiness is internal state; an orchestrator cannot probe it over gRPC or HTTP | 0.3 |

## Observability

| Gap | Impact | Phase |
| --- | --- | --- |
| Stage timings are accumulated totals, not histograms | queue, tokenise, and forward percentiles are unavailable; only cumulative sums are recorded | 0.2 |
| No metrics export | no Prometheus or OpenTelemetry endpoint yet | 0.3 |

## Validation

| Gap | Impact | Phase |
| --- | --- | --- |
| No cluster evidence | sidecar and Service round trips, same-node versus cross-node placement, disconnected artifact start, restricted security context, and restart behaviour are unmeasured | 0.3 |
| Benchmarks are single-environment | all published numbers come from one contributor's homelab and have not been independently reproduced. See [performance.md](performance.md) | ongoing |
| Behaviour under pod CPU limits unknown | published numbers are from an unconstrained host and will not transfer directly | 0.21 |

## Reporting a gap

If you hit something not listed here, please open an issue. Reports that include
the commit SHA, the classifier artifact revision, and how to reproduce are the
most useful. Security issues follow [SECURITY.md](../SECURITY.md) instead.
