# Capacity model: replicas needed for ≥99% classification coverage

Answers "how many llm-d-sc replicas for 99%+ coverage at 20–30k gateway RPS?"

## ⚠ The blocking answer first: replicas do not currently help

**Praxis's `llm_d_sc` filter pins its gRPC channel to a single llm-d-sc pod.**
Measured with 2 replicas at 400 rps novel:

| Pod | Classifications served |
|---|--:|
| `llm-d-sc-…-dnx5s` | **10,404** |
| `llm-d-sc-…-f4llj` | **0** |

**100.0% of traffic to one endpoint.** Scaling 1→2 replicas produced *identical*
capacity (462 → 463 classifications/sec, 1.00×) and coverage halved from 100.7%
to 51.1% because the offered rate doubled while capacity did not.

This is the standard gRPC-over-ClusterIP behaviour: `kube-proxy` load-balances at
**connection** establishment, and a long-lived HTTP/2 channel multiplexes every
request over that one connection forever. Adding replicas adds idle pods.

**Until the filter does client-side load balancing across endpoints — a headless
Service with per-endpoint channels, or an llm-d-style routing layer in front of
the classifier pool — the answer to "how many replicas" is "more replicas will
not help".** This is a code change in the integration, not a tuning decision.

## Measured inputs

| Quantity | Value | How measured |
|---|--:|---|
| Per-replica classification ceiling | **~480/sec** | classified count pins at 11,900–12,150 per 25 s regardless of offered rate |
| Per-replica rate at ≥99% coverage | **~450/sec** | coverage 101.7% at 450, drops to 93.7% at 500 |
| p99 at that point | 321 ms | same arm |
| Comfortable point (p99 ≈ 72 ms) | **~400/sec** | coverage 101.7%, p99 72.1 ms |
| Cross-check, different method | 481.5 rps | direct gRPC, 93.8% verified miss rate |

Two independent methods agreeing on ~480/sec is the strongest number in this
campaign.

## Observed miss rates (this corpus)

| Traffic shape | Miss rate |
|---|--:|
| zipf (heavy-tailed) | **39.6 %** |
| uniform | 59.6 % |
| hotset (80/20) | 69.4 % |

These are **high** because the corpus holds 200,000 unique utterances against a
50,000-entry L1 cache, so the working set cannot fit. A production workload with a
bounded topic space would sit far lower. Treat them as an upper bound, and measure
your own — miss rate is the single input the whole model turns on.

## The arithmetic — once load balancing is fixed

Replicas ≈ (gateway RPS × miss rate) ÷ 450, then divided by scaling efficiency.

Round-1 measured **6.22× at 8 replicas** on the miss path via direct gRPC with
client-side fan-out — about **78 % efficiency**. That is the best available
estimate and is applied below.

| Miss rate | Novel/s at 20k | Replicas @20k | Novel/s at 30k | Replicas @30k |
|--:|--:|--:|--:|--:|
| 1 % | 200 | **1** | 300 | **1** |
| 2 % | 400 | **2** | 600 | **2** |
| 5 % | 1,000 | **3** | 1,500 | **5** |
| 10 % | 2,000 | **6** | 3,000 | **9** |
| 20 % | 4,000 | **12** | 6,000 | **18** |
| 40 % (measured zipf) | 8,000 | **23** | 12,000 | **35** |

**Practical reading:** 20–30k gateway RPS at ≥99% coverage is comfortable at a
**1–5 % miss rate** (1–5 replicas) and expensive above 20 % (12–35 replicas). The
lever with the highest leverage is not replica count — it is **miss rate**, i.e.
cache sizing and working-set locality.

## Confidence

* **High:** the per-replica ~480/sec ceiling (two independent methods), the
  coverage breakpoint at 450→500, and the connection-pinning finding (100.0 %
  to one pod).
* **Unverified:** the 78 % scaling efficiency *through a gateway* — it comes from
  direct gRPC with client-side fan-out, and could not be reproduced through Praxis
  precisely because of the pinning. **The replica counts above are projections,
  not measurements**, and must be re-measured once load balancing exists.
* **Workload-dependent:** every miss rate here is a property of this corpus.
