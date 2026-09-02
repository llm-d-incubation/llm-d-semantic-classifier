# 0.22 Slice 1 — Local Regression Benchmark

## Purpose

Check that adding the context-completeness branch does not regress ordinary
full-context classification. This is a local regression comparison, not a new
performance acceptance gate or a claim of attributable improvement.

## Method

- Before revision: `a17834b5c3beb4186e2c1c1d8eb757dce3ed5b85`.
- After revision: `ea6bd3a652e3b8e39e224e6ce937e498fba51ca5`.
- Backend: Candle, pinned sensitivity artifact revision
  `d82ff10d41fcf7d33f90e0597e6621bf1ff94ed4`.
- Host: Apple M4 Pro; loopback topology.
- Matrix: cache hit/miss × 32/64/128/256-token target × concurrency 1/4.
- Each scenario: 20 warmup and 100 measured requests.
- Before: four valid trials; after: three valid trials. Reported figures are
  per-scenario medians across trials.

## Result

| Workload | Before p99 range | After p99 range | Finding |
|---|---:|---:|---|
| Cache hit | 0.145–0.351 ms | 0.133–0.345 ms | no regression observed |
| Cache miss | 14.53–151.63 ms | 12.46–91.64 ms | no regression observed |

The after values are lower for most miss scenarios, but these sequential local
trials do not isolate CPU frequency, thermal state, or background load. They
must not be interpreted as an implementation-caused speedup.

`DELTA` requests are intentionally absent from this ordinary classification
matrix: U-048 proves their semantic performance property directly—zero exact
cache interactions and zero raw model forwards.
