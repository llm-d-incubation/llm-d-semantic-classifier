# Semantic Cache Classifier — Design

**Date:** 2026-08-30
**Status:** Approved design (pre-implementation)
**Related (existing, stubbed):** `specs/0.22-cache-session/spec.md`, `specs/0.24-runtime-pluggability/spec.md`

## 1. Summary

Add an **optional, pluggable semantic cache** to the classifier so that
semantically-similar prompts reuse a previously computed label without re-ranking.
Example: `"what is the capital of France"` → `simple`; a later
`"what is the capital of Japan"` is close in embedding space and returns `simple`
from the cache.

The semantic tier is a **best-effort optimization layer**, off by default, backed by
Redis (RediSearch vector index). It sits alongside — not in place of — the existing
in-memory exact (1:1) cache. Classification never fails because of the cache.

## 2. Goals / Non-goals

**Goals**
- Maximize **paraphrase hit-rate**: semantically equivalent prompts get the same label.
- **Persistence & sharing** of labels via Redis (survive restarts, shareable across future replicas).
- **Toggleable plugin**: off by default; enabled by config. Zero cost when off.
- **Fail-open**: Redis unavailable/slow ⇒ degrade to compute; never error the request.

**Non-goals (YAGNI at current scale)**
- Not a standalone cache microservice (documented as the future scale-out path).
- Not reducing embedding compute — the embedding (BERT forward) is unavoidable on a
  lookup and is explicitly *not* what this saves. This tier saves the (cheap) ranking
  step; its value is hit-rate, persistence, and label stability.
- No strong correctness guarantees on false-positive similarity hits (wrong labels are
  tolerable per the routing use case); bounded only by threshold + TTL.

## 3. Context (as-built)

- Rust crate `llm-d-sc`. Embedding-based nearest-anchor ranker (not a trained head).
  Every forward computes a **mean-pooled, L2-normalized BERT embedding**
  (`src/embedding.rs`) — exactly the vector a cosine KNN needs.
- Plugin seam already exists: everything is generic over the `ClassifierRuntime` trait
  (`src/classify.rs`).
- Current "1:1 cache": `ExactCache`/`SharedCache` (`src/cache.rs`) — in-memory
  `HashMap` keyed by a **BLAKE3** fingerprint over
  `classifier_id + model_rev + tokenizer_rev + taxonomy_rev + normalized_text`,
  FIFO eviction (cap 50k), single-flight coalescing. Lives in the `ServiceCore<R>`
  wrapper (`src/classify.rs`), so all backends inherit caching.
- No Redis, no vector store today — this is net-new.
- Config: env vars (live) + a TOML `Config` with a `KNOWN_BACKENDS` registry pattern
  (`src/config.rs`).

## 4. Chosen approach — Two-stage runtime seam + pluggable cache tier

The semantic lookup needs the embedding, but the exact cache keys on text *before*
embedding. So the runtime forward is split so the cache layer can interpose between
embed and rank.

### 4.1 Data flow

```
Classify(req)
  └─ ServiceCore::classify
       1. L1 exact  (BLAKE3 text key)            ── hit ─▶ return   (in-memory, fastest)
       2. embed(input)                            ── the one unavoidable BERT forward
       3. L2 semantic KNN(vector, filter=identity, τ)
                                                  ── hit ─▶ return stored label (skip rank)
       4. rank(embedding)                         ── miss path only
       5. write-back:  L1 sync,  L2 fire-and-forget (bounded channel → bg task)
```

Disciplines carried from the exact cache:
- **Cache-identity isolation:** L2 entries carry a TAG of
  `classifier_id + model_rev + tokenizer_rev + taxonomy_rev`; a KNN query filters on it.
  A revision bump can never serve a stale label (mirrors the BLAKE3 identity fields).
- **Embed once:** step 2 feeds both step 3 and step 4.
- **Writes never block:** step 5's Redis write goes through a bounded channel to a
  background task; the inference thread returns immediately. On a full channel, the
  write is dropped (best-effort).

### 4.2 Architectural pattern

Layered **two-tier cache** (L1 in-process exact / L2 shared semantic) behind a
**strategy interface**, inside the existing modular monolith. Not microservices, not
event-driven. Redis is a best-effort cache, never a source of truth ⇒ fail-open by
construction.

## 5. Components

| Component | Responsibility | Input → Output | Location |
|---|---|---|---|
| `ClassifierRuntime` (split) | Two composable steps | `embed(Input) → Embedding`; `rank(&Embedding, &Input) → Result` | `src/classify.rs` |
| `Embedding` | Value type: L2-normalized vector + revisions | — | `src/embedding.rs` |
| `SemanticCache` (new trait) | Pluggable L2 lookup/insert seam | `lookup(&Embedding) → Option<Hit>`; `insert(&Embedding, &Result)` | `src/cache.rs` |
| `NoopSemanticCache` | Default when off (zero-cost) | always miss | `src/cache.rs` |
| `RedisSemanticCache` | Vector KNN; fail-open; async write-back; circuit breaker | embedding → nearest label ≥ τ | new `src/cache/redis.rs` |
| `ServiceCore<R>` | Orchestrates L1→embed→L2→rank→write | Input → Result | `src/classify.rs` |
| `ExactCache`/`SharedCache` (L1) | Unchanged — exact hits + single-flight | text hash → Result | `src/cache.rs` |
| Cache config + registry | Select strategy, τ, Redis URL, TTL; validate | env/TOML → `CacheConfig` | `src/config.rs` |
| Metrics | L1/L2 hit-rate, KNN latency, degrade/breaker count, τ-reject | counters/histograms | `src/metrics.rs` |

## 6. Redis representation

- **Redis Stack / Redis 8+** (RediSearch vector index). MVP index type: **`FLAT`**,
  distance **COSINE** (embeddings are already L2-normalized) — exact KNN, simplest.
  `HNSW` is the documented scale lever if the entry set grows large.
- Per entry (hash): `embedding` (vector field), `ranked` (JSON of labels+scores),
  revision fields, `identity` (TAG), `created_at`, `hit_count`. Per-entry **TTL**.
- Query: `KNN 1` over the vector field, filtered by `identity` TAG; accept if
  best cosine similarity ≥ τ.
- Eviction: entry TTL + Redis `maxmemory` with `allkeys-lru`. Cache is disposable ⇒
  eviction always safe.

## 7. Configuration & toggle

Off by default. Extends the existing registry/validation pattern in `src/config.rs`.

| Setting | Env | Default |
|---|---|---|
| Strategy | `LLM_D_SC_CACHE` = `exact` \| `redis-semantic` | `exact` |
| Redis URL | `LLM_D_SC_REDIS_URL` | (required if `redis-semantic`) |
| Similarity threshold τ | `LLM_D_SC_CACHE_THRESHOLD` | `0.90` (tunable; hit-rate-favoring) |
| Entry TTL | `LLM_D_SC_CACHE_TTL` | `86400s` |
| Redis op timeout | `LLM_D_SC_CACHE_TIMEOUT_MS` | `50` |

Validation mirrors `KNOWN_BACKENDS`: unknown strategy rejected; `redis-semantic`
requires a URL.

## 8. Failure modes & pre-mortem

Only new SPOF is Redis, **contained to L2**: every Redis failure degrades to
"L1 + compute". Redis is a performance SPOF, not an availability SPOF.

| # | Failure | Mitigation |
|---|---|---|
| 1 | Redis slow/unreachable stalls inference thread | Tight per-op timeout + fail-open; **circuit breaker** skips L2 for a cooldown after N consecutive errors |
| 2 | False-positive semantic hit (wrong label) | τ threshold (per-taxonomy capable); tolerable per use case; TTL ages out bad entries |
| 3 | Unbounded growth | Per-entry TTL + `maxmemory`/`allkeys-lru` |
| 4 | Stale labels after model/taxonomy bump | Identity TAG filter — new revisions never match old entries; no manual flush |
| 5 | Write-back backpressure | Bounded channel; drop write on full (best-effort) |
| 6 | Poisoned/garbage vectors | Validate vector dim on read; skip+delete malformed, treat as miss |
| 7 | Index cold start | Warms naturally; no correctness impact |

Watch hardest: **#1** (circuit breaker is non-optional) and **#2** (monitor L2 hit-rate
vs. sampled label-correctness to tune τ).

## 9. Trade-off matrix

Recommended = **A: in-process two-tier behind a strategy trait.**
Alternative = **B: standalone semantic-cache microservice/sidecar.**

| Dimension | A (recommended) | B (standalone service) |
|---|---|---|
| Scalability | Great at moderate scale; L2 already shareable. Ceiling: KNN on inference thread. | Higher ceiling, unneeded now; extra hop eats savings. |
| Complexity | Low — reuses `ServiceCore` seam + existing embedding; one trait, one module. | High — new service, contract, deploy, monitoring. |
| Maintainability | High — one crate, one config surface, existing patterns. | Lower — cross-version drift. |
| Latency (hit) | L1 µs; L2 one local Redis RTT. | Extra service hop. |
| Fault tolerance | Fail-open to compute. | Same, more parts to fail. |
| Ops cost | One opt-in Redis dependency. | Redis + a service. |

**Verdict:** A wins at the stated scale. Because Redis is hidden behind the
`SemanticCache` trait, migrating to B later is an implementation swap, not a rewrite.

## 10. Testing strategy

- **Unit:** `NoopSemanticCache` always-miss; `SemanticCache` trait contract; config
  parsing/validation (unknown strategy, missing URL); τ threshold accept/reject;
  identity-TAG isolation across revisions.
- **Fail-open:** Redis unreachable / timeout / malformed entry ⇒ request still succeeds
  via compute; circuit breaker opens after N errors and short-circuits L2.
- **Integration (feature-gated / dockerized Redis Stack):** insert then paraphrase-hit;
  exact-repeat served by L1 not L2; revision bump ⇒ miss; TTL expiry ⇒ miss.
- **No regression when off:** default `exact` path behaves exactly as today (embed once,
  existing single-flight and FIFO intact).

## 11. Open items for the implementation plan

- Exact wire shape of the split `ClassifierRuntime` (associated `Embedding` type vs.
  concrete) and how `CandleClassifier`'s existing embed/rank internals are extracted.
- Redis client choice + connection pooling; where the (bounded, timed) KNN call runs
  relative to the inference pool.
- Circuit-breaker parameters (error count, cooldown).
- Whether τ/TTL become per-taxonomy now or stay global for MVP (default: global).
