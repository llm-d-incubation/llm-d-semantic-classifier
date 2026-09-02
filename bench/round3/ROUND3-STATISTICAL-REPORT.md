# Round 3 — statistical report
Generated from 128 captured runs across 41 distinct arms, each replicated. Every figure is a median over independent repetitions with a bootstrap 95% CI.
**Traffic:** frozen 200,000-utterance corpus (seed `20260903`), 12 domains, 2 B–18 KB. **Config:** llm-d-sc at 16 workers / `RAYON_NUM_THREADS=1` / 16 CPU — Rayon pinned rather than left to track the CPU limit. **Arrival-rate arms are OPEN-LOOP** (Poisson), so offered rate is independent of response latency.
## Knees


**praxis — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 291 | 1.82 | 2.01 | 2.74 | 0 |
| 500 | 541 | 1.79 | 1.98 | 2.65 | 0 |
| 1,000 | 1,041 | 1.80 | 2.02 | 2.67 | 0 |
| 2,000 | 2,041 | 1.92 | 194.21 | 312.14 | 0 |
| 4,000 | 4,031 | 2.04 | 80.25 | 215.89 | 0 |
| 8,000 | 8,030 | 2.15 | 81.25 | 206.41 | 0 |
| 16,000 | 16,029 | 2.44 | 33.44 | 200.31 | 26,036 |

* **Latency knee: between 1,000 and 2,000 rps offered** — p90 goes 2.02 → 194.21 ms (96×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 8,000 rps error-free (8,030 achieved) — far past the latency knee, and misleading on its own.


**llmd — open-loop arrival-rate sweep**

| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |
|--:|--:|--:|--:|--:|--:|
| 250 | 291 | 2.11 | 19.80 | 146.09 | 0 |
| 500 | 541 | 2.07 | 3.16 | 150.56 | 0 |
| 1,000 | 1,038 | 2.17 | 108.63 | 215.66 | 0 |
| 2,000 | 2,041 | 2.13 | 187.48 | 285.66 | 0 |

* **Latency knee: between 500 and 1,000 rps offered** — p90 goes 3.16 → 108.63 ms (34×) while p50 barely moves. This is the operating limit.
* Throughput knee: absorbs 2,000 rps error-free (2,041 achieved) — far past the latency knee, and misleading on its own.


## Praxis + llm-d-sc


### Praxis + llm-d-sc — open-loop arrival rate

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| 250 rps offered | 3 | 291 | 291–291 | 1.82 | 2.01 | 2.20 | 2.74 | 3.77 | 5.40 | 1.86 | 0.22 | 0 | ok |
| 500 rps offered | 3 | 541 | 541–541 | 1.79 | 1.98 | 2.17 | 2.65 | 3.49 | 5.90 | 1.83 | 0.22 | 0 | ok |
| 1000 rps offered | 3 | 1,041 | 1,041–1,041 | 1.80 | 2.02 | 2.22 | 2.67 | 3.66 | 144.02 | 1.84 | 1.42 | 0 | ok |
| 2000 rps offered | 3 | 2,041 | 2,031–2,041 | 1.92 | 194.21 | 229.57 | 312.14 | 392.53 | 434.96 | 36.79 | 81.01 | 0 | ok |
| 4000 rps offered | 3 | 4,031 | 4,030–4,041 | 2.04 | 80.25 | 119.83 | 215.89 | 317.10 | 396.26 | 21.07 | 45.86 | 0 | ok |
| 8000 rps offered | 3 | 8,030 | 8,030–8,030 | 2.15 | 81.25 | 140.55 | 206.41 | 311.97 | 417.64 | 18.66 | 46.70 | 0 | ok |
| 16000 rps offered | 3 | 16,029 | 14,988–16,030 | 2.44 | 33.44 | 120.43 | 200.31 | 315.29 | 432.67 | 15.87 | 40.77 | 26,036 | **FAILED** |


### Praxis + llm-d-sc — context size

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| 64 B | 3 | 349 | 348–399 | 190.35 | 311.79 | 331.32 | 358.37 | 392.16 | 414.77 | 183.71 | 99.87 | 0 | ok |
| 256 B | 3 | 663 | 657–715 | 39.35 | 239.17 | 263.08 | 288.21 | 311.86 | 342.29 | 96.90 | 94.85 | 0 | ok |
| 1024 B | 3 | 952 | 952–995 | 17.12 | 203.32 | 246.36 | 278.69 | 304.45 | 333.73 | 67.37 | 83.46 | 0 | ok |
| 4096 B | 3 | 911 | 833–972 | 19.66 | 207.38 | 246.43 | 279.27 | 307.30 | 337.45 | 70.01 | 84.34 | 0 | ok |
| 16384 B | 3 | 968 | 958–975 | 15.84 | 198.89 | 240.23 | 274.43 | 299.14 | 335.66 | 66.14 | 82.56 | 0 | ok |
| 65536 B | 3 | 981 | 973–993 | 17.90 | 197.76 | 240.76 | 275.66 | 305.23 | 344.40 | 65.42 | 81.88 | 0 | ok |


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


### llm-d IPP + llm-d-sc — open-loop arrival rate

| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | premises |
|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|
| 250 rps offered | 3 | 291 | 291–291 | 2.11 | 19.80 | 76.15 | 146.09 | 155.41 | 195.47 | 11.38 | 29.69 | 0 | ok |
| 500 rps offered | 3 | 541 | 541–541 | 2.07 | 3.16 | 38.25 | 150.56 | 167.44 | 215.00 | 8.08 | 24.99 | 0 | ok |
| 1000 rps offered | 3 | 1,038 | 1,036–1,039 | 2.17 | 108.63 | 153.14 | 215.66 | 292.32 | 326.45 | 28.51 | 51.90 | 0 | ok |
| 2000 rps offered | 2 | 2,041 | 2,041–2,041 | 2.13 | 187.48 | 221.18 | 285.66 | 356.44 | 408.18 | 36.84 | 77.94 | 0 | ok |


## vLLM SR adapter + llm-d-sc

