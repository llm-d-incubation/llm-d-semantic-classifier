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

| Path | llm-d-sc in path? | Knee | Peak req/s | **Peak req/min** | p99 at knee | % of backend ceiling |
|---|:--:|---:|---:|---:|---:|---:|
| Praxis control (static routing) | no | 1024 | 65,901 | 3,954,073 | 26.29 ms | — |
| **Backend direct (ceiling)** | n/a | 256 | **47,589** | 2,855,350 | 8.08 ms | 100 % |
| **Praxis + llm-d-sc classification** | **yes** | 128 | **37,738** | 2,264,276 | 6.27 ms | **79 %** |
| **llm-d inference gateway (EPP)** | **no** | 32 | **11,296** | 677,744 | 6.38 ms | **24 %** |
| **llm-d IPP + llm-d-sc** (`ext_proc`→IPP) | **yes** | 256 | **59,925** | 3,595,473 | 9.33 ms | — |
| llm-d IPP control (no `ext_proc`) | no | 512 | 126,708 | 7,602,463 | 7.98 ms | — |

> The two IPP rows exceed the 47,589 req/s backend ceiling measured earlier
> because that ceiling was taken against **three** `vcr-small` replicas at 200 ms
> on the large tier; the IPP arms ran with both tiers at 0 ms simulated latency.
> Compare IPP rows to each other, and Praxis rows to each other.

**Two different llm-d paths are measured, and only one carries llm-d-sc.**

* **llm-d inference gateway (EPP)** — Istio Gateway → InferencePool → EPP running
  `queue-scorer`, `prefix-cache-scorer`, `active-request-scorer` against live pod
  metrics. **No llm-d-sc**: verified, not assumed — the EPP plugin config and args
  contain zero references to it.
* **llm-d IPP + llm-d-sc** — the `llm-d-ipp-scorer` POC
  ([llm-d-inference-payload-processor#299](https://github.com/llm-d/llm-d-inference-payload-processor/issues/299)):
  Envoy `ext_proc` → IPP → llm-d-sc gRPC, where the `llm-d-sc-scorer` plugin
  classifies the prompt and scores candidate models. **This is llm-d WITH
  llm-d-sc**, and it is the strongest-performing classified path in the campaign.

That asymmetry cuts **against** Praxis in the table, so read it two ways:

* **Like for like (neither classifying):** Praxis control 65,901 req/s vs the
  llm-d EPP gateway's 11,296 req/s — a **5.8× gap**.
* **Praxis handicapped:** Praxis running full semantic classification (37,738)
  is still **3.3× faster than the EPP gateway doing no classification at all**.
* **But llm-d's IPP path beats both.** Envoy `ext_proc` → IPP → llm-d-sc reaches
  **59,925 req/s while classifying** — more than Praxis's classified 37,738 and
  more than five times the EPP gateway. Two things make that comparison
  uncomfortable rather than decisive, and both are stated rather than buried:
  the IPP arm ran with both simulated tiers at 0 ms while the earlier ceiling was
  taken with one tier at 200 ms; and it did so on **less CPU** (Envoy 4 req/12
  limit + IPP 8/16, against Praxis 16/32). A CPU-matched rerun is the honest next
  step before treating this as a verdict on either proxy.

Neither reading is a like-for-like *routing-quality* comparison: llm-d's EPP picks
an endpoint from live queue-depth and prefix-cache state, which Praxis's static
control listener does not attempt. This is a capacity comparison.

**Praxis reaches 79 % of what the backends can absorb unaided, while carrying
semantic classification.**

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

## Latency and tail stability: Praxis vs llm-d IPP

Throughput comparisons hid this. Both paths classify, both front the same
backends, so latency at matched concurrency is the fair read.

### Median and tail, matched concurrency

| Conc | Praxis p50 | Praxis p99 | IPP p50 | IPP p99 |
|---:|---:|---:|---:|---:|
| 8 | **1.957 ms** | **2.898 ms** | 2.003 ms | 3.015 ms |
| 32 | 2.314 ms | 3.492 ms | **2.094 ms** | **3.323 ms** |
| 128 | 3.646 ms | 6.275 ms | **3.207 ms** | **5.723 ms** |
| 256 | 6.725 ms | 11.040 ms | **4.747 ms** | **9.333 ms** |
| 512 | 12.755 ms | 22.662 ms | **8.122 ms** | **16.222 ms** |

Praxis is **faster at low load** (c8) and near-tied to c128; IPP pulls ahead only
under heavy load.

### Tail *ratio*: Praxis is tighter at every load

p99/p50 — how far the tail sits from the median:

| Conc | Praxis | IPP |
|---:|---:|---:|
| 8 | **1.48×** | 1.51× |
| 64 | **1.68×** | 1.72× |
| 256 | **1.64×** | 1.97× |
| 512 | **1.78×** | 2.00× |

### But past saturation, Praxis's EXTREME tail diverges

This is the finding a throughput table cannot show. Both arms at concurrency 512,
**zero errors** in each:

| | p50 | p99 | **p99.9** | **max** | p99.9/p99 |
|---|---:|---:|---:|---:|---:|
| **Praxis** | 12.755 ms | 22.662 ms | **352.443 ms** | **467.864 ms** | **15.55×** |
| **IPP** | 8.122 ms | 16.222 ms | 22.908 ms | 33.502 ms | **1.41×** |

At concurrency 1024 Praxis is worse still: p99 57.5 ms, **p99.9 754.9 ms, max
1,336 ms** — 1.3-second stalls on 1-in-1000 requests, reported as successes.

**Both intuitions are correct, at different loads:**

* **At or below concurrency 256** — Praxis is the more predictable proxy: tighter
  p99/p50 *and* comparable p99.9 (14.4 ms vs IPP's 16.0 ms).
* **Past its knee (512+)** — Praxis stops degrading gracefully. Its failure mode
  is rare, severe stalls rather than uniform slowdown; IPP's worst case stays
  within 1.41× of its p99.

For an enterprise gateway that is a defensible trade — excellent behaviour in the
operating range, poor behaviour past it — but it makes **staying under the knee
an SLO requirement, not a tuning preference**, and it means p99 alone will not
detect the problem. Alarm on p99.9.

The resource asymmetry cuts against Praxis here too: 16 req/32 limit CPU versus
Envoy 4/12 plus IPP 8/16.

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

* **The EPP gateway arm still has no llm-d-sc**, and the IPP arm is a different
  insertion point (payload processor, not endpoint picker). Classification cost
  *on the EPP path specifically* remains unmeasured.
* **CPU parity between the gateways was not controlled.** Praxis ran on 16 req/32
  limit; Envoy+IPP on 4/12 and 8/16. The IPP path's advantage may be partly or
  wholly a proxy-efficiency difference rather than an architectural one.
* **No PR raised** for the vLLM SR adapter. It works and is benchmarked here, but
  upstreaming it needs a decision on where the softmax belongs: in an adapter, or
  as an optional normalised-score mode in llm-d-sc itself.
* **P5 gateway scaling** is backend-bound past 2 replicas.
* **No accuracy evaluation** of semantic-cache hits at threshold 0.90.
