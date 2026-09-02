# Round 2 — Praxis and llm-d, characterised

**Cluster:** CoreWeave *waldorf* · **Namespace:** `cnuland-dev` · **Branch:** `v0.2-staging`
**Runs:** 269 captured arms · raw per-request samples retained

Round 1 produced two wrong conclusions and one retracted finding. Every one came
from an unverified assumption rather than a bad measurement, so round 2 was rebuilt
around a harness that **refuses to report an arm whose premises it cannot confirm**.
All figures here are generated from `results/json/`; see `ROUND2-STATISTICAL-REPORT.md`
for complete distributions.

---

## What round 1 got wrong, and what changed

| Defect | Effect on round-1 numbers | Fix |
|---|---|---|
| `tcp_nodelay` never set on accepted sockets (**llm-d-sc bug**) | 1.83 % of requests in a hard 40–42 ms delayed-ACK cluster, which set p99 for *every* cache-hit arm | fixed in `f57d4ef`: **+86 % throughput, p99 40.9 → 1.2 ms** |
| Driver counted transport `Ok` as success | throughput could have been empty responses | responses now validated (all returned real labels) |
| Driver held a global mutex per request | the *driver* was the ceiling | per-worker tallies |
| Warmup counted in requests, not keys covered | partially cold cache measured as service latency → **retracted** routing-loss claim | warmup floor of 40 req/key, auto-raised |
| Latency-differentiated backends used for capacity arms | classified path pinned to the slow tier; measured vllm-vcr, not the gateway | separate uniform-fast config for capacity |

---

## Headline: gateway capacity, same backends, same driver

The single most useful comparison in this campaign. Both gateways front the
**identical** vllm-vcr backends, so the difference is the gateway.

| Path | Knee (concurrency) | Peak req/s | **Peak req/min** | p99 at knee | % of backend ceiling |
|---|---:|---:|---:|---:|---:|
| Praxis control (static routing) | 1024 | 65,901 | 3,954,073 | 26.29 ms | — |
| **Backend direct (ceiling)** | 256 | **47,589** | 2,855,350 | 8.08 ms | 100 % |
| **Praxis + llm-d-sc classification** | 128 | **37,738** | 2,264,276 | 6.27 ms | **79 %** |
| **llm-d inference gateway (EPP)** | 32 | **11,296** | 677,744 | 6.38 ms | **24 %** |

**Praxis with semantic classification sustains ~3.3× the throughput of the llm-d
inference gateway** on this hardware, and reaches 79 % of what the backends can
absorb unaided.

> **Do not add the vLLM SR adapter to this table.** An earlier revision listed it
> here and it was misread as a faster gateway. It is not a gateway: it classifies
> and returns, and never contacts a backend. The tell is that it measures
> **53,171 req/s — above the 47,589 req/s backend ceiling.** Nothing that has to
> proxy to those backends can exceed their capacity, so a number above the
> ceiling is proof the work is different, not proof the software is faster. Its
> figures live in their own section below.

### The same classifier, at three levels of work

Comparing like with like makes the picture obvious:

| Path | req/s | What it adds |
|---|---:|---|
| llm-d-sc raw gRPC | 302,895 | classification only, binary protocol |
| llm-d-sc via the HTTP adapter | 53,171 | + HTTP/1.1 and JSON encode/decode (**5.7×**) |
| Praxis full gateway (classified) | 37,738 | + body buffering, prompt extraction, cluster selection, **backend proxy and response** |

Wire payloads for one request confirm it: the adapter moves 93 B in / 158 B out;
Praxis moves 162 B in / 580 B out **and** performs a full backend round trip.

Praxis is doing strictly the most work of the three. The figure that would
actually indict it is its own control listener (65,901 req/s with classification
removed); the gap to 37,738 is the classification cost compounding under load.

A fair caveat: these gateways are not doing the same job. Praxis's `llm_d_sc`
filter picks a **cluster** (which model tier should serve this). llm-d's EPP picks
an **endpoint** (which replica of a pool should serve this) and runs three scoring
plugins — prefix-cache, queue-depth and active-request — against live pod metrics.
Those are complementary decisions, not competing ones. This table compares
*gateway cost and capacity*, not routing quality.

### Cost of classification across the ladder (Praxis, paired)

| Concurrency | Classified req/s | Control req/s | Cost |
|---:|---:|---:|---:|
| 8 | 4,111 | 5,003 | −17.8 % |
| 16 | 7,720 | 9,216 | −16.2 % |
| 32 | 13,929 | 16,985 | −18.0 % |
| 64 | 23,985 | 26,776 | −10.4 % |
| 128 | 34,242 | 40,838 | −16.2 % |
| 512 | 37,738 | 53,768 | −29.8 % |

**~16 % up to the knee, widening past it.** Round 1 reported −53 %, but that arm
was confounded by a latency-differentiated backend; with the destination held
constant the honest number is ~16 % in the useful operating range.

### Cost of the llm-d gateway across the ladder (paired)

| Concurrency | llm-d req/s | Direct req/s | Cost |
|---:|---:|---:|---:|
| 8 | 2,999 | 5,423 | −44.7 % |
| 32 | 7,641 | 17,625 | −56.6 % |
| 128 | 10,752 | 38,889 | −72.4 % |
| 256 | 11,296 | 47,589 | −76.3 % |

---

## Route count is free — at both layers

Round 1 showed ranking cost is flat from 48 to 2,000 **anchors**. Round 2 tests the
other axis: the number of **clusters the gateway chooses between**, with every
cluster pointing at the same backend so only table size varies.

| Routes (clusters) | Cached req/s | Novel req/s | Cached p50 |
|---:|---:|---:|---:|
| 2 | 23,916 | 116 | 2.66 ms |
| 4 | 23,506 | 117 | 2.68 ms |
| 8 | 24,101 | 112 | 2.62 ms |
| 16 | 23,613 | 118 | 2.66 ms |
| 32 | 23,352 | 115 | 2.67 ms |

**Flat across a 16× increase in route-table size.** Combined with the anchor
sweep, routes are free in the classifier *and* at the gateway. For a routing
product this is a strong result: route-table richness is not a performance
trade-off.

---

## Cache hit ratio remains the dominant variable

Through the gateway, exactly as it was directly against the classifier:

| Workload | exact req/s | redis-semantic req/s | p50 (exact) |
|---|---:|---:|---:|
| 100 % cached | 13,838 | 12,760 | 2.33 ms |
| 90 % hit | 1,146 | 1,143 | 8.24 ms |
| 50 % hit | 231 | 236 | 131.21 ms |
| 0 % (all novel) | 113 | 119 | 283.93 ms |

**A 122× throughput range from hit ratio alone**, and `redis-semantic` is
indistinguishable from `exact` at every mix — the fourth independent confirmation
(after the direct hit-ratio sweep, the 2,000-anchor sweep, and the cross-replica
forward count).

## Context size

| Context bytes | Cached req/s | Novel req/s | Novel p50 |
|---:|---:|---:|---:|
| 64 | 24,874 | 150 | 104.75 ms |
| 256 | 23,989 | 116 | 136.70 ms |
| 1,024 | 23,708 | 50 | 318.69 ms |
| 4,096 | 22,863 | 38 | 416.15 ms |
| 16,384 | 15,819 | — | — |

The **cached** path is nearly flat to 4 KB (gateway overhead dominates); the
**novel** path degrades ~4× from 64 B to 4 KB. The turn-vs-document hypothesis
holds: per-turn agent context is cheap, document-sized context is not.

## Gateway horizontal scale — and a measurement limit

| Praxis replicas | Classified req/s | Control req/s |
|---:|---:|---:|
| 1 | 37,510 | 48,414 |
| 2 | 54,686 | 48,587 |
| 4 | 54,295 | 48,140 |

A second Praxis replica lifts the classified path **+46 %**, then it plateaus.
But the control arm is flat at ~48,400 across all replica counts, and measured
**backend headroom was 47,206 req/s** — so both arms are at the backend ceiling
past two replicas. **P5 is backend-bound and cannot answer gateway scaling beyond
2 replicas**; that needs more backend capacity than this run had. Stated rather
than glossed.

---

## vLLM Semantic Router integration (stretch goal — delivered)

llm-d-sc now works with vLLM Semantic Router. The integration is a small adapter,
`bench/round2/vsr-adapter`, and it surfaced one genuine incompatibility worth
recording.

### How the integration works

vLLM SR classifies in-process via its own Candle binding, but it also supports
remote heads: a classifier declared `type: sequence_classifier`, backed by an
external model whose endpoint speaks the `http_classify` contract
(`pkg/classification/http_classifier.go`):

```
POST /classify        Authorization: Bearer <access_key>
  request   {"inputs": "<text>"}
  response  [{"label": "...", "score": 0.93}, ...]
```

llm-d-sc's `Classify` already returns `ranked` as (label, score) pairs over a
versioned taxonomy, so the adapter is a transport bridge: gRPC in, JSON out.

Router-side config:

```yaml
vllm_endpoints:
  - name: llm-d-sc
    llm_provider: openai
    model_role: classification
    llm_endpoint:
      address: llm-d-sc-vsr-adapter
      port: 8080
      protocol: http
classifiers:
  - name: llm-d-sc-complexity
    type: sequence_classifier
    model: llm-d-sc
    labels: [SIMPLE, MEDIUM, COMPLEX, REASONING]
```

### The incompatibility: similarities are not probabilities

**A naive adapter is rejected outright.** vLLM SR validates the contract —
`alignScoresToMapping` errors if the scores do not sum to ~1.0, and the docs are
explicit that "sigmoid multi-label outputs and label subsets are rejected".

llm-d-sc emits **cosine similarities in [−1, 1]**, not a distribution. A real
response before normalisation:

```json
[{"label":"COMPLEX","score":0.99972},{"label":"SIMPLE","score":-0.22546},
 {"label":"REASONING","score":-0.23415},{"label":"MEDIUM","score":-0.60279}]   // sums to -0.063
```

The two systems agree on the **ranking** and disagree on the **score semantics**.
The adapter bridges them with a softmax (monotonic, so llm-d-sc's ordering and
argmax are preserved exactly; sums to 1 by construction; temperature
configurable). Verified end to end:

| Prompt | Top label | Distribution sum |
|---|---|---:|
| "Design a multi-region microservices architecture with failover" | **COMPLEX** (0.560) | 1.000000 |
| "What is the capital of France?" | **SIMPLE** (0.532) | 1.000000 |
| "Prove by induction that the sum of the first n odd numbers is n squared" | **REASONING** (0.558) | 1.000000 |

> **Carry this caveat forward.** These are softmaxed similarities, not calibrated
> probabilities. Any router-side threshold (`gte: 0.5` and friends) is a threshold
> on *this transform* and must be tuned against it, not inherited from a model
> that emits real posteriors.

### How it performs

| Concurrency | req/s | req/min | p50 | p90 | p99 | mean | errors |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 8 | 22,671 | 1,360,282 | 0.321 ms | 0.499 ms | 0.664 ms | 0.352 ms | 0 |
| **32** | **42,387** | 2,543,243 | 0.713 ms | 1.049 ms | **1.372 ms** | 0.754 ms | 0 |
| 128 | 50,281 | 3,016,868 | 2.560 ms | 3.306 ms | 4.098 ms | 2.545 ms | 0 |
| 256 | 51,558 | 3,093,460 | 4.947 ms | 6.463 ms | 8.720 ms | 4.965 ms | 0 |
| 512 | 53,171 | 3,190,263 | 9.589 ms | 12.126 ms | 15.726 ms | 9.606 ms | 2,098 |

**Knee at concurrency 32 (42,387 req/s, p99 1.37 ms); peak 53,171 req/s
(3.19M req/min)**, with the adapter beginning to shed (HTTP 502) at 512.

Every response was validated for contract compliance under load — the driver
counts an unnormalised 200 as a failure, because the router would.

On novel prompts the model forward dominates as everywhere else: 97 req/s at
concurrency 8 (p50 81.2 ms), 117 req/s at 32 (p50 275.8 ms).

**These numbers do not measure vLLM Semantic Router, and are not comparable to
the gateway table.** Two things they are not:

* *Not vLLM SR.* vLLM SR itself was never benchmarked here. This measures the
  adapter that serves llm-d-sc over its contract — the classifier side of the
  integration, not the router.
* *Not a gateway.* The adapter classifies and returns. It never proxies to a
  backend, which is why it can post 53,171 req/s against a backend ceiling of
  47,589 req/s — a rate no proxying gateway could reach.

What they *are* good for: sizing vLLM SR's remote-classifier budget. Its default
`llm_timeout_seconds: 5` is generous against a p99 of 1.37 ms at the knee, but a
**cache-miss** classification costs 81–276 ms, so a cold or churning working set
is the case to plan for — the same conclusion the gateway campaigns reached.

## llm-d: the gateway is the ceiling, not the pool

| InferencePool endpoints | req/s | p50 |
|---:|---:|---:|
| 1 | 10,294 | 12.10 ms |
| 2 | 9,864 | 11.35 ms |
| 3 | 10,489 | 11.62 ms |
| 6 | 10,109 | 12.37 ms |

**Flat.** Adding endpoints to the pool does not raise throughput, because the
gateway saturates around 11,300 req/s regardless. The llm-d ladder also goes
retrograde past its knee — 11,296 req/s at concurrency 256 falls to 9,201 at
1024 while p50 climbs 22.6 → 109.1 ms.

llm-d context sensitivity is steeper than Praxis's cached path: 9,597 req/s at
64 B down to 4,179 at 64 KB (−56 %).

## What is still open

* **llm-d-sc is not yet integrated with llm-d.** The llm-d arm measures the llm-d
  gateway itself (EPP endpoint selection). No integration PR was found in
  `llm-d/llm-d`, `praxis-proxy/praxis`, any `llm-d-incubation` repository, or
  `inference-payload-processor-rs`. The natural insertion point is an ext_proc
  filter ahead of the EPP -- the same shape as the vLLM SR adapter, which is now
  a working precedent for it.
* **No PR raised** for the vLLM SR adapter. It works and is benchmarked here, but
  upstreaming it needs a decision on where the softmax belongs: in an adapter, or
  as an optional normalised-score mode in llm-d-sc itself.
* **P5 gateway scaling** is backend-bound past 2 replicas.
* **No accuracy evaluation** of semantic-cache hits at threshold 0.90.
