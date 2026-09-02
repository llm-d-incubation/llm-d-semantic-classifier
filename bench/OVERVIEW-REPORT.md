# llm-d-sc v0.2-staging — performance characterisation

**Cluster:** CoreWeave *waldorf* · **Namespace:** `cnuland-dev` · **Date:** 2026-09-02
**Branch under test:** `v0.2-staging` (semantic cache #22 + context-completeness #23)

Every figure below is generated from captured JSON in `results/json/`, with raw
per-request samples retained. Nothing is transcribed by hand. The companion
`STATISTICAL-REPORT.md` carries the complete distributions; this document
explains what they mean.

---

## Reading this as a network engineer

llm-d-sc is a **request-scoped classification service on the data path**. Treat
it as a middlebox: every client request traverses it before reaching a backend,
so its service time is added to every flow and its admission limit is a hard
ceiling on offered load.

Two terms used throughout:

* **Offered concurrency** — simultaneously outstanding requests (session depth),
  not connections. The driver is closed-loop: it holds N requests in flight and
  replaces each one as it completes.
* **Cache-hit vs cache-miss path** — these are two completely different cost
  regimes in the same service, differing by ~3 orders of magnitude. Any capacity
  statement that does not say which path it describes is meaningless.

| Path | What runs | Typical p50 | Single-replica ceiling |
|---|---|---:|---:|
| **Cache hit** | blake3 key → HashMap lookup | **0.24 ms** | ~96,800 req/s |
| **Cache miss** | tokenize → ModernBERT forward → rank | **~480 ms** | ~67 req/s |

That ratio — roughly **1,400× throughput** between hit and miss — is the single
most important fact about this service.

---

## 1. Where is the bottleneck?

**There are three distinct ceilings, and they bind in different order depending
on the workload.**

### 1a. Admission bound: 256 in-flight per replica (hard)

`DEFAULT_QUEUE_BOUND = 256` (`src/grpc/classify.rs:36`) caps total admitted
(in-flight + queued) work per replica, enforced by a semaphore plus a bounded
channel. Exceeding it returns gRPC `RESOURCE_EXHAUSTED` — explicit shedding, not
silent queueing.

Measured, single replica, cache-hit:

| Offered concurrency | OK | RESOURCE_EXHAUSTED |
|---:|---:|---:|
| 256 | 2,265,664 | **0** |
| 512 | 2,200,433 | 14,496 (0.65 %) |
| 1024 | 2,217,397 | 13,655 (0.61 %) |

The zero-error boundary is **(256, 512]** — exactly the configured bound. This
independently reproduces, from inside the binary, the `(250, 500]` boundary
Intel's Arena campaign found as a black box.

**This bound is not configurable at runtime.** The only env knobs are
`LLM_D_SC_CLASSIFIER`, `LLM_D_SC_INFERENCE_WORKERS`, `LLM_D_SC_LISTEN`,
`LLM_D_SC_METRICS_LOG_SECS`, `LLM_D_SC_MODEL_DIR`, `LLM_D_SC_TRACE_CAPACITY`,
`LLM_D_SC_CACHE`, `LLM_D_SC_REDIS_URL`. Operators cannot raise admission without
a rebuild.

### 1b. Throughput knee: offered concurrency 64 (cache-hit)

| Concurrency | req/s | Δ vs previous | p50 | p99 |
|---:|---:|---:|---:|---:|
| 16 | 69,845 | +92 % | 0.183 ms | 0.356 ms |
| 32 | 86,536 | +24 % | 0.253 ms | 0.620 ms |
| **64** | **96,804** | +12 % | 0.474 ms | 1.085 ms |
| 128 | 94,117 | **−3 %** | 1.007 ms | 19.121 ms |
| 256 | 90,620 | −4 % | 1.977 ms | 23.806 ms |

Throughput scales ~90 % per doubling to concurrency 16, bends at 32, **peaks at
64, then goes retrograde**. Past the peak you pay 2–9× latency for *less*
throughput. p99 also inflects hard between 64 and 128 (1.09 ms → 19.12 ms).

**Operating point: offered concurrency 32–64 per replica.**

### 1c. Compute bound: the model forward (cache-miss)

On the miss path the ModernBERT forward dominates absolutely. Service-reported
stage timings at 256 B: `tokenize p50=80µs`, `forward p50=122.88ms`,
cache-hit `total p50=15µs`. The forward is **~1,500× the tokenizer** and
**~8,000× a cache hit**.

**Verdict:** on the hit path the bottleneck is contention/admission in the
service path, not CPU. On the miss path it is unambiguously the model forward.

---

## 2. How many connections per minute?

Single replica, cache-hit, 4 workers, 256 B context:

> **96,804 req/s = 5,808,249 requests per minute**, zero errors.

Scaled out, cache-hit, 8 workers/replica:

| Replicas | req/s | **req/min** | Scaling vs 1 |
|---:|---:|---:|---:|
| 1 | 168,414 | 10,104,816 | 1.00× |
| 2 | 320,606 | 19,236,334 | 1.90× |
| 4 | 424,125 | 25,447,478 | 2.52× |
| 8 | 602,988 | **36,179,267** | 3.58× |

On the **cache-miss** path the honest numbers are three orders of magnitude lower:

| Replicas | req/s | req/min |
|---:|---:|---:|
| 1 | 66 | 3,970 |
| 8 | 380 | 22,817 |

**Quote the number with its path.** "36 million requests/minute" is true for
cached classifications on 8 replicas; "23 thousand requests/minute" is true for
novel prompts on the same hardware.

---

## 3. Ideal scaling

### Vertical — executor workers (`LLM_D_SC_INFERENCE_WORKERS`)

The two paths want *opposite* things.

| Workers | hit req/s | miss req/s |
|---:|---:|---:|
| 1 | 39,759 | 9 |
| 2 | 51,926 | 18 |
| 4 | 84,962 | 35 |
| **8** | **148,875** | 67 |
| 16 | 107,795 | 115 |
| 32 | 87,896 | **181** |

* **Cache-hit peaks at 8 workers**, then *falls* 41 % by 32 workers — added
  threads buy contention, not throughput.
* **Cache-miss scales monotonically** to 32 workers (near-linear to 8: 2.0×,
  1.94×, 1.91× per doubling; then 1.72×, 1.57×). It is compute-bound, so more
  workers genuinely help.

> **Critical operational note.** `default_worker_width()` is
> `available_parallelism().min(4)` (`src/handoff.rs:62`). **Raising the CPU limit
> alone cannot widen the pool past 4.** `LLM_D_SC_INFERENCE_WORKERS` must be set
> explicitly, and the CPU limit raised to match, or the arm is CPU-starved.

**Recommendation: 8 workers / 8 CPU per replica** — the hit-path optimum and
within 40 % of the miss-path optimum. Go to 16–32 only if the workload is
miss-dominated.

### Horizontal — replicas

Horizontal scaling is *better on the expensive path*, which is the useful
direction:

| Replicas | hit scaling | miss scaling |
|---:|---:|---:|
| 2 | 1.90× | 1.94× |
| 4 | 2.52× | 3.62× |
| 8 | 3.58× | **5.76×** |

The hit path saturates shared resources (network, driver, transport) before the
service; the miss path is CPU-bound per replica and parallelises well.

**Recommendation: scale horizontally for miss-heavy workloads; scale vertically
(to 8 workers) first for hit-heavy ones.**

---

## 4. Semantic cache: when to turn it on

**Measured result: the L2 semantic cache produced no throughput benefit in any
arm, and cost ~5 % on the hit path.**

The tier was genuinely active — the service logged
`semantic cache enabled (redis-semantic, threshold 0.9)` and Redis created index
`sc_semantic_idx`, so this is a real measurement rather than a silent
degradation to exact-only.

| Arm | exact req/s | redis-semantic req/s | Δ |
|---|---:|---:|---:|
| hit, keyspace 1 | 105,338 | 100,511 | −4.6 % |
| hit, keyspace 100 | 108,622 | 102,835 | −5.3 % |
| hit, keyspace 10,000 | 146 | 146 | 0 % |
| novel (all misses) | 67 | 67 | 0 % |

### Why — and it is architectural, not a bug

In `ServiceCore::classify` the forward closure runs:

```rust
let embedding = runtime.embed(&input)?;              // ~122 ms  <-- the cost
if let Some(hit) = semantic.lookup(&embedding, &tag) // needs the embedding
    { return Ok(hit); }
let result = runtime.rank(&embedding, &input)?;      // ~microseconds
```

A vector similarity search **requires the query embedding**, so the embedding
must be computed before the L2 lookup can happen. An L2 hit therefore still pays
the expensive part and saves only ranking — 48 cosine similarities, microseconds.

This is confirmed by the route-count sweep: taxonomies of 40, 48 and 50 anchors
all produce ~66 req/s on the miss path. Ranking is free at this scale, so there
is nothing for the semantic cache to save.

### The "large taxonomy" escape hatch was tested, and it does not exist

The obvious counter-argument is that ranking must eventually get expensive
enough to be worth caching. It was tested directly with synthetic taxonomies from
48 to 2,000 anchors (4 labels, 12–500 anchors each). Cache-miss throughput:

| Anchors | exact req/s | redis-semantic req/s | p50 (exact) |
|---:|---:|---:|---:|
| 48 | 67 | 65 | 478.9 ms |
| 200 | 67 | 67 | 477.8 ms |
| 800 | 67 | 67 | 477.9 ms |
| **2,000** | **67** | **67** | 482.1 ms |

**Perfectly flat.** A 42× increase in route count costs nothing measurable,
because ranking is still negligible beside a ~480 ms embedding. There is no
realistic route count at which the semantic cache's saving becomes material.

So it pays off only if one of these changes:

* **A much cheaper embedder** — if embedding dropped toward the cost of ranking.
* **Sharing across replicas** — an L2 hit on replica B for work replica A already
  did. This is real value the single-replica arms here cannot show, and it is the
  one case worth a dedicated multi-replica cold-start test.

**Recommendation for today: leave `LLM_D_SC_CACHE=exact` (the default).** Turn on
`redis-semantic` only after a multi-replica cold-start test demonstrates
cross-replica benefit exceeding the ~5 % hit-path cost. The runtime toggle exists
and the image now ships with the feature compiled in, so this is a one-env-var
decision requiring only a pod restart.

**Not measured: accuracy.** A semantic hit at threshold 0.90 serves a *different
but similar* prompt's label. On a classifier whose job is separating tiers, that
could blur borderline prompts. No accuracy evaluation was run. Do not enable it
in production on latency grounds alone.

---

## 4b. Route count is free

Ranking is `anchor-topk-mean`: one cosine similarity per anchor. Intuition says
more routes must cost more. Measured across 48 → 2,000 anchors, cache-miss
throughput is **flat at 67 req/s** and p50 varies by under 1 %.

**You can add routes without paying for them.** The taxonomy can grow by orders
of magnitude before ranking shows up next to the embedding. For a routing product
this is a strong result: route-table richness is not a performance trade-off.

The caveat is accuracy, not speed — more anchors mean more chances for a
near-miss — but that is a modelling question this campaign did not measure.

## 5. Context size

### Cache-miss (the path that matters for novel prompts)

| Context bytes | req/s | p50 | vs 256 B |
|---:|---:|---:|---:|
| 64 | 57 | 585.6 ms | — |
| 256 | 67 | 481.2 ms | baseline |
| 1,024 | 31 | 1,036.4 ms | **2.2×** |
| 4,096 | 24 | 1,351.7 ms | **2.8×** |
| 16,384 | 21 | 1,530.8 ms | **3.2×** |
| 65,536 | 23 | 1,396.4 ms | 2.9× |

**The hypothesis is confirmed.** Small agent-turn contexts (64–256 B) classify in
~0.5 s; a 1 KB context already doubles that, and 4 KB nearly triples it.

The **plateau above ~16 KB** is not the model getting efficient — it is the
tokenizer truncating at the model's maximum position. Beyond that, extra context
is *discarded*, so a whole-document prompt is both slow **and** classified on a
truncated prefix. That is a correctness concern, not just a latency one.

### Cache-hit

Even hits degrade with size, because the blake3 cache key is computed over the
full normalized text:

| Context bytes | req/s |
|---:|---:|
| 256 | 154,216 |
| 4,096 | 68,775 |
| 16,384 | 28,596 |
| 65,536 | 12,134 |

256 B → 64 KB costs **92 % of hit throughput**.

**Recommendation: llm-d-sc is well suited to per-turn agent context and poorly
suited to whole-document context.** If documents must be routed, classify a
bounded summary or the turn delta, not the document.

---

## 6. Praxis integration — the finding that matters most

Topology: driver → Praxis (`llm_d_sc` filter) → vllm-vcr simulated endpoints.
Praxis exposes a **control listener** (`:8081`) identical to the measured one
(`:8080`) except that cluster selection is a static `router` rather than the
classifier, so the delta is the cost of deciding *semantically* and nothing else.

### Warm filter overhead is small

| Listener | req/s | p50 | p90 | p99 |
|---|---:|---:|---:|---:|
| `:8081` static router | 1,430 | 22.362 ms | 23.255 ms | 23.654 ms |
| `:8080` classified | 1,271 | 24.425 ms | 27.655 ms | 35.528 ms |

**+2.06 ms p50, −11 % throughput.** For a routing decision on the request path,
that is cheap.

### But novel prompts do not get classified at all

With the POC default `timeout_ms: 100` and a cache-miss forward of ~120 ms:

| Workload | p50 | Tier actually reached |
|---|---:|---|
| Cached prompts | 103.35 ms | `large` (92 ms backend) — **classified** |
| **Novel prompts** | 23.99 ms | `general` → small (22 ms) — **failed open** |

Backend latencies were verified directly (small 22.37 ms, large 92.05 ms), so the
tier reached is unambiguous.

**Any prompt the classifier has not seen before is routed by fail-open, not by
classification.** The gateway returns 200 and looks healthy; the routing simply
is not happening. This is invisible in error rates.

Raising the timeout fixes correctness and destroys latency:

| `timeout_ms` | req/s | p50 | Outcome |
|---:|---:|---:|---|
| 100 | 270 | 26.6 ms | fails open |
| 250 | 72 | 273.4 ms | classifies |
| 500 | 64 | 249.0 ms | classifies |
| 1000 | 23 | 683.3 ms | classifies, degraded |

**Recommendation.** Do not ship `timeout_ms: 100` with a CPU classifier. Either
warm the cache for the expected prompt distribution, accept ~250 ms on first
touch, or move classification off the synchronous request path. And **alarm on
fail-open rate** — a gateway that silently stops routing is worse than one that
errors, because nothing pages.

---

## 7. What was NOT measured

Stated plainly, per house rule 7:

* **Single operator, single cluster, not independently reproduced.** Shared
  CoreWeave cluster with other tenants' workloads on neighbouring nodes.
* **Backends are simulated.** vllm-vcr with a real vLLM Rust frontend and
  configurable TTFT/ITL — good for latency shape, not a real model server.
* **No accuracy evaluation** of semantic-cache hits at threshold 0.90.
* **No cross-replica semantic-cache test**, which is where L2 would most plausibly
  pay off.
* **Miss-path arms have fewer samples** (~1–3 k) than hit-path arms (~2 M), so
  miss-path p99.9 is not well determined. p50/p90 are sound.
* **The `llm-d` integration was not benchmarked.** The integration PR could not
  be located: `llm-d/llm-d`, `praxis-proxy/praxis`, every `llm-d-incubation`
  repository and `inference-payload-processor-rs` were searched without finding
  it. The harness is topology-agnostic — `scbench --mode http` will drive any
  OpenAI-shaped gateway — so this needs only a pointer to the PR and an endpoint,
  not new tooling.
* **The vLLM Semantic Router integration is a stretch goal** and was not started.

## 8. Summary recommendations

| # | Recommendation | Basis |
|---|---|---|
| 1 | Run **8 workers / 8 CPU** per replica | hit-path peak; §3 |
| 2 | Hold **offered concurrency 32–64** per replica | knee at 64, retrograde beyond; §1b |
| 3 | Size for **256 in-flight per replica**; scale out past that | hard admission bound; §1a |
| 4 | **Raise `timeout_ms` above the forward latency, or warm the cache** | novel prompts fail open; §6 |
| 5 | **Alarm on fail-open rate**, not just error rate | silent routing loss; §6 |
| 6 | Keep **`LLM_D_SC_CACHE=exact`** until cross-replica benefit is shown | §4 |
| 7 | Classify **turn deltas, not documents** | 3× latency + truncation; §5 |
| 9 | **Add routes freely** — 48→2,000 anchors costs nothing | §4b |
| 8 | Make `DEFAULT_QUEUE_BOUND` runtime-configurable | operators cannot tune admission; §1a |
