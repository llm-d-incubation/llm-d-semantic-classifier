# llm-d-sc v0.2-staging — performance characterisation

**Cluster:** CoreWeave *waldorf* · **Namespace:** `cnuland-dev` · **Date:** 2026-09-02
**Branch under test:** `v0.2-staging` (semantic cache #22 + context-completeness #23)

Every figure below is generated from captured JSON in `results/json/`, with raw
per-request samples retained. Nothing is transcribed by hand. The companion
`STATISTICAL-REPORT.md` carries the complete distributions; this document
explains what they mean.

---

## Corrections to the first pass

House rule 8: corrections are published, not quietly replaced. Both datasets are
retained — original arms under `c1/c2/c3`, corrected ones under `c1v2/c2v2/c3v2`.

Continued benchmarking found a **bug in llm-d-sc itself** that was setting p99 for
every cache-hit measurement, plus a bug in my own driver. Two headline conclusions
from the first pass did not survive.

**Root cause — Nagle was enabled on every accepted connection.**
`Server::builder().tcp_nodelay(..)` only applies when tonic owns the listener. The
server uses `serve_with_incoming` with its own `TcpListenerStream` (so accepted
connections can be counted for I-008), so tonic never touched the socket. The
client half was already correct. Histogramming the tail rather than reading
percentiles made it unmistakable — over 200,000 requests:

| Band | Requests | Share |
|---|---:|---:|
| < 5 ms | 196,342 | 98.17 % |
| 5–35 ms | 4 | 0.00 % |
| **40–42 ms** | **3,658** | **1.83 %** |

Nothing occupies the gap. A hard cluster at exactly 40 ms is the peer's Linux
delayed-ACK timer, not service latency — and that 1.83 % alone set p99 (~40.9 ms)
for every hit arm. Fixed in `f57d4ef`; measured effect at concurrency 128:

| | Before | After |
|---|---:|---:|
| Throughput | 117,728 req/s | **219,617 req/s** (+86 %) |
| p99 | 40.872 ms | **1.225 ms** (33× better) |
| 40–45 ms band | 3,658 samples | **0 samples** |

**Correction 1 — vertical scaling does not peak at 8 workers and decline.**
The apparent 41 % fall past 8 workers was the delayed-ACK tail. Hit-path
throughput saturates at ~16 workers and then holds flat:

| Workers | Before | After |
|---:|---:|---:|
| 8 | 148,875 | 160,073 |
| 16 | 107,795 ↓ | **284,513** ↑ |
| 32 | 87,896 ↓ | 287,957 |
| 48 | — | 282,512 |

**Correction 2 — there is no retrograde region on the offered-load ladder.**
With Nagle disabled and a per-request global mutex removed from my driver, mean
latency stays pinned at ~1.1 ms while throughput scales linearly (57k → 118k →
226k at concurrency 64/128/256). A system whose latency does not rise with
offered load is not saturated; the "knee at 64" was measurement apparatus.

**Driver defects fixed** (they affected the numbers, so they are disclosed):
*responses were never validated* — a transport-level `Ok` proves bytes came back,
not that a classification happened. Re-validation showed all 200,000 responses
returning `OK` with label `COMPLEX`, so the throughput was real work, but the
check should have been there from the start (rule 3). And *status tallying held a
global mutex per request*, making the driver itself the ceiling.

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

### 1b. Concurrency: latency, not throughput, is what degrades

*(Corrected — the first pass reported a retrograde region past concurrency 64.
That was measurement apparatus; see Corrections.)*

Post-fix ladder, 1 replica, 4 workers, cache-hit:

| Concurrency | req/s | p50 | p99 | errors |
|---:|---:|---:|---:|---:|
| 16 | 84,445 | 0.180 ms | 0.331 ms | 0 |
| 32 | 103,874 | 0.240 ms | 0.552 ms | 0 |
| 64 | 113,851 | 0.440 ms | 1.074 ms | 0 |
| 128 | 107,259 | 0.923 ms | 19.740 ms | 0 |
| 256 | 107,133 | 1.786 ms | 18.581 ms | 0 |
| 384 | 113,430 | 2.572 ms | 24.446 ms | **14,980** |
| 512 | 105,362 | 3.682 ms | 23.647 ms | 25,664 |

Throughput **plateaus** around 105–114 k req/s (at 4 workers) rather than
collapsing. What degrades is latency: p50 rises roughly linearly with offered
concurrency past 64, and p99 steps up an order of magnitude between 64 and 128.

With 16 workers and 64 connections the same single replica reaches **302,895
req/s**, so the plateau above is a worker-width limit, not a service-path limit.

**Operating point: offered concurrency 32–64 per replica** — beyond that you buy
latency, not throughput.

### 1c. Compute bound: the model forward (cache-miss)

On the miss path the ModernBERT forward dominates absolutely. Service-reported
stage timings at 256 B: `tokenize p50=80µs`, `forward p50=122.88ms`,
cache-hit `total p50=15µs`. The forward is **~1,500× the tokenizer** and
**~8,000× a cache hit**.

**Verdict:** on the hit path the bottleneck is contention/admission in the
service path, not CPU. On the miss path it is unambiguously the model forward.

---

## 2. How many connections per minute?

**The honest answer: it depends almost entirely on your cache hit ratio, and the
range is 2,000×.** Everything else — workers, replicas, transport — moves the
number by single-digit multiples. Hit ratio moves it by three orders of magnitude.

### The curve that matters (1 replica, 16 workers, 256 B)

| Hit ratio | req/s | **req/min** | p50 | p99 |
|---:|---:|---:|---:|---:|
| 0 % | 113 | 6,810 | 567.63 ms | 621.59 ms |
| 50 % | 228 | 13,671 | 282.23 ms | 422.92 ms |
| 80 % | 579 | 34,730 | 94.69 ms | 282.40 ms |
| 90 % | 1,134 | 68,037 | 43.40 ms | 216.61 ms |
| 95 % | 2,225 | 133,525 | 15.53 ms | 185.22 ms |
| 99 % | 10,350 | 620,981 | 0.44 ms | 122.01 ms |
| 100 % | 225,964 | **13,557,855** | 0.27 ms | 0.54 ms |

**Each additional nine of hit ratio multiplies throughput roughly tenfold.** The
last 1 % of misses costs ~96 % of the achievable throughput. If you take one
number away from this campaign, take this curve — capacity planning for llm-d-sc
is cache-hit-ratio planning.

### Ceilings at the extremes

| Configuration | req/s | req/min |
|---|---:|---:|
| 1 replica, 16 workers, 64 connections, 100 % hit | 302,895 | 18,173,700 |
| 4 replicas, 16 workers, 100 % hit | 614,483 | 36,868,980 |
| 8 replicas, 8 workers, 100 % hit | 651,512 | **39,090,720** |
| 1 replica, 0 % hit (every prompt novel) | 113 | 6,810 |
| 8 replicas, 0 % hit | 404 | 24,217 |

Quote the number **with its hit ratio**. "39 million requests/minute" and "24
thousand requests/minute" are both true statements about the same software on the
same hardware.

## 2b. Connections are a client-side lever worth 6×

Concurrency and connections are different things, and the second is easy to get
wrong. Holding offered concurrency **fixed at 256** and varying only the number of
HTTP/2 connections:

| Connections | req/s | p50 |
|---:|---:|---:|
| 1 | 47,845 | 5.321 ms |
| 2 | 88,810 | 2.860 ms |
| 4 | 152,791 | 1.606 ms |
| 8 | 223,339 | 1.089 ms |
| 16 | 251,276 | 0.989 ms |
| 32 | 290,192 | 0.850 ms |
| **64** | **302,895** | 0.812 ms |
| 128 | 301,883 | 0.780 ms |
| 256 | 291,570 | 0.819 ms |

A single HTTP/2 connection caps at **47,845 req/s** regardless of how much
concurrency you offer it. Pooling to 64 connections is worth **6.3×** on identical
offered load, and there is nothing to gain past 64.

**Recommendation: client connection pool of 32–64 per llm-d-sc Service.** This
costs nothing and is the single cheapest throughput win available.

## 2c. ClusterIP is not a bottleneck

Paired against deterministic client-side fan-out to all Pod IPs, same targets,
same connection count, 4 replicas:

| Concurrency | ClusterIP | Direct Pod-IP | Ratio |
|---:|---:|---:|---:|
| 128 | 406,151 | 412,949 | 0.984 |
| 512 | 609,292 | 614,483 | 0.992 |

The Service layer costs **0.8–1.6 %**. This independently reproduces Intel's
Arena finding (their ClusterIP/direct ratio ranged 0.962–1.028). Bypassing
`kube-proxy` is not worth the operational complexity.

## 3. Ideal scaling

### Vertical — executor workers (`LLM_D_SC_INFERENCE_WORKERS`)

Corrected post-`tcp_nodelay`. The two paths behave differently, but neither
declines:

| Workers | hit req/s | hit p99 | miss req/s | miss p50 |
|---:|---:|---:|---:|---:|
| 1 | 41,959 | 15.005 ms | 9 | 3,415.99 ms |
| 2 | 60,576 | 21.478 ms | 18 | 1,742.24 ms |
| 4 | 82,472 | 20.687 ms | 35 | 906.52 ms |
| 8 | 160,073 | 4.648 ms | 66 | 482.53 ms |
| **16** | **284,513** | **0.859 ms** | 116 | 278.05 ms |
| 32 | 287,957 | 0.910 ms | 175 | 179.98 ms |
| 48 | 282,512 | 1.025 ms | **200** | **160.93 ms** |

* **Cache-hit saturates at ~16 workers** (284k req/s, p99 0.859 ms) and then holds
  flat to 48. Extra threads neither help nor hurt.
* **Cache-miss keeps improving** to 48 workers — it is compute-bound on the model
  forward, so more executor threads genuinely add capacity (per-doubling: 2.00×,
  1.94×, 1.89×, 1.76×, 1.51×, then 1.14× at 48 as it flattens).

> **Operational trap.** `default_worker_width()` is
> `available_parallelism().min(4)` (`src/handoff.rs:62`). **Raising the CPU limit
> alone cannot widen the pool past 4.** `LLM_D_SC_INFERENCE_WORKERS` must be set
> explicitly and the CPU limit raised to match.

**Recommendation: 16 workers / 16 CPU per replica.** That is the hit-path
saturation point and delivers sub-millisecond p99. Go to 32–48 only for
miss-dominated traffic.

### Horizontal — replicas

| Replicas | hit req/s | scaling | miss req/s | scaling |
|---:|---:|---:|---:|---:|
| 1 | 156,054 | 1.00× | 65 | 1.00× |
| 2 | 325,034 | 2.08× | 130 | 2.00× |
| 4 | 573,646 | 3.68× | 240 | 3.69× |
| 8 | 651,512 | 4.17× | 404 | **6.22×** |

Horizontal scaling is **near-linear on the miss path** (6.22× at 8 replicas) and
sublinear on the hit path, where shared resources (driver, network, transport)
bind before the service does.

**Recommendation: scale horizontally for miss-heavy traffic; reach 16 workers
first for hit-heavy traffic.**

## 3b. Stability under sustained load

A 10-minute soak at 95 % hit ratio, concurrency 256, keyspace 2,000:

| Metric | Value |
|---|---:|
| Requests | 1,375,603 |
| Throughput | 2,293 req/s |
| **Errors** | **0** |
| p50 / p90 / p99 | 104.38 / 159.14 / 275.26 ms |
| p99.9 / max | 1,923.74 / 2,133.83 ms |

Throughput matches the 30-second run at the same hit ratio (2,225 req/s), so
there is **no drift, leak or degradation** over 1.4 M requests. The higher p50
versus the short run is Little's law, not decay — same saturated throughput at 4×
the concurrency.

## 4. Semantic cache: when to turn it on

**Answer: not at any hit ratio, not at any route count, and not across replicas.
As currently architected it cannot help. Leave `LLM_D_SC_CACHE=exact`.**

This is a stronger claim than the first pass made, and it is now supported by
three independent tests rather than one. The tier was genuinely active throughout
— the service logged `semantic cache enabled (redis-semantic, threshold 0.9)` and
Redis created index `sc_semantic_idx`, so these are real measurements and not a
silent degradation to exact-only.

### Test 1 — across the entire realistic hit-ratio range

| Hit ratio | exact req/s | redis-semantic req/s | Δ |
|---:|---:|---:|---:|
| 0 % | 113 | 113 | −0.5 % |
| 50 % | 228 | 226 | −0.9 % |
| 80 % | 579 | 570 | −1.6 % |
| 90 % | 1,134 | 1,093 | −3.6 % |
| 95 % | 2,225 | 2,195 | −1.3 % |
| 99 % | 10,350 | 10,242 | −1.0 % |
| 100 % | 225,964 | 214,730 | −5.0 % |

**Never faster. Always 0.5–5.0 % slower.**

### Test 2 — large taxonomies do not rescue it

The natural objection is that ranking must eventually get expensive enough to be
worth caching. Synthetic taxonomies from 48 to 2,000 anchors, cache-miss path:

| Anchors | exact req/s | redis-semantic req/s | p50 (exact) |
|---:|---:|---:|---:|
| 48 | 67 | 65 | 478.9 ms |
| 200 | 67 | 67 | 477.8 ms |
| 800 | 67 | 67 | 477.9 ms |
| 2,000 | 67 | 67 | 482.1 ms |

Perfectly flat. A 42× increase in route count costs nothing measurable.

### Test 3 — cross-replica reuse, the case most likely to favour it

This was the one scenario the first pass flagged as untested and plausibly
favourable: replica B reusing work replica A already did. Four replicas, cold
Redis flushed between arms, 2,000 distinct keys cycled:

| Arm | req/s | **Model forwards (L1 miss delta)** |
|---|---:|---:|
| exact | 318,509 | **8,000** |
| redis-semantic | 332,074 | **8,000** |

8,000 = 4 replicas × 2,000 keys. **Identical.** The L2 tier eliminated exactly
zero model forwards. Replica B did not reuse replica A's work.

### It WORKS — that was never verified in the first pass, and it should have been

An earlier revision argued the L2 tier saves nothing, but never checked that it
functionally hits at all. Those are different claims. It does hit, and it hits
almost every time.

On an L2 hit the code returns **before** `semantic.insert`, so Redis stops
growing. Vectors-stored versus requests-issued therefore reads out the hit rate
directly. Flushing Redis and running 3,425 novel prompts:

| Metric | Value |
|---|---:|
| Requests issued | 3,425 |
| Vectors stored in Redis | **6** |
| **Implied L2 hit rate** | **99.8 %** |

Throughput over that run was 114 req/s, against 116 req/s with L2 disabled —
identical. (The apparent p50 difference, 134.7 ms vs 278.1 ms, is Little's law:
concurrency 16 versus 32, not a saving.)

So the tier is not broken and not rarely-hitting. **It hits nearly always and
still saves nothing measurable**, which is exactly what the code predicts.

It does not help through the gateway either. On novel prompts through Praxis at
`timeout_ms: 100`:

| Cache | req/s | Classified correctly | Redis vectors |
|---|---:|---:|---:|
| exact | 1,859 | 6.6 % | 0 |
| redis-semantic | 1,713 | 7.4 % | 3 |

An L2 hit rate near 100 % moves routing correctness by under a percentage point,
because the embedding still has to happen before the lookup can be issued.

> **Accuracy warning, now demonstrable.** 3,425 distinct prompts collapsed into
> **6** cached answers at threshold 0.90. These synthetic prompts share identical
> filler text so this overstates the effect for real traffic, but the mechanism is
> real: at 0.90 the tier will serve one prompt's label for another. Any decision
> to enable it needs an accuracy evaluation, which this campaign did not run.

### Why it cannot help — architectural, not a defect

In `ServiceCore::classify` the forward closure runs:

```rust
let embedding = runtime.embed(&input)?;              // ~480 ms  <-- the cost
if let Some(hit) = semantic.lookup(&embedding, &tag) // needs the embedding
    { return Ok(hit); }
let result = runtime.rank(&embedding, &input)?;      // microseconds
```

A vector similarity search **requires the query embedding**, so the embedding must
be computed before the L2 lookup can happen. An L2 hit therefore still pays the
expensive part and saves only ranking. That is true on the same replica, on a
different replica, and at any taxonomy size — which is exactly what all three
tests show.

**For semantic caching to pay off, the lookup would have to be reachable without
first embedding** — for example keyed on a cheap lexical signature, with the
embedding computed only on an L2 miss. That is an architectural change, not a
tuning decision, and well outside "minor bug fix" scope.

**Recommendation: keep the default `LLM_D_SC_CACHE=exact`.** The runtime toggle
and the compiled-in feature are both worth keeping — they cost nothing when off,
and they make the tier available the moment the lookup path changes. But there is
no configuration today in which turning it on is the right call on performance
grounds.

**Not measured: accuracy.** A semantic hit at threshold 0.90 serves a *different
but similar* prompt's label. On a classifier whose job is separating tiers that
could blur borderline prompts. No accuracy evaluation was run.

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

### Filter overhead is not a constant — it is a capacity tax

At low concurrency the `llm_d_sc` filter is cheap: **+2.06 ms p50, −11 %
throughput** against an identical static-router control. That number is true and
was the first pass's headline, but it is misleading on its own. Under a full
saturation ladder — both listeners paired, backends scaled and set to zero
simulated latency so the gateway is the limiter:

| Concurrency | Classified req/s | Control req/s | Cost | Classified p50 |
|---:|---:|---:|---:|---:|
| 16 | 7,883 | 9,531 | −17 % | 2.06 ms |
| 64 | 25,629 | 30,674 | −16 % | 2.43 ms |
| 128 | 35,184 | 49,643 | −29 % | 3.52 ms |
| 256 | **37,783** | 57,445 | −34 % | 6.67 ms |
| 512 | 37,297 | 64,726 | −42 % | 13.56 ms |
| 1024 | 35,384 | 75,658 | **−53 %** | 28.38 ms |

**The classified path saturates at ~37,800 req/s (2.27 M req/min) while the
control path is still climbing at concurrency 1024.** For capacity planning the
right statement is "classification roughly halves gateway throughput at
saturation", not "+2 ms per request".

The backends were verified not to be the limit: `vcr-small` × 3 with zero
simulated latency sustains 45,399 req/s directly, above the gateway ceiling.

### RETRACTED: "most routing decisions are silently lost"

**A previous revision of this document claimed that 74–99 % of routing decisions
were lost at `timeout_ms: 100`, even with a warm cache. That claim was wrong and
is withdrawn.** It was an artifact of my own benchmark, not a property of
llm-d-sc or Praxis. Publishing the retraction rather than deleting the claim
(house rule 8).

The error: warmup was counted in **requests**, not in **distinct keys covered**.
With a 300-key working set, many warmup requests re-hit already-warm keys, so the
cache was never fully populated before measurement began. The cache-miss forwards
that leaked into the measurement window exceeded the 100 ms timeout and failed
open — and I attributed that to the timeout rather than to my warmup.

Isolating warmup as the only variable settles it. Same keyspace, same concurrency
(32), same `timeout_ms: 100`, `LLM_D_SC_CACHE=exact`:

| Keyspace | Warmup requests | Classified correctly |
|---:|---:|---:|
| 1 | 800 | 100.0 % |
| 10 | 800 | 100.0 % |
| 50 | 1,500 | 100.0 % |
| 300 | 3,000 | 24.7 % ← under-warmed |
| **300** | **20,000** | **99.7 %** |

A separate concurrency sweep confirms the same conclusion from the other
direction: with a fully warm cache, routing is **100 % correct at every offered
concurrency from 1 to 128** at `timeout_ms: 100`.

### What is actually true about the timeout

The real, and much narrower, statement:

* **A warm classifier routes correctly at `timeout_ms: 100`** — at any
  concurrency tested, for any working set that has actually been warmed.
* **A cache MISS costs ~480 ms and cannot complete inside a 100 ms budget**, so
  genuinely novel prompts fail open to the default cluster. That part of the
  original finding stands.
* Therefore the exposure is **cold start and working-set churn**, not steady
  state. A service restart, a scale-up, or a shift in the prompt distribution
  produces a window in which routing silently degrades until the cache refills.

| `timeout_ms` | req/s (novel prompts) | p50 | Outcome |
|---:|---:|---:|---|
| 100 | 270 | 26.6 ms | fails open |
| 250 | 72 | 273.4 ms | classifies |
| 500 | 64 | 249.0 ms | classifies |
| 1000 | 23 | 683.3 ms | classifies, degraded |

**Recommendation, revised.** `timeout_ms: 100` is defensible for a warm,
steady-state deployment. What it does not survive is a cold cache. Either
pre-warm the expected prompt distribution at startup, or accept that routing is
approximate until the cache fills — and in both cases measure the fail-open rate
so the cold window is visible rather than assumed.

### A classifier outage is invisible — it looks like a speed-up

The filter fails open by design, which is the right default. The problem is that
the failure is undetectable from any signal an operator normally watches.
Measured with the filter pointed at a dead endpoint (`127.0.0.1:1`, which refuses
inside the pod's own netns and needs no DNS):

| State | req/s | p50 | HTTP status |
|---|---:|---:|---|
| Classifier healthy | 25,522 | 2.46 ms | 100 % 200 |
| **Classifier unreachable** | **29,155** | **2.13 ms** | **100 % 200** |

With the classifier completely gone the gateway is **14 % faster**, error rate is
**zero**, and every request returns 200. On a latency-and-errors dashboard a total
classifier outage reads as a performance improvement. Nothing pages, and all
traffic quietly collapses onto the default cluster.

This Praxis build's admin `/metrics` returns HTTP 200 with a **zero-byte body**,
so there is currently no gateway-side counter to alarm on either.

> **Recommendations.** Do not ship `timeout_ms: 100` with a CPU classifier — it
> loses the majority of routing decisions even when the cache is warm. Export and
> **alarm on the fail-open rate**; a gateway that silently stops routing is worse
> than one that errors, because an erroring gateway pages someone. Treat
> "classification coverage" as an SLI in its own right, separate from latency and
> error rate.

## 6b. Failure modes that DO behave well

The L2 tier degrades correctly. With Redis killed mid-campaign, `redis-semantic`
fell back to exact-only with no impact:

| State | req/s | p50 | errors |
|---|---:|---:|---:|
| Redis reachable | 116 | 278.2 ms | 0 |
| **Redis killed** | 118 | 274.7 ms | **0** |

The consecutive-failure circuit breaker and bounded write-back work as designed.

## 6c. The ABSTAIN path (PR #23) works, and is essentially free

`context_completeness: DELTA` must short-circuit to `ABSTAIN` before any cache or
model work. Verified on the wire:

| `context_completeness` | req/s | p50 | Statuses returned |
|---|---:|---:|---|
| `FULL` | 345 | 178.009 ms | `COMPLEX` × 5,155, `MEDIUM` × 20 |
| **`DELTA`** | **273,561** | **0.219 ms** | `ABSTAIN` × 4,103,707 |

**793× faster**, confirming it returns before the model forward. The `FULL` arm
also shows the classifier genuinely discriminating between labels rather than
returning a constant — useful independent evidence that the throughput figures
elsewhere represent real classification work.

## 7. What was NOT measured

Stated plainly, per house rule 7:

* **Single operator, single cluster, not independently reproduced.** Shared
  CoreWeave cluster with other tenants' workloads on neighbouring nodes.
* **Backends are simulated.** vllm-vcr with a real vLLM Rust frontend and
  configurable TTFT/ITL — good for latency shape, not a real model server.
* **No accuracy evaluation** of semantic-cache hits at threshold 0.90.
* **Praxis horizontal scaling was not tested** — all gateway arms used a single
  Praxis replica.
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
| 1 | **Plan capacity by cache hit ratio first.** It spans 2,000×; nothing else comes close | §2 |
| 2 | Run **16 workers / 16 CPU** per replica (not 8) | §3 |
| 3 | **Client connection pool of 32–64** — worth 6.3× on identical load | §2b |
| 4 | Hold **offered concurrency 32–64** per replica | §1b |
| 5 | Size for **256 in-flight per replica**; scale out past that | §1a |
| 6 | `timeout_ms: 100` is fine WARM; pre-warm the cache or routing degrades silently while cold | §6 |
| 7 | **Alarm on fail-open rate / classification coverage** — an outage looks like a speed-up | §6 |
| 8 | Keep **`LLM_D_SC_CACHE=exact`** — no configuration makes L2 pay today | §4 |
| 9 | Classify **turn deltas, not documents** | §5 |
| 10 | **Add routes freely** — 48→2,000 anchors costs nothing | §4b |
| 11 | Don't bypass ClusterIP — it costs 0.8–1.6 % | §2c |
| 12 | Budget **~half your gateway throughput** for classification at saturation | §6 |
| 13 | Make `DEFAULT_QUEUE_BOUND` runtime-configurable | §1a |
| 14 | Export gateway routing counters — Praxis `/metrics` is currently empty | §6 |
| 15 | `context_completeness: DELTA` is a free fast path (793×) — use it for follow-up turns | §6c |
