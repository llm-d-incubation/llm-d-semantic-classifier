# Round 3 — statistical report (corrected data only)
Generated from 306 captured runs across 100 distinct arms, each replicated. Every figure is a median over independent repetitions with a bootstrap 95% CI. `p99.9` reads `n/a` where an arm has fewer than 1,000 measured requests, because it cannot be resolved from that sample.
**Traffic:** frozen 200,000-utterance corpus (seed `20260903`), 12 domains, 2 B–18 KB. **Config:** llm-d-sc at 16 workers / `RAYON_NUM_THREADS=1` / 16 CPU — Rayon pinned rather than left to track the CPU limit. **Arrival-rate arms are OPEN-LOOP** (Poisson), so offered rate is independent of response latency.
## Excluded arms
These were measured but are retired. They are excluded here so a corrected report cannot be read alongside a withdrawn number; the raw files remain in `data/json/` for anyone checking the correction.
| arm family | runs excluded | why |
|---|--:|---|
| `r3-*-rate*` | 63 | open-loop warmup carry-in inflated every rate by concurrency/window (1024/25 = 40.96 rps); superseded by the corrected r3b sweep |
| `r3-*-ctx*` | 54 | --context-bytes was ignored in corpus mode, so all six arms sent identical traffic; the sweep measured nothing |
## Knees (corrected sweep, r3b)


**praxis-miss — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 28.34 | 42.98 | 71.16 | 0 |
| 500 | 498 | 536.70 | 581.69 | 607.03 | 0 |
| 1,000 | 996 | 3.03 | 587.27 | 611.14 | 0 |
| 2,000 | 1,995 | 1.96 | 580.83 | 607.78 | 0 |
| 4,000 | 4,000 | 2.00 | 561.84 | 606.63 | 0 |
| 8,000 | 8,006 | 2.11 | 2.70 | 609.82 | 0 |

* **Latency knee: between 250 and 500 rps offered** — p90 goes 42.98 → 581.69 ms (14×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 8,000 rps error-free (8,006 achieved) — far past the latency knee, and misleading on its own.


**praxis-mix80 — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 250 | 1.91 | 2.08 | 2.92 | 0 |
| 500 | 504 | 1.88 | 2.04 | 2.40 | 0 |
| 1,000 | 1,001 | 1.89 | 2.06 | 2.45 | 0 |
| 2,000 | 2,006 | 1.94 | 2.13 | 2.87 | 0 |
| 4,000 | 4,003 | 2.07 | 62.15 | 99.66 | 0 |
| 8,000 | 8,001 | 2.26 | 86.12 | 121.24 | 0 |

* **Latency knee: between 2,000 and 4,000 rps offered** — p90 goes 2.13 → 62.15 ms (29×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 8,000 rps error-free (8,001 achieved) — far past the latency knee, and misleading on its own.


**praxis-hit — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 1.86 | 2.04 | 2.29 | 0 |
| 500 | 498 | 1.86 | 2.03 | 2.29 | 0 |
| 1,000 | 996 | 1.86 | 2.04 | 2.62 | 0 |
| 2,000 | 1,995 | 1.92 | 2.12 | 2.61 | 0 |
| 4,000 | 4,001 | 1.99 | 2.26 | 2.99 | 0 |
| 8,000 | 8,006 | 2.13 | 2.56 | 3.21 | 0 |
* Throughput knee: absorbs 8,000 rps error-free (8,006 achieved) — far past the latency knee, and misleading on its own.


**llmd-miss — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 28.08 | 44.47 | 63.96 | 0 |
| 500 | 498 | 546.10 | 586.08 | 606.97 | 0 |
| 1,000 | 996 | 2.99 | 588.48 | 610.65 | 0 |
| 2,000 | 1,995 | 2.13 | 580.31 | 609.09 | 0 |
| 4,000 | 4,001 | 2.16 | 563.83 | 604.43 | 0 |
| 8,000 | 8,006 | 2.24 | 2.94 | 610.37 | 0 |

* **Latency knee: between 250 and 500 rps offered** — p90 goes 44.47 → 586.08 ms (13×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 8,000 rps error-free (8,006 achieved) — far past the latency knee, and misleading on its own.


**llmd-mix80 — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 250 | 2.12 | 2.33 | 3.30 | 0 |
| 500 | 504 | 2.04 | 2.22 | 2.62 | 0 |
| 1,000 | 1,002 | 2.06 | 2.27 | 2.89 | 0 |
| 2,000 | 2,006 | 2.09 | 2.32 | 2.91 | 0 |
| 4,000 | 4,003 | 2.26 | 61.12 | 100.31 | 0 |
| 8,000 | 8,001 | 2.44 | 85.55 | 119.10 | 0 |

* **Latency knee: between 2,000 and 4,000 rps offered** — p90 goes 2.32 → 61.12 ms (26×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 8,000 rps error-free (8,001 achieved) — far past the latency knee, and misleading on its own.


**llmd-hit — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 2.03 | 2.25 | 2.68 | 0 |
| 500 | 498 | 2.00 | 2.19 | 2.58 | 0 |
| 1,000 | 996 | 2.01 | 2.24 | 2.86 | 0 |
| 2,000 | 1,995 | 2.05 | 2.32 | 3.04 | 0 |
| 4,000 | 4,001 | 2.15 | 2.49 | 3.20 | 0 |
| 8,000 | 8,006 | 2.28 | 2.75 | 3.47 | 0 |
* Throughput knee: absorbs 8,000 rps error-free (8,006 achieved) — far past the latency knee, and misleading on its own.


**vsr-miss — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 25.75 | 40.88 | 60.38 | 0 |
| 500 | 470 | 475.64 | 573.45 | 596.73 | 2,174 |
| 1,000 | 502 | 81.81 | 580.33 | 601.31 | 36,766 |
| 2,000 | 459 | 0.34 | 572.55 | 598.28 | 115,141 |
| 4,000 | 447 | 0.37 | 554.47 | 598.40 | 266,497 |
| 8,000 | 437 | 0.43 | 0.71 | 601.84 | 567,807 |

* **Latency knee: between 250 and 500 rps offered** — p90 goes 40.88 → 573.45 ms (14×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 250 rps error-free (248 achieved) — far past the latency knee, and misleading on its own.


**vsr-mix80 — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 250 | 0.28 | 0.37 | 0.47 | 0 |
| 500 | 504 | 0.29 | 0.39 | 0.57 | 0 |
| 1,000 | 1,002 | 0.31 | 0.42 | 0.54 | 0 |
| 2,000 | 2,006 | 0.35 | 0.48 | 0.61 | 0 |
| 4,000 | 3,886 | 0.42 | 60.97 | 99.93 | 11,810 |
| 8,000 | 4,487 | 0.51 | 82.94 | 117.68 | 255,103 |

* **Latency knee: between 2,000 and 4,000 rps offered** — p90 goes 0.48 → 60.97 ms (128×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 2,000 rps error-free (2,006 achieved) — far past the latency knee, and misleading on its own.


**vsr-hit — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 248 | 0.28 | 0.36 | 0.48 | 0 |
| 500 | 498 | 0.30 | 0.39 | 0.49 | 0 |
| 1,000 | 996 | 0.30 | 0.40 | 0.52 | 0 |
| 2,000 | 1,995 | 0.34 | 0.46 | 0.64 | 0 |
| 4,000 | 4,000 | 0.41 | 0.56 | 0.73 | 0 |
| 8,000 | 8,007 | 0.49 | 0.72 | 0.96 | 0 |
* Throughput knee: absorbs 8,000 rps error-free (8,007 achieved) — far past the latency knee, and misleading on its own.


## Praxis + llm-d-sc


### Praxis + llm-d-sc — cache: exact, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 734 | 499–817 | 29.28 | 124.45 | 142.21 | 159.37 | 179.13 | 210.60 | 43.63 | 41.17 | 0 | ok |
| uniform | 3 | 376 | 372–393 | 76.41 | 168.26 | 178.57 | 193.44 | 207.77 | 228.49 | 85.16 | 56.34 | 0 | ok |
| unique | 3 | 404 | 354–610 | 67.78 | 163.26 | 173.30 | 188.40 | 203.41 | 233.42 | 79.28 | 53.76 | 0 | ok |
| zipf | 3 | 1,334 | 1,270–1,373 | 13.67 | 55.81 | 120.53 | 140.45 | 157.68 | 197.15 | 23.98 | 31.62 | 0 | ok |


### Praxis + llm-d-sc — cache: redis-semantic, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 651 | 587–760 | 32.28 | 145.31 | 173.28 | 196.14 | 217.84 | 250.93 | 49.12 | 49.42 | 0 | ok |
| uniform | 3 | 309 | 298–329 | 92.01 | 213.00 | 225.80 | 249.58 | 268.77 | 305.11 | 103.45 | 73.57 | 0 | ok |
| unique | 3 | 302 | 278–1,268 | 94.84 | 214.13 | 226.64 | 246.76 | 267.71 | 341.58 | 105.79 | 73.23 | 0 | ok |
| zipf | 3 | 1,183 | 931–1,235 | 14.88 | 58.68 | 138.79 | 171.30 | 194.31 | 246.55 | 27.05 | 37.29 | 0 | ok |


### Praxis + llm-d-sc — route-table size

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| 02 routes | 3 | 954 | 759–1,011 | 56.63 | 123.11 | 185.16 | 222.15 | 259.56 | 310.98 | 67.03 | 47.37 | 0 | ok |
| 04 routes | 3 | 935 | 754–991 | 58.25 | 126.48 | 189.84 | 225.46 | 257.81 | 288.87 | 68.46 | 47.91 | 0 | ok |
| 08 routes | 3 | 937 | 758–1,001 | 57.80 | 126.01 | 188.95 | 225.73 | 278.65 | 754.88 | 68.16 | 48.75 | 0 | ok |
| 16 routes | 3 | 936 | 756–1,005 | 57.55 | 125.93 | 189.23 | 226.55 | 333.36 | 688.42 | 68.34 | 49.34 | 0 | ok |
| 32 routes | 3 | 927 | 753–985 | 58.26 | 127.84 | 191.23 | 227.74 | 286.14 | 738.62 | 68.92 | 49.12 | 0 | ok |


### Praxis + llm-d-sc — classification cost (paired A/B)

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| classified-c0032 | 3 | 2,623 | 2,485–2,991 | 2.49 | 27.33 | 47.12 | 163.87 | 186.12 | 332.20 | 12.22 | 27.28 | 0 | ok |
| classified-c0128 | 3 | 6,367 | 5,907–6,782 | 3.58 | 82.53 | 107.52 | 187.55 | 269.81 | 328.85 | 20.15 | 39.97 | 0 | ok |
| classified-c0512 | 3 | 39,443 | 38,424–39,459 | 5.89 | 15.28 | 26.41 | 227.15 | 356.27 | 599.25 | 12.96 | 35.54 | 0 | ok |
| control-c0032 | 3 | 17,082 | 16,969–17,295 | 1.89 | 2.21 | 2.40 | 2.71 | 3.85 | 376.32 | 1.87 | 3.17 | 0 | ok |
| control-c0128 | 3 | 48,239 | 48,108–48,280 | 2.59 | 3.60 | 3.91 | 4.52 | 8.12 | 24.20 | 2.65 | 0.79 | 0 | ok |
| control-c0512 | 3 | 72,119 | 71,364–72,148 | 7.03 | 8.87 | 9.47 | 10.88 | 16.17 | 168.10 | 7.10 | 1.70 | 0 | ok |


## llm-d IPP + llm-d-sc


### llm-d IPP + llm-d-sc — cache: exact, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 648 | 581–760 | 32.31 | 146.55 | 172.68 | 194.87 | 215.94 | 243.59 | 49.41 | 49.55 | 0 | ok |
| uniform | 3 | 310 | 299–328 | 91.51 | 212.62 | 225.84 | 245.30 | 266.51 | 285.19 | 103.21 | 73.33 | 0 | ok |
| unique | 3 | 304 | 281–599 | 94.18 | 213.25 | 225.60 | 245.13 | 271.65 | 291.62 | 105.27 | 72.88 | 0 | ok |
| zipf | 3 | 1,178 | 931–1,220 | 14.97 | 57.93 | 141.22 | 171.39 | 191.04 | 237.87 | 27.17 | 37.56 | 0 | ok |


### llm-d IPP + llm-d-sc — cache: redis-semantic, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 652 | 589–749 | 32.26 | 143.66 | 173.42 | 194.82 | 223.05 | 250.94 | 49.09 | 49.34 | 0 | ok |
| uniform | 3 | 309 | 297–332 | 91.87 | 213.52 | 226.31 | 245.88 | 270.01 | 298.84 | 103.60 | 73.69 | 0 | ok |
| unique | 3 | 301 | 280–1,255 | 95.31 | 215.66 | 228.81 | 249.40 | 274.00 | 295.91 | 106.40 | 73.93 | 0 | ok |
| zipf | 3 | 1,183 | 932–1,237 | 15.00 | 58.06 | 138.11 | 171.18 | 193.40 | 237.83 | 27.03 | 37.16 | 0 | ok |


### llm-d IPP + llm-d-sc — classification cost (paired A/B)

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| classified-c0032 | 3 | 3,163 | 2,655–3,500 | 2.54 | 20.72 | 37.58 | 130.77 | 150.45 | 201.97 | 10.11 | 21.82 | 0 | ok |
| classified-c0128 | 3 | 2,095 | 1,946–3,662 | 65.44 | 118.33 | 149.51 | 215.43 | 249.17 | 289.88 | 61.29 | 50.33 | 0 | ok |
| classified-c0512 | 3 | 45,413 | 44,923–46,479 | 5.29 | 11.26 | 15.92 | 186.15 | 287.73 | 433.44 | 11.30 | 30.03 | 0 | ok |
| control-c0032 | 3 | 17,692 | 17,564–17,703 | 1.89 | 2.26 | 2.45 | 2.80 | 3.93 | 17.91 | 1.81 | 0.45 | 0 | ok |
| control-c0128 | 3 | 49,713 | 49,294–50,915 | 2.49 | 3.54 | 3.83 | 4.46 | 7.14 | 23.38 | 2.57 | 0.75 | 0 | ok |
| control-c0512 | 3 | 79,948 | 79,733–79,967 | 6.31 | 8.70 | 9.43 | 11.08 | 14.73 | 36.48 | 6.41 | 1.82 | 0 | ok |


## vLLM SR adapter + llm-d-sc


### vLLM SR adapter + llm-d-sc — cache: exact, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 651 | 585–770 | 32.24 | 149.55 | 173.85 | 195.16 | 217.29 | 278.07 | 49.23 | 49.80 | 0 | ok |
| uniform | 3 | 309 | 298–331 | 92.27 | 213.60 | 226.48 | 246.34 | 263.93 | 292.14 | 103.42 | 74.08 | 0 | ok |
| unique | 3 | 302 | 280–595 | 95.17 | 214.17 | 227.30 | 246.93 | 272.30 | 304.97 | 105.71 | 73.88 | 32 | ok |
| zipf | 3 | 1,183 | 943–1,228 | 15.12 | 57.31 | 145.81 | 171.49 | 192.15 | 227.06 | 27.01 | 37.62 | 0 | ok |


### vLLM SR adapter + llm-d-sc — cache: redis-semantic, by traffic shape

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| hotset | 3 | 655 | 588–753 | 32.25 | 143.71 | 173.29 | 193.90 | 221.45 | 254.79 | 48.92 | 49.40 | 0 | ok |
| uniform | 3 | 310 | 299–330 | 92.25 | 212.92 | 225.71 | 245.07 | 267.51 | 292.44 | 103.29 | 73.60 | 0 | ok |
| unique | 3 | 301 | 279–1,265 | 95.57 | 215.83 | 228.56 | 247.84 | 273.80 | 289.78 | 106.34 | 74.43 | 32 | ok |
| zipf | 3 | 1,178 | 928–1,233 | 15.12 | 58.34 | 139.44 | 172.10 | 192.94 | 234.66 | 27.15 | 37.57 | 0 | ok |

