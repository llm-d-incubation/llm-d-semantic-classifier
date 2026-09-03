# Round 3 — Praxis, llm-d and vLLM Semantic Router, all with llm-d-sc

**Cluster:** CoreWeave *waldorf* · **Namespace:** `cnuland-dev` · **Branch:** `v0.2-staging`
**261 captured runs across 85 replicated arms** · raw telemetry retained

Round 3 exists because an external methodological review found four defects that
made the rounds 1–2 cache-miss conclusions unsafe. Each was fixed in the
**instrument**, not caveated in the prose — and the fixes changed the headline
numbers substantially.

| Defect (round 1–2) | Fix | Effect on conclusions |
|---|---|---|
| `RAYON_NUM_THREADS` uncontrolled; vertical sweep moved 3 variables at once | pinned and swept with CPU held | direction confirmed (**RT=1 ≈ 2.4× RT=4**, replicated); "16 workers is ideal" withdrawn |
| Closed-loop only — cannot observe queue explosion | true open-loop, constant + Poisson | revealed a **latency knee 8× below** the throughput knee |
| One run per cell | 3 reps, randomised, bootstrap 95% CI | CIs span up to 3.5×; single runs were never safe |
| Synthetic filler prompts | frozen 200k-utterance corpus, 12 domains | **classification cost went from ~16% to 45–96%** |
| Premise flags written to an unarchived tmp file | persisted in every result JSON | caught a real 404 arm before it was published |

---

## Headline 1 — classifier inference dominates cost, but the gateway still shapes the tail

At identical realistic traffic, all three stacks converge:

| Traffic shape | Praxis | llm-d (IPP) | vLLM SR adapter |
|---|--:|--:|--:|
| unique (all novel) | 404 | 304 | 302 |
| uniform | 376 | 310 | 309 |
| zipf | 1,334 | 1,178 | 1,183 |
| hotset | 734 | 648 | 651 |

llm-d and vLLM SR are within ~1% of each other at every shape — **even though the
vLLM SR adapter only classifies and never proxies the inference request**. When a
path that skips proxying performs identically to one that doesn't, the shared
component is dominating. That component is llm-d-sc's forward.

**But "the classifier is the bottleneck, not any gateway" would be too strong.**
Praxis runs 10–30% above the other two on throughput, and its *tail* behaviour is
materially different — it holds p90 roughly 2× longer under rising load. The
defensible claim is: **classifier inference is the dominant cost on novel and
mixed traffic, while gateway implementation still materially affects tail
behaviour.**

## Headline 2 — inline classification is a routing-plane capacity cost

Rounds 1–2 measured ~16%. That figure came from `--cache-mode hit --keyspace 1`:
one warm key, 100% cache hits, so classification was nearly free by construction.
With realistic traffic containing genuine misses:

| Concurrency | Praxis cost | llm-d cost |
|--:|--:|--:|
| 32 | **−84.6%** | −82.1% |
| 128 | **−86.8%** | −95.8% |
| 512 | −45.3% | −43.2% |

Both gateways pay **43–96%**. Stated precisely:

> **Inline CPU classification reduces maximum gateway-path throughput by 43–96%
> versus the same gateway without classification, under this simulated-backend
> workload.**

That mouthful matters. These backends return in milliseconds; a real generated
response takes seconds. A 10–100 ms classifier against 5–20 s of generation is a
small fraction of end-to-end latency. **This is a routing-plane capacity cost, not
an end-to-end inference cost** — important when a gateway must carry thousands of
agents, but not a claim that users see a 90% penalty.

## Headline 3 — two knees, and only one of them matters

The open-loop generator's payoff. Praxis, Poisson arrivals:

| Offered rps | Achieved | p50 ms | **p90 ms** | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 1,000 | 1,041 | 1.80 | **2.02** | 2.67 | 0 |
| 2,000 | 2,041 | 1.92 | **194.21** | 312.14 | 0 |
| 8,000 | 8,030 | 2.15 | 81.25 | 206.41 | 0 |
| 16,000 | 16,029 | 2.44 | 33.44 | 200.31 | 26,036 |

**Latency knee 1,000 → 2,000 rps: p90 explodes 96× while p50 moves 6%.** The
service still absorbs the offered rate, so throughput looks perfect; the damage is
entirely in the tail, where requests queue behind cache misses. A closed-loop
client slows down exactly when this happens and cannot see it.

llm-d's tail collapses earlier — knee at **500 → 1,000 rps** (p90 3.16 → 108.63 ms)
— so **Praxis holds its tail roughly 2× longer** under identical traffic.

**Operating guidance: size on the latency knee, not the throughput knee.** Quoting
8,000 rps for Praxis is defensible only if p90 ≈ 80 ms is acceptable; if the SLO is
single-digit milliseconds the real limit is ~1,000 rps.

## Praxis + llm-d-sc

### Praxis + llm-d-sc — open-loop arrival rate (Poisson)

| offered rps | achieved | p50 ms | p90 ms | p99 ms | p99.9 ms | mean ms | stddev | errors |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 250 | 291 | 1.82 | 2.01 | 2.74 | 3.77 | 1.86 | 0.22 | 0 |
| 500 | 541 | 1.79 | 1.98 | 2.65 | 3.49 | 1.83 | 0.22 | 0 |
| 1,000 | 1,041 | 1.80 | 2.02 | 2.67 | 3.66 | 1.84 | 1.42 | 0 |
| 2,000 | 2,041 | 1.92 | 194.21 | 312.14 | 392.53 | 36.79 | 81.01 | 0 |
| 4,000 | 4,031 | 2.04 | 80.25 | 215.89 | 317.10 | 21.07 | 45.86 | 0 |
| 8,000 | 8,030 | 2.15 | 81.25 | 206.41 | 311.97 | 18.66 | 46.70 | 0 |
| 16,000 | 16,029 | 2.44 | 33.44 | 200.31 | 315.29 | 15.87 | 40.77 | 26,036 |

**Latency knee: 1,000 → 2,000 rps** — p90 2.02 → 194.21 ms (**96×**) while p50 barely moves. This is the operating limit.

Throughput knee: absorbs 8,000 rps error-free — well past the latency knee, and misleading on its own.


### Praxis + llm-d-sc — classification cost (paired A/B, the only controlled comparison)

| concurrency | classified rps | control rps | cost | classified p99 | control p99 |
|--:|--:|--:|--:|--:|--:|
| 32 | 2,623 | 17,082 | **-84.6%** | 163.87 ms | 2.71 ms |
| 128 | 6,367 | 48,239 | **-86.8%** | 187.55 ms | 4.52 ms |
| 512 | 39,443 | 72,119 | **-45.3%** | 227.15 ms | 10.88 ms |

### Praxis + llm-d-sc — cache configuration × traffic shape

| traffic shape | exact rps | 95% CI | redis-semantic rps | Δ | exact p50 | exact p99 |
|---|--:|:--|--:|--:|--:|--:|
| unique | 404 | 354–610 | 302 | -25.2% | 67.78 | 188.40 |
| uniform | 376 | 372–393 | 309 | -17.7% | 76.41 | 193.44 |
| zipf | 1,334 | 1,270–1,373 | 1,183 | -11.3% | 13.67 | 140.45 |
| hotset | 734 | 499–817 | 651 | -11.2% | 29.28 | 159.37 |

### Praxis + llm-d-sc — context size

| context bytes | rps | p50 ms | p90 ms | p99 ms | mean ms |
|--:|--:|--:|--:|--:|--:|
| 64 | 349 | 190.35 | 311.79 | 358.37 | 183.71 |
| 256 | 663 | 39.35 | 239.17 | 288.21 | 96.90 |
| 1,024 | 952 | 17.12 | 203.32 | 278.69 | 67.37 |
| 4,096 | 911 | 19.66 | 207.38 | 279.27 | 70.01 |
| 16,384 | 968 | 15.84 | 198.89 | 274.43 | 66.14 |
| 65,536 | 981 | 17.90 | 197.76 | 275.66 | 65.42 |

## llm-d IPP + llm-d-sc

### llm-d IPP + llm-d-sc — open-loop arrival rate (Poisson)

| offered rps | achieved | p50 ms | p90 ms | p99 ms | p99.9 ms | mean ms | stddev | errors |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 250 | 291 | 2.11 | 19.80 | 146.09 | 155.41 | 11.38 | 29.69 | 0 |
| 500 | 541 | 2.07 | 3.16 | 150.56 | 167.44 | 8.08 | 24.99 | 0 |
| 1,000 | 1,038 | 2.17 | 108.63 | 215.66 | 292.32 | 28.51 | 51.90 | 0 |
| 2,000 | 2,041 | 2.13 | 188.37 | 291.58 | 354.30 | 36.97 | 78.99 | 0 |
| 4,000 | 4,030 | 2.25 | 56.38 | 190.51 | 283.27 | 16.70 | 38.91 | 0 |
| 8,000 | 8,030 | 2.39 | 78.93 | 201.82 | 280.44 | 18.63 | 45.56 | 0 |
| 16,000 | 16,029 | 2.65 | 41.40 | 177.93 | 267.85 | 14.95 | 38.49 | 0 |

**Latency knee: 500 → 1,000 rps** — p90 3.16 → 108.63 ms (**34×**) while p50 barely moves. This is the operating limit.

Throughput knee: absorbs 16,000 rps error-free — well past the latency knee, and misleading on its own.


### llm-d IPP + llm-d-sc — classification cost (paired A/B, the only controlled comparison)

| concurrency | classified rps | control rps | cost | classified p99 | control p99 |
|--:|--:|--:|--:|--:|--:|
| 32 | 3,163 | 17,692 | **-82.1%** | 130.77 ms | 2.80 ms |
| 128 | 2,095 | 49,713 | **-95.8%** | 215.43 ms | 4.46 ms |
| 512 | 45,413 | 79,948 | **-43.2%** | 186.15 ms | 11.08 ms |

### llm-d IPP + llm-d-sc — cache configuration × traffic shape

| traffic shape | exact rps | 95% CI | redis-semantic rps | Δ | exact p50 | exact p99 |
|---|--:|:--|--:|--:|--:|--:|
| unique | 304 | 281–599 | 301 | -0.9% | 94.18 | 245.13 |
| uniform | 310 | 299–328 | 309 | -0.5% | 91.51 | 245.30 |
| zipf | 1,178 | 931–1,220 | 1,183 | +0.5% | 14.97 | 171.39 |
| hotset | 648 | 581–760 | 652 | +0.7% | 32.31 | 194.87 |

### llm-d IPP + llm-d-sc — context size

| context bytes | rps | p50 ms | p90 ms | p99 ms | mean ms |
|--:|--:|--:|--:|--:|--:|
| 64 | 386 | 173.06 | 268.74 | 308.63 | 166.27 |
| 256 | 687 | 27.19 | 242.44 | 297.43 | 93.44 |
| 1,024 | 974 | 17.03 | 202.54 | 282.95 | 65.87 |
| 4,096 | 542 | 138.19 | 247.71 | 291.04 | 117.71 |
| 16,384 | 753 | 22.33 | 229.85 | 291.30 | 85.00 |
| 65,536 | 979 | 17.20 | 201.43 | 283.05 | 65.57 |

## vLLM SR + llm-d-sc

### vLLM SR + llm-d-sc — open-loop arrival rate (Poisson)

| offered rps | achieved | p50 ms | p90 ms | p99 ms | p99.9 ms | mean ms | stddev | errors |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 250 | 291 | 0.25 | 0.35 | 116.36 | 120.70 | 4.47 | 20.34 | 0 |
| 500 | 541 | 0.26 | 0.36 | 115.09 | 124.59 | 2.55 | 15.07 | 0 |
| 1,000 | 1,041 | 0.26 | 0.34 | 116.65 | 124.04 | 2.45 | 15.13 | 0 |
| 2,000 | 1,901 | 0.32 | 153.67 | 251.70 | 306.69 | 33.67 | 68.15 | 9,806 |
| 4,000 | 3,024 | 0.42 | 136.00 | 227.26 | 290.70 | 32.22 | 59.77 | 77,869 |
| 8,000 | 4,919 | 0.50 | 85.09 | 176.68 | 263.18 | 20.69 | 42.58 | 246,571 |
| 16,000 | 4,415 | 0.63 | 54.68 | 175.67 | 267.60 | 15.57 | 38.02 | 839,063 |

**Latency knee: 1,000 → 2,000 rps** — p90 0.34 → 153.67 ms (**449×**) while p50 barely moves. This is the operating limit.

Throughput knee: absorbs 1,000 rps error-free — well past the latency knee, and misleading on its own.


### vLLM SR + llm-d-sc — cache configuration × traffic shape

| traffic shape | exact rps | 95% CI | redis-semantic rps | Δ | exact p50 | exact p99 |
|---|--:|:--|--:|--:|--:|--:|
| unique | 302 | 280–595 | 301 | -0.5% | 95.17 | 246.93 |
| uniform | 309 | 298–331 | 310 | +0.2% | 92.27 | 246.34 |
| zipf | 1,183 | 943–1,228 | 1,178 | -0.5% | 15.12 | 171.49 |
| hotset | 651 | 585–770 | 655 | +0.6% | 32.24 | 195.16 |

### vLLM SR + llm-d-sc — context size

| context bytes | rps | p50 ms | p90 ms | p99 ms | mean ms |
|--:|--:|--:|--:|--:|--:|
| 64 | 387 | 172.51 | 269.78 | 307.24 | 165.69 |
| 256 | 697 | 25.23 | 241.67 | 296.91 | 92.18 |
| 1,024 | 974 | 18.77 | 203.08 | 287.88 | 65.91 |
| 4,096 | 449 | 155.47 | 257.02 | 297.26 | 142.01 |
| 16,384 | 697 | 25.44 | 241.36 | 298.77 | 92.17 |
| 65,536 | 974 | 18.43 | 203.63 | 289.55 | 65.87 |

### Praxis — route-table size

| routes | rps | 95% CI | p50 ms | p99 ms |
|--:|--:|:--|--:|--:|
| 2 | 954 | 759–1,011 | 56.63 | 222.15 |
| 4 | 935 | 754–991 | 58.25 | 225.46 |
| 8 | 937 | 758–1,001 | 57.80 | 225.73 |
| 16 | 936 | 756–1,005 | 57.55 | 226.55 |
| 32 | 927 | 753–985 | 58.26 | 227.74 |

<!-- generated from 261 runs / 85 arms -->

---

## Still open

* **Route correctness across 32 routes is untested.** Route-table *cardinality* is
  free (927–954 rps across 2→32), but every cluster pointed at the same backend,
  so this shows cost, not correctness. Needs uniquely tagged stubs and a confusion
  matrix.
* **CPU parity between gateways** is still uncontrolled (Praxis 16 req/32 limit;
  Envoy 4/12 + IPP 8/16).
* **Accuracy** of semantic-cache hits at threshold 0.90, and of the vLLM SR
  softmax transform, remain unmeasured.
* **Network impairment, pod loss and rolling updates** under sustained open-loop
  load were not run.
* **The Rayon result is directional, not a mapped optimum.** An earlier *isolated,
  single-run* diagnostic observed **6.65×** versus the unset default; the
  replicated round-3 comparison confirms the direction (W16/RT1 ≈ 248 req/s vs
  W16/RT4 ≈ 103 req/s, 3 reps each) but the full W × RT surface did not complete
  before the cluster window closed. Quote the direction, not the 6.65×.
