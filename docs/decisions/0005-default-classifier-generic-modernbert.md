# ADR-0005: The default classifier is the generic mmBERT/ModernBERT intent model

Date: 2026-08-18
Status: Accepted

## Decision

`hack/fetch-model` and the documented default configuration pull the **generic
domain/intent classifier used by vLLM Semantic Router**:

```
llm-semantic-router/mmbert-intent-classifier-merged
pinned revision: bf8d3833707d1bb8f9237260c271ca0d5982462d
```

Custom or fine-tuned classifiers (including the sensitivity model used during
early development) are opt-in, selected explicitly by configuration. They are
never the default.

## What this model is

Verified from its `config.json`:

| Property | Value |
|---|---|
| `model_type` | `modernbert` |
| architecture | `ModernBertForSequenceClassification` |
| hidden size / layers | 768 / 22 |
| max position embeddings | 8192 |
| vocab | 256,000 (multilingual) |
| local attention window | 128 |
| labels | 14: biology, business, chemistry, computer science, economics, engineering, health, history, law, math, other, philosophy, physics, psychology |

It is an mmBERT checkpoint, which is the multilingual ModernBERT architecture, so
`model_type` is genuinely `modernbert`.

## Why this is the right default

1. **Ecosystem alignment.** A user who already runs vLLM Semantic Router should
   get comparable signals from llm-d-sc without sourcing a different model.
2. **It is a sequence classifier with a real label set.** The head emits 14
   named domain labels, so `llm-d-sc` can return meaningful labels immediately.
   This removes the largest honesty gap in 0.1, where the bundled fixture had an
   unverified taxonomy and ranking was demonstrated against synthetic prototypes.
3. **General before special.** A generic classifier covering the common cases is
   the correct default for a component other projects will adopt; bespoke models
   belong behind explicit configuration.

## Implementation consequences

This is not a configuration edit. It changes the runtime path:

- **A second adapter is required.** The current backend implements the
  SentenceTransformers shape (transformer, mean pooling, normalize, then cosine
  similarity against prototypes). This model is a **sequence classification**
  head: transformer, then a classifier head producing logits over 14 labels,
  with softmax for confidence. Both adapters must coexist because the
  `classifier.json` artifact contract already declares a strategy.
- **ModernBERT support exists.** `candle-transformers` 0.11 implements
  ModernBERT (rotary embeddings, alternating local/global attention), so no new
  model code is needed, only wiring plus a config parse.
- **The tokenizer differs** (256k multilingual vocab). Existing golden token-ID
  fixtures pin the old model and remain valid for it; new goldens are required
  for the default.
- **Parity fixtures must be regenerated** against the pinned revision, with the
  reference produced by the pinned Python stack as before.
- **Performance numbers become stale.** 22 layers at hidden size 768 is
  materially larger than the ~22.7M-parameter model measured in
  `docs/performance.md`; that document must be re-run and re-labelled, not
  edited to match a guess.

## Status

The sensitivity model stays supported as an opt-in custom classifier and keeps
its existing parity evidence. Nothing about the unverified-taxonomy caveat
changes for it; that caveat simply stops applying to the default path.
