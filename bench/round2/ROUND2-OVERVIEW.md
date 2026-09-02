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

## What is still open

* **llm-d-sc is not yet integrated with llm-d.** The llm-d arm measures the llm-d
  gateway itself (EPP endpoint selection). No integration PR was found in
  `llm-d/llm-d`, `praxis-proxy/praxis`, any `llm-d-incubation` repository, or
  `inference-payload-processor-rs`. The natural insertion point is an ext_proc
  filter ahead of the EPP; that is implementation work, not benchmarking.
* **vLLM Semantic Router** — stretch goal, not started.
* **P5 gateway scaling** is backend-bound past 2 replicas.
* **No accuracy evaluation** of semantic-cache hits at threshold 0.90.
