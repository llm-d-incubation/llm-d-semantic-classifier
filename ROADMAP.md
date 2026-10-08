# Roadmap

Where `llm-d-sc` goes next, and why in that order.

This document is a proposal for discussion, not a commitment. Dates are
deliberately absent: the sequence is argued from dependency and evidence, and
phases land when their exit criteria are met.

It complements [`docs/known-gaps.md`](docs/known-gaps.md), which stays the
authoritative list of *current limitations*. Known gaps answers "what is missing
today"; this answers "what we do about it, in what order, and how we will know
it worked." Where a gap already carries a phase number, the mapping is called
out so the two documents do not drift.

## The argument

The v0.2 benchmark campaign
([data](https://github.com/cnuland/llm-d-sc-v0.2-benchmarking)) changed what the
most urgent work is. Three results drive the ordering below.

**1. Saturation is invisible, and it looks like a speed-up.** Above the
classifier's capacity the service does not queue and does not error. It fails
open: requests bypass classification and take the fast default path. At 4,000
req/s offered, classification coverage is **34.4%** — 65.6% of traffic reaches a
backend unclassified — with **zero errors at every rate** and a p50 that
*improves* from 564 ms to 2.03 ms. Throughput tracks the offered rate exactly.
Every signal an operator normally watches reports a system that is healthy and
getting better.

**2. Capacity is a hard ceiling, and the forward is the floor.** On three
replicas, coverage holds above 99% to **~1,200 req/s**, then capacity pins at
**~1,390 classifications/sec** from 1,600 req/s upward regardless of offered
load. Per-replica ceiling is **~463/s**, unchanged across v0.2 — the model
forward is the floor. v0.2 did not make the classifier faster; it made capacity
reachable, lifting max deployable capacity from ~480/s to ~1,390/s by fixing
horizontal scaling.

**3. Approximate caching on the input does not work for this task.** The L0 text
prefilter was built specifically to key on text and consult before the forward.
Measured against the full model on 6,000 real prompts, error rate stayed flat at
7.6–10.2% from J≥0.50 to J≥0.80 while hit rate collapsed from 50% to 4.6%:
**text similarity and label agreement are near-independent.** The L2 semantic
tier is separately mis-placed — its key is the forward's output, so a hit saves
microseconds and costs a network round trip. Exact caching remains the only
caching that pays.

Taken together: **make saturation visible and act on it automatically, then
reduce forwards per decision, then widen what the classifier can decide, then
deepen how gateways use it.** Because the forward is the floor and approximate
caching is refuted, throughput now comes from exactly two places: making the
forward cheaper, or performing fewer of them.

| Phase | Theme | Question it answers |
| --- | --- | --- |
| 1 | Operability | Can you run this in production and know it is working? |
| 2 | Fewer forwards, cheaper forwards | Can you serve more traffic per replica? |
| 3 | Widen the decision | Can it answer questions beyond complexity? |
| 4 | Integration depth | Do gateways use it well? |

Two tracks run across all four: **evaluation integrity** and **project
hygiene**.

---

## Phase 1 — Operability

Nothing else is safe to deploy until an operator can see saturation. This phase
is a hard prerequisite for Phase 2's throughput claims being verifiable in
production rather than only on a bench.

| Work | Notes | Existing issue |
| --- | --- | --- |
| Metrics endpoint | `llm_d_sc_classify_total` already carries a `classified` dimension, so coverage is an unambiguous ratio. What is missing is the scrape surface: a Prometheus or OpenTelemetry endpoint exporting it alongside per-stage histograms, queue depth, admission rejections, cache hit ratio **by tier**, and classifier/model revision and digest as labels. | [#11](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/11) — currently phased 0.3; proposed to move first |
| Ship the coverage alert, not just the metric | Coverage is the only signal that detects saturation, so the alert rule ships with it: `sum(rate(llm_d_sc_classify_total{classified="true"}[5m])) / sum(rate(llm_d_sc_classify_total[5m]))`. Two traps to document: a **zero delta is an invalid read, not 0%**, and `LOW_CONFIDENCE` / `UNMAPPED_LABEL` / `ABSTAIN` are *successful* classifications the router declined to act on, so counting only `OK` under-reports. | new |
| Confirm linearity past two replicas | Two replicas measured 2.04x after the filter's pinned channel and a single-host `nodeSelector` were both fixed. The three-replica arm reads 2.09x but was demand-limited, and at n=1 per arm a 2% delta is not a signal. Needs one demand-sufficient arm at four or more replicas before autoscaling is built on an assumption of linearity. | new |
| Coverage-keyed autoscaling | Only after the above two. A default HPA on CPU or request rate **will never fire**: at 34% coverage the service looks healthy by every one of those signals. Scaling on coverage is what makes this different from stock autoscaling. | new |
| Per-request deadlines and cancellation | A queued request the caller has abandoned should not consume a forward. | [#12](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/12) |
| Graceful drain on shutdown | An autoscaler that removes pods needs defined drain semantics or it drops in-flight work. | [#13](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/13) |
| Health-checking endpoint | Readiness is internal state today; an orchestrator cannot probe it. | known-gaps, phase 0.3 |
| Behaviour under pod CPU limits | Every published number comes from an unconstrained host and will not transfer directly to a limited pod. | [#14](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/14) |

**Exit criteria**

- Coverage is exported, documented, and alertable, with invalid reads
  distinguishable from genuine zero.
- Classification throughput is measured at four or more replicas under
  demand-sufficient load and scales within 10% of linear, or the ceiling is
  explained and documented.
- A cluster adds replicas in response to coverage degradation, and drains them
  without dropping in-flight work.
- Published throughput figures exist for a CPU-limited pod.

---

## Phase 2 — Fewer forwards, cheaper forwards

The forward is ~31 ms against microseconds for ranking, and the per-replica
ceiling of ~463/s has not moved across v0.2. Two of the obvious levers are
already closed: the L0 text prefilter was built and measured as a failure, and
the L2 semantic tier is mis-placed by construction. **Caching on input
similarity is not the lever it appeared to be**, which leaves exactly two
routes — make the forward cheaper, or perform fewer of them.

| Work | Notes | Existing issue |
| --- | --- | --- |
| Batch concurrent misses through one forward | The clearest remaining multiplier on per-replica capacity, and it operates in exactly the regime that saturates. | [#16](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/16) |
| Quantisation and shape discipline | INT8 or dynamic quantisation, bucketed padding instead of per-request shapes, an explicit `max_seq_len` truncation policy, encoder attention kernels. The forward is 99.4% of request latency and ~463/s per replica is the floor it sets, so this is the only work that moves the per-replica number itself. | new |
| Session-scoped classification | Promoted here from integration work because it is now a **primary throughput lever**, not only a stability one. Complexity is largely a property of a conversation, not a turn. Classifying once per session with an exponentially weighted average across turns reduces forwards per conversation directly — and with input-similarity caching refuted, session reuse is the main remaining way to do fewer forwards. | new |
| Cache hits must not pass through admission | A hit costs 632 ns and can currently be rejected with `RESOURCE_EXHAUSTED`. | [#2](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/2) |
| FIFO to LRU eviction | Exact caching is the only caching that pays, so its hit rate is worth the recency bookkeeping. | [#9](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/9) |
| Auto-tune executor width | `RAYON_NUM_THREADS=1` measured 3.25x faster than 4 at the same worker count — intra-op parallelism competes with the worker pool for cores. That constant is hardware-specific and will be wrong elsewhere, so it should be discovered at startup rather than documented. | known-gaps, phase 0.2 |
| vLLM as an optional inference backend | Scoped honestly: for a 23M-parameter MiniLM, vLLM is unlikely to win on single-request latency and adds a hop. The plausible wins are continuous batching at high offered rates and operating one engine instead of two. Implement behind the existing backend seam and A/B against Candle at matched load; adopt only if it moves ~463/s per replica. | new |
| Recover the trained head's 12.3% | The head costs 12.3% of classification throughput against anchor cosine on saturated arms, and it is worth paying because anchor cosine scored below a constant classifier on 3 of 5 signals. But the cost is one extra matmul on a forward that dominates everything, so it should be recoverable by fusing it into the forward rather than by reverting the decision rule. | new |

**Not planned, and why.** Recording these so they are not proposed again:

- **L0 text prefilter.** Built, measured on 6,000 real prompts against the full
  model, rejected. Error rate is flat at 7.6–10.2% across J≥0.50 to J≥0.80
  while hit rate collapses 50% → 4.6%. Text similarity does not predict label
  agreement, so there is no threshold that is both safe and useful.
- **Re-keying the L2 semantic tier.** Follows from the same result: any
  pre-forward key available to us is a text key.

A caching proposal should only be reopened by a key that predicts the *label*
rather than the *text*.

**Exit criteria**

- Per-replica classification ceiling moves measurably above ~463/s, or the
  forward is shown to be irreducible on this hardware.
- Coverage holds above 99% beyond ~1,200 req/s on three replicas.
- Forwards per conversation fall measurably under session-scoped
  classification, with routing stability no worse.

---

## Phase 3 — Widen the decision

| Work | Notes | Existing issue |
| --- | --- | --- |
| Sequence-classification runtime adapter | Hard dependency for everything else in this phase. The 0.1 backend ranks embeddings against anchors and cannot serve a sequence-classification model, which is why such artifacts are deliberately not offered by `hack/fetch-model`. | [#7](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/7) |
| **Domain routing with user-supplied anchors** | The design commitment worth making: **a user should not have to train a model to route their own domains.** Classification here is a thin layer over the embedding — encode the query and a set of labelled anchors, take the class with the highest top-k mean cosine. So accept roughly 20 examples per class as configuration, embed them at startup, and rank at request time. Ranking is microseconds, so the runtime cost is near zero. Offer contrastive fine-tuning as the *upgrade* path for taxonomies where zero-shot anchors are not separable — see Red Hat AI Innovation Team's [Embedding SFT](https://ai-innovation.team/training_hub/#/algorithms/embedding_sft) in `training_hub`, whose canonical use case is exactly semantic routing. | known-gaps, phase 0.4 |
| Candidate-aware classification and model-affinity scoring | Today the service answers "how complex is this prompt?" in isolation. Routing actually needs "which of *these* candidate models suits this prompt?", which is a different function with a different signature. This is probably the largest single product differentiator available. | [#18](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/18) |
| Multidimensional evaluation | Evaluate on a second axis — candidate model type — rather than label accuracy alone. Required to make the item above assessable; a model-affinity claim cannot be scored by a single-axis accuracy number. | new |
| Additional decision model families | Gate behind the adapter and a stable backend trait, and require each new family to land with a multidimensional eval row rather than a claim. | new |
| `ABSTAIN` on insufficient context | The project already publishes a `contested` split: roughly 26–30% of real prompts where three independent jurors do not agree. That is a measured map of where the taxonomy does not resolve. A principled abstention into fallback is better than a confident wrong route. | [#8](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/8) |

**Exit criteria**

- A user adds a domain taxonomy from configuration alone — no rebuild, no
  retraining — and gets a measured accuracy report for it.
- Candidate-aware scoring is evaluated on both axes and published.
- `ABSTAIN` rate on the `contested` split is materially higher than on the
  unanimous split.

---

## Phase 4 — Integration depth

| Work | Notes |
| --- | --- |
| Session identity and turn accounting at the gateway | The classification half of this lands in Phase 2 as a throughput lever. What remains here is the gateway contract: who owns session identity, how turns are counted across a conversation, and how a mid-session re-classification is signalled so routing does not flap. |
| Explicit fallback contract | Fail-open is currently emergent behaviour discovered in a benchmark. It should be a configurable, documented policy that emits a signal when it engages. |
| Infrastructure-aware routing | Accept queue-depth and KV-cache-pressure signals so tier selection and pod selection compose rather than compete. |
| Decision cache sharing | Gateway and classifier cache independently today. A shared decision cache across gateway replicas raises the effective hit rate, and the hit rate is the knee. |

**Exit criteria**

- Session-mode classification demonstrably reduces classifications per
  conversation and reduces route changes per conversation.
- Fail-open is configurable and observable.

---

## Cross-cutting: evaluation integrity

The evaluation has a measured ceiling, and it changes where accuracy work
should go.

Gold labels were audited by blind paired adjudication in two strata — the rows
the model got wrong *and* a sample of the rows it got right — with the judge
shown two candidate labels in random order and no indication of provenance.
**Roughly 4.9% of gold labels are themselves wrong**, so a perfect classifier
scored against this eval reaches about 0.95, not 1.0. Measured real-traffic
accuracy is 0.8963.

That leaves under six points of headroom, some of which is noise. **The
remaining accuracy is in label quality and taxonomy resolution, not in model
capacity.** Work accordingly:

- Improve gold label quality and re-publish the ceiling estimate alongside any
  accuracy figure, so the two are never read apart.
- Treat the ~26–30% non-unanimous fraction as a taxonomy problem. Either the
  tiers need sharper definitions or those prompts genuinely do not resolve, and
  `ABSTAIN` is the honest answer.
- Resolve the outstanding vLLM Semantic Router model arm before drawing
  conclusions from it. The existing P5 data shows 43.3% coverage at only 400
  req/s with p50 pinned at 1002–1003 ms and almost no variance, which is the
  signature of a hard one-second timeout rather than a capacity limit. Those
  numbers are currently unexplained, not a result.
- **Independent reproduction.** Every published figure comes from one campaign,
  on one cluster, run by one contributor. For a project whose differentiator is
  measurement rigour, reproduction by a second party is a credibility
  milestone worth scheduling rather than hoping for.

## Cross-cutting: project hygiene

Small, but each is an adoption blocker for someone.

| Work | Existing issue |
| --- | --- |
| Migrate default classifier artifacts out of a personal namespace into the `llm-d` organisation | [#6](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/6) |
| `fetch-model` must verify that present files match the pinned revision rather than skipping download | [#5](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/5) |
| Reconcile the cost classifier definition with `fetch-model`, and pin the mutable reference | [#1](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/1) |
| Refresh `known-gaps.md` now that cluster evidence exists | [#15](https://github.com/llm-d-incubation/llm-d-semantic-classifier/issues/15) |

---

## Open questions for discussion

1. **Does scaling stay linear past two replicas?** 1→2 measured 2.04x once the
   filter's pinned channel and the single-host `nodeSelector` were fixed. The
   three-replica arm was demand-limited, so linearity above two is assumed
   rather than measured. Autoscaling policy depends on the answer.
2. **How far should anchors be configurable before a ModelCar is required?**
   There is a real trade-off between "drop examples in a ConfigMap" ergonomics
   and reproducibility of a digest-pinned artifact.
3. **Does vLLM as a backend justify its operational cost at 23M parameters?**
   Worth deciding on measurement rather than on consistency with the rest of
   the stack.
4. **Should session-mode classification live here or in the gateway?** Keeping
   session state in the classifier is simpler for callers and worse for
   horizontal scale.
