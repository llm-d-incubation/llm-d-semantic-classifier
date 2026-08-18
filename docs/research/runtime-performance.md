# Runtime performance: decisions and measured results

Condensed design rationale for llm-d-sc's hot path. This is the subset of the
project's research that became implemented decisions, with **measured** numbers
substituted for the original predictions. Full research notes live in the
development repository; upstream readers should not need them.

## The one rule that shaped the architecture

> The async networking runtime must not become the model execution scheduler.

A classifier in the inference request path has two workloads with opposite
characteristics: many small network events, and a small number of CPU-saturating
model forwards. Running the second on the runtime that serves the first starves
sockets, inflates unrelated requests, and makes tail latency unpredictable.

llm-d-sc therefore separates them explicitly:

```
gateway --gRPC--> tonic handler ──> result cache ──hit──> response
                                        │
                                       miss
                                        ▼
                               bounded admission
                                        │
                                        ▼
                          dedicated inference executor
                                        │
                                        ▼
                            resident classifier runtime
```

The handler only admits work and awaits a response; the forward happens on a
dedicated thread. Admission beyond the configured bound is **rejected
explicitly** rather than queued, because queueing past capacity converts an
overload into a latency incident for every caller.

## Optimisation order (do less work before doing work faster)

1. **Don't infer.** An exact-result cache keyed by a versioned fingerprint.
2. **Keep the hot path resident.** Model and tokenizer load once per active
   revision; a warmup forward runs before the service reports ready.
3. **Bound concurrency.** Networking concurrency and inference concurrency are
   different budgets and must be configured separately.
4. **Avoid copies.** Typed results are cached and cloned, never re-serialised
   through strings.
5. Only then consider kernels, dtypes, or batching — and only against a
   measured bottleneck.

## Cache identity is a correctness property

The cache key is a 32-byte BLAKE3 fingerprint over the classifier id, the model
revision, the tokenizer revision, the taxonomy revision, and a hash of the
normalized input — never the raw prompt as sole identity.

Two consequences, both deliberate:

- A revision change **cannot** serve a stale classification; the key changes.
- The raw prompt is not retained in cache identity or telemetry, only hashes.

A 64-bit hash was rejected: a collision serves a *wrong classification*, and
`std::collections::hash_map::DefaultHasher` is explicitly not stable across
compiler versions, which would silently invalidate a persisted key space.

Identical concurrent misses are coalesced (single-flight), so a cold-start
stampede on one key produces one forward, not N.

## Measured behaviour

Host: 16-core Apple Silicon, `--release` (opt-level 3, fat LTO, one codegen
unit). Model: a pinned BERT-based SentenceTransformers embedding classifier
(~22.7M parameters), Candle CPU backend. Full methodology and manifest:
`docs/performance.md`.

**Uncached inference by input length** (single request, no cache, no network):

| tokens | p50 | p95 | p99 | req/s |
|---:|---:|---:|---:|---:|
| 32 | 11.9ms | 12.6ms | 13.0ms | 85 |
| 64 | 15.2ms | 16.2ms | 16.7ms | 65 |
| 128 | 23.4ms | 25.2ms | 26.0ms | 42 |
| 256 | 51.9ms | 54.7ms | 55.7ms | 19 |

p99 sits within ~10% of p50 at every length: the model itself is not the source
of tail risk.

**Concurrency** (64-token inputs, shared resident classifier):

| concurrency | p50 | p99 | aggregate req/s |
|---:|---:|---:|---:|
| 1 | 15.5ms | 18.2ms | 64 |
| 2 | 15.6ms | 17.3ms | 128 |
| 4 | 17.5ms | 20.0ms | 227 |
| 8 | 26.3ms | 31.2ms | 302 |

Linear to 2, still favourable at 4, past the knee at 8 (+33% throughput for
+50% latency). This is why admission is bounded rather than unbounded: beyond
the knee, additional in-flight work degrades every caller's latency.

**Cache hit cost** (versioned key + lookup + typed clone):

| input size | p50 | hits/s |
|---:|---:|---:|
| 144 B | 1.08us | ~1.0M |
| 2 KB | 2.00us | ~459K |
| 8 KB | 5.54us | ~169K |

A hit is roughly four orders of magnitude cheaper than a miss, so hit rate — not
raw forward speed — governs mean latency:

| hit rate | mean latency (64-token miss path) |
|---:|---:|
| 50% | ~7.8ms |
| 90% | ~1.6ms |
| 99% | ~0.16ms |

## Practical guidance

- Truncate classification input aggressively. Routing decisions do not need
  256 tokens, and cost grows faster than linearly.
- Cap inference concurrency near the measured knee (4 on this profile) and shed
  beyond it.
- Treat hit rate as a first-class operational metric; it dominates the SLO.
- Re-measure per hardware profile before adopting any absolute latency target.
  These are developer-hardware, loopback numbers; cluster CPU and pod CPU limits
  change them materially.

## Deliberately not done

Custom kernels, alternative runtime backends, micro-batching, and allocator
replacement are all unjustified until a measured bottleneck demands them. The
runtime backend is an abstraction (`ClassifierRuntime`) precisely so those
choices stay replaceable without touching the service.
