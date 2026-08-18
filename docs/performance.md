# Performance

All numbers on this page were measured on a **single contributor's homelab**
(@cnuland) and have **not been independently reproduced**. Treat them as a
directional baseline and a methodology reference, not as project performance
claims and not as a service level objective.

> **Call for external validation.** If you run these benchmarks on other
> hardware, please open an issue or a pull request adding your results. Numbers
> from a second environment are worth more to this project than better numbers
> from the same one. See [Reproducing](#reproducing) below.

## Environment

| Field | Value |
| --- | --- |
| Operator | @cnuland homelab (single-operator, unaudited) |
| Host | Apple M4 Max, 16 cores |
| Build | `--release` (opt-level 3, fat LTO, one codegen unit) |
| Backend | Candle 0.11, **CPU** (no GPU) |
| Transport | loopback (in-host), **not** a cluster network path |
| Model | pinned BERT-based SentenceTransformers embedding classifier, ~22.7M parameters |
| Classifier definition | fixture prototypes; the real taxonomy is unverified |
| Independent reproduction | **none** |

Full machine-readable results, including per-scenario manifests:
[`20260818-full-matrix.json`](benchmarks/20260818-full-matrix.json) and
[`20260818-bootstrap-sha-462c270.json`](benchmarks/20260818-bootstrap-sha-462c270.json).

**Reproducibility.** The matrix was run twice, before and after a substantial
refactor that moved caching and metrics onto the real classifier path and
renamed a large part of the tree. Results agreed within run-to-run noise: cache
hits identical at 0.09 ms p50, misses within 0.9 ms at every input length
(12.66 vs 11.81 at 32 tokens, 16.08 vs 15.10 at 64, 22.72 vs 22.79 at 128,
49.69 vs 49.65 at 256). Tables below cite the second run, taken at the commit
intended for the first upstream push.

## Cache hits, end to end over gRPC

300 measured requests per scenario after 300 warmup requests.

| Input | Concurrency | p50 | p90 | p95 | p99 | Throughput |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 tok | 1 | 0.09 ms | 0.10 ms | 0.10 ms | 0.11 ms | 11,204 req/s |
| 32 tok | 4 | 0.10 ms | 0.12 ms | 0.13 ms | 0.21 ms | 32,706 req/s |
| 64 tok | 1 | 0.09 ms | 0.10 ms | 0.10 ms | 0.11 ms | 11,146 req/s |
| 64 tok | 4 | 0.12 ms | 0.13 ms | 0.14 ms | 0.29 ms | 29,704 req/s |
| 128 tok | 4 | 0.11 ms | 0.13 ms | 0.14 ms | 0.26 ms | 31,532 req/s |
| 256 tok | 4 | 0.11 ms | 0.13 ms | 0.15 ms | 0.32 ms | 30,744 req/s |

Hit cost is flat across input length, as expected: a hit performs zero
tokenisation and zero model forwards. The harness asserts this rather than
assuming it, by checking the service's cache counters around the measured window.

## Cache misses, end to end over gRPC

300 measured requests per scenario after 100 warmup requests. Measurement keys
are drawn from a namespace disjoint from the warmup keys, so a "miss" scenario
cannot accidentally measure pre-warmed hits.

| Input | Concurrency | p50 | p90 | p95 | p99 | max | Throughput |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 tok | 1 | 11.81 ms | 12.72 ms | 13.55 ms | 14.37 ms | 27.62 ms | 83.4 req/s |
| 32 tok | 4 | 46.65 ms | 48.23 ms | 48.60 ms | 49.46 ms | 50.09 ms | 85.5 req/s |
| 64 tok | 1 | 15.10 ms | 15.96 ms | 16.52 ms | 17.78 ms | 18.50 ms | 65.7 req/s |
| 64 tok | 4 | 60.76 ms | 63.53 ms | 65.06 ms | 71.03 ms | 71.64 ms | 65.4 req/s |
| 128 tok | 1 | 22.79 ms | 24.42 ms | 24.94 ms | 27.54 ms | 30.10 ms | 43.4 req/s |
| 128 tok | 4 | 90.20 ms | 93.26 ms | 94.41 ms | 100.83 ms | 101.86 ms | 44.1 req/s |
| 256 tok | 1 | 49.65 ms | 51.72 ms | 54.12 ms | 56.30 ms | 59.57 ms | 20.0 req/s |
| 256 tok | 4 | 199.73 ms | 206.86 ms | 216.20 ms | 228.16 ms | 239.72 ms | 19.9 req/s |

**Known limitation visible in this data.** At concurrency 4, miss latency rises
roughly four-fold while throughput does not improve. The inference executor
currently runs a single worker, so concurrent misses serialise behind it. This is
tracked as an open issue; the admission bound is working as designed, the
executor width is not yet configurable.

## Inference floor, classifier called directly

Single request, no cache, no network: the cost of one real forward, pooling,
normalisation, and ranking. 200 measured requests per row after 20 warmups.

| Input | p50 | p90 | p95 | p99 | max | req/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 tok | 11.86 ms | 12.40 ms | 12.58 ms | 13.00 ms | 13.06 ms | 85 |
| 64 tok | 15.22 ms | 15.93 ms | 16.21 ms | 16.70 ms | 22.48 ms | 65 |
| 128 tok | 23.35 ms | 24.59 ms | 25.15 ms | 26.00 ms | 32.14 ms | 42 |
| 256 tok | 51.85 ms | 53.76 ms | 54.72 ms | 55.69 ms | 59.97 ms | 19 |

p99 sits within roughly 10% of p50 at every input length: on this host, the model
itself is not a source of tail latency. Cost grows faster than linearly with
input length (8x the tokens for about 4.4x the time).

## Parallelism of the classifier itself

The classifier called directly from N threads, sharing one resident model,
64-token inputs. This measures what the hardware can do, independent of the
service's admission design.

| Concurrency | p50 | p95 | p99 | Aggregate throughput |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 15.5 ms | 16.7 ms | 18.2 ms | 64 req/s |
| 2 | 15.6 ms | 16.6 ms | 17.3 ms | 128 req/s |
| 4 | 17.5 ms | 18.6 ms | 20.0 ms | 227 req/s |
| 8 | 26.3 ms | 29.8 ms | 31.2 ms | 302 req/s |

Scaling is linear to concurrency 2, still favourable at 4, and past the knee at 8
(+33% throughput for +50% latency). On this host, 4 is the point where admission
should shed rather than admit.

## Cache layer cost in isolation

BLAKE3 versioned key construction, lookup, and typed result clone. 5,000
measured operations per row.

| Input size | p50 | p95 | p99 | Operations/s |
| ---: | ---: | ---: | ---: | ---: |
| 144 B | 1.08 us | 1.12 us | 1.21 us | 1,047,870 |
| 540 B | 0.71 us | 0.92 us | 0.92 us | 1,310,673 |
| 2,052 B | 2.00 us | 2.54 us | 2.54 us | 458,523 |
| 8,208 B | 5.54 us | 7.00 us | 7.04 us | 168,689 |

Cost scales with input size, consistent with hashing throughput of roughly
1.5 GB/s on this host.

## What the numbers imply

A hit is about four orders of magnitude cheaper than a miss, so on this host the
hit rate dominates mean latency far more than forward speed does:

| Hit rate | Mean latency, 64-token miss path |
| ---: | ---: |
| 50% | ~7.8 ms |
| 90% | ~1.6 ms |
| 99% | ~0.16 ms |

Directional conclusions, valid only for this environment:

- Truncate classification inputs. Cost grows faster than linearly, and routing
  decisions rarely need long context.
- Cap inference concurrency near the measured knee and shed beyond it.
- Track hit rate as a first-class operational metric.

## Reproducing

```bash
./hack/fetch-model                        # pinned classifier artifact
cargo build --release --bin bench-runner
BENCH_WARMUP=100 BENCH_MEASURE=300 \
LLM_D_SC_MODEL_DIR=./artifacts/models/sensitivity \
  ./target/release/bench-runner            # writes docs/benchmarks/<timestamp>.json
```

The runner refuses to run against the synthetic test pipeline: if the model
artifact is missing it exits with an error rather than producing numbers that
look real. Every run records its own manifest (commit SHA, model and tokenizer
revisions, backend, host CPU, topology, concurrency, cache mode, input length,
warmup and measurement counts) alongside the results.

## Not yet measured

- Cluster topology: sidecar and Service (ClusterIP) round trips, same-node
  versus cross-node placement.
- Behaviour under pod CPU limits, which will differ substantially from an
  unconstrained host.
- GPU backends.
- Per-stage latency distributions. Stage timings are currently accumulated
  totals rather than histograms, so queue, tokenise, and forward percentiles are
  not yet available.
- Saturation beyond concurrency 4, deadline expiry, and cancellation behaviour
  (phase 0.20 and 0.21).
