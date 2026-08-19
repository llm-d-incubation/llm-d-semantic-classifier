# Classifiers, taxonomies, and anchors

llm-d-sc classifies text into ranked semantic signals. This document explains how a
taxonomy is defined, why it is data rather than code, and how to supply your own.

## The mechanism

A classifier is a fine-tuned sentence-embedding model plus a **classifier definition**:
a JSON document naming the labels and giving a handful of example texts (**anchors**)
per label.

At load time the resident model embeds every anchor once. At request time it embeds the
input and scores each label by the **mean cosine similarity of that label's top-k
nearest anchors** (k defaults to 3). The labels are returned ranked, highest first.

```
input text ──embed──► vector ──cosine──► anchors per label ──top-k mean──► ranked labels
```

There is no classification head. That is deliberate:

- **The taxonomy is replaceable without retraining.** Adding a label, or changing what a
  label means, is an edit to a JSON file. A softmax head would require a new training run
  and a new artifact for every taxonomy change.
- **The taxonomy is versioned data.** `taxonomy_revision` participates in the cache key
  alongside the model and tokenizer revisions, so changing anchors can never serve a stale
  classification from before the change.
- **A label is a region, not a point.** Averaging a label's anchors into one centroid
  discards the spread of a legitimately broad label. Top-k mean keeps the region while
  staying robust to one unrepresentative anchor.

## Built-in classifiers

Three definitions are compiled into the binary, so every instance can classify with no
external file and no network fetch.

| Classifier | Labels | Use |
|---|---|---|
| `complexity` (default) | `SIMPLE`, `MEDIUM`, `COMPLEX`, `REASONING` | Pick a serving tier: a lookup does not need a frontier model |
| `cost` | `MINIMAL`, `LOW`, `MODERATE`, `HIGH` | Expected serving cost of answering |
| `sensitivity` | `PUBLIC`, `INTERNAL`, `CONFIDENTIAL`, `REGULATED`, `NEVER_EGRESS` | Whether a prompt may leave the boundary |

Each pairs with its own model. Fetch and run:

```
./hack/fetch-model --classifier complexity
./target/release/llm-d-sc-classify "Design a multi region ledger with idempotent writes."

./hack/fetch-model --classifier sensitivity
./target/release/llm-d-sc-classify --classifier sensitivity "Here is our production API key."
```

In the server, select one with `LLM_D_SC_CLASSIFIER` (default `complexity`):

```
LLM_D_SC_CLASSIFIER=sensitivity LLM_D_SC_MODEL_DIR=/models llm-d-sc-server
```

`sensitivity` is a routing signal, not a security control. It is a similarity ranking with a
measured error rate, and must not be the only barrier preventing secret exfiltration.

## Custom taxonomies

`LLM_D_SC_CLASSIFIER` also accepts a path. The definition format:

```json
{
  "classifier_id": "support-desk",
  "signal": "domain",
  "taxonomy_revision": "support-desk-v1",
  "model_repo": "cnuland/llm-d-sc-complexity",
  "model_revision": "c5f55ef419d268ba843c544dc00988d1e9878044",
  "top_k": 3,
  "labels": ["BILLING", "OUTAGE", "HOWTO"],
  "anchors": {
    "BILLING": ["I was charged twice", "refund my last invoice", "update my billing address"],
    "OUTAGE":  ["the site is down", "we see 500 errors in production", "the API is unreachable"],
    "HOWTO":   ["how do I rotate my key", "where do I set the timeout", "how do I add a user"]
  }
}
```

A definition is validated at load: every declared label must have at least one anchor, and
every anchor group must be a declared label. A taxonomy that cannot rank one of its own
labels fails at startup rather than silently returning a partial ranking under load.

### Authoring anchors

- **Five to twelve anchors per label.** Below about five the top-k mean degenerates toward a
  single nearest neighbour; far more than twelve mostly adds load time.
- **Write anchors the way users write, not the way documentation writes.** Anchors are matched
  against real prompts, so phrase them as prompts.
- **Cover the spread of the label, not its centre.** Three near-identical anchors describe a
  point; the model already generalises around a point.
- **Check the boundaries.** The useful test is not whether an obvious case lands correctly, it
  is whether a case that could plausibly go either way lands where your policy says it should.

Balanced label coverage matters more than volume. An under-anchored label loses ties to a
well-anchored neighbour, which reads like a model failure but is a data failure.

### Does a custom taxonomy need a fine-tuned model?

Not necessarily. Anchors on a general-purpose embedder work when labels are topically distinct
(billing vs outage vs how-to). Fine-tuning matters when labels are distinguished by something
the general embedding space does not encode: on held-out data, complexity ranking scores 0.625
on the base model and 0.975 on the fine-tuned one, because generic embeddings encode topic, not
task difficulty.

Custom-domain support beyond these three built-ins is planned for a later version; the mechanism
above already works and is exercised by `I-074`.

## Measured accuracy

See `docs/benchmarks/` for the held-out evaluation of each built-in classifier, including the
methodology and the baseline comparison. All figures were produced on a single homelab machine
and need independent reproduction.
