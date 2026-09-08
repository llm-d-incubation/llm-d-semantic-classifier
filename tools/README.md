# train-classifier

Contrastive finetuning of a sentence-transformer on classifier anchors. Takes a
classifier definition (JSON with labels and anchor texts), finetunes an embedding
model so same-class texts cluster together, and optionally evaluates the result
against a held-out dataset.

The finetuned model drops into the existing Rust runtime with no code changes.

## Quick start

```bash
# 1. Set up the Python environment (once)
python3 -m venv sc
source sc/bin/activate
pip install -r tools/requirements.txt

# 2. Train and evaluate
tools/train-classifier \
  --classifier classifiers/space-ops.json \
  --dataset evals/datasets/space-ops-heldout.jsonl
```

This produces a finetuned model in `artifacts/models/space-ops-ft/` and an
evaluation report at `artifacts/models/space-ops-ft/eval-report.json`.

## Parameters

| Flag | Default | Description |
|---|---|---|
| `--classifier` | *(required)* | Path to classifier definition JSON |
| `--dataset` | *(none)* | Held-out JSONL for evaluation (triggers eval after training) |
| `--output` | `artifacts/models/<id>-ft` | Where to save the finetuned model |
| `--labels` | all labels | Comma-separated subset of labels to train on |
| `--base-model` | `all-MiniLM-L6-v2` | HuggingFace model to start from |
| `--epochs` | `10` | Training epochs |
| `--batch-size` | `16` | Batch size |
| `--warmup-ratio` | `0.1` | Fraction of steps for learning rate warmup |

## How it works

1. Reads anchor texts from the classifier JSON (10–20 example prompts per label).
2. Generates same-class pairs from all pairwise combinations of anchors within
   each label. With 10 anchors per label, this yields 90 pairs per label.
3. Trains with **MultipleNegativesRankingLoss** — every other pair in the batch
   acts as an implicit negative, so no manually constructed negative pairs are
   needed.
4. Saves the model in ModelCar format (safetensors + tokenizer + config).
5. If `--dataset` is provided, runs the Rust `eval-classifier` binary for
   production-parity evaluation, then enriches the report with per-label metrics
   and a suitability verdict.

Training typically takes 10–40 seconds on CPU (Apple Silicon) depending on the
number of anchors.

## Label filtering

Use `--labels` to train on a subset of classes — useful when you want to sharpen
a specific boundary without affecting others:

```bash
tools/train-classifier \
  --classifier classifiers/complexity.json \
  --labels MEDIUM,COMPLEX \
  --dataset evals/datasets/complexity-heldout.jsonl
```

Evaluation always runs against **all** labels, so regressions on unselected
classes are visible in the report.

## Evaluation and the suitability verdict

When `--dataset` is provided, the tool produces an `eval-report.json` with
accuracy, macro F1, per-label precision/recall/F1, confusion matrix, latency,
and a **suitability** verdict:

| Verdict | Criteria | What it means |
|---|---|---|
| **STRONG** | macro F1 >= 0.85 *and* every class F1 >= 0.75 | Ready for production use |
| **GOOD** | macro F1 >= 0.70 *and* every class F1 >= 0.50 | Usable but has weak spots — review per-label metrics |
| **WEAK** | anything below | Not ready — some classes are unreliable |

### Reading the report

Start with the **suitability** verdict, then look at per-label F1 to find the
weak spots:

- **Low recall** on a label means prompts of that type are being misclassified
  as something else — the model misses them.
- **Low precision** on a label means other prompts are being pulled into it —
  the label is a "magnet."
- The **confusion matrix** shows exactly which labels are being confused.

### When is a model good enough?

There is no universal threshold — it depends on the cost of misrouting:

- **Routing to different-cost models** (e.g., simple vs reasoning): a GOOD
  verdict is often sufficient. A misroute to a more capable model wastes money
  but still produces a correct answer.
- **Security-sensitive classification** (e.g., sensitivity tiers): aim for
  STRONG. A misroute could expose data to the wrong tier.
- **Domain classification** (e.g., topic routing): GOOD is usually fine.
  Semantically distinct domains score well with minimal finetuning.

If the verdict is WEAK, check:

1. **Are the anchors diverse enough?** Anchors skewed to one domain (e.g., all
   software/CS) will fail on prompts from other domains.
2. **Are the labels structurally distinct?** Labels that share vocabulary and
   differ only by intent (e.g., "read data" vs "reason about data") are harder
   to separate than distinct topics.
3. **Do you need more anchors?** More anchors per label means more training pairs
   and better coverage of the label's semantic space.

## Held-out dataset format

One JSON object per line:

```jsonl
{"text": "What is the capital of France?", "tier": "SIMPLE"}
{"text": "Is a tomato a fruit or a vegetable?", "tier": "SIMPLE", "hard": true}
```

| Field | Required | Description |
|---|---|---|
| `text` | yes | The prompt to classify |
| `tier` | yes | Ground truth label (must match a label in the classifier JSON) |
| `hard` | no | `true` for boundary/ambiguous cases (reported separately) |

The dataset should be **authored independently of the anchors** — if you tune
anchors based on eval failures, the eval set becomes in-distribution and the
numbers stop meaning anything.

## Classifier definition format

```json
{
  "classifier_id": "my-classifier",
  "signal": "domain",
  "taxonomy_revision": "v1",
  "model_repo": "sentence-transformers/all-MiniLM-L6-v2",
  "method": "anchor-topk-mean",
  "top_k": 3,
  "labels": ["LABEL_A", "LABEL_B", "LABEL_C"],
  "anchors": {
    "LABEL_A": ["example prompt 1", "example prompt 2", "..."],
    "LABEL_B": ["..."],
    "LABEL_C": ["..."]
  }
}
```

Aim for 10+ anchors per label. Anchors should be representative of the label's
semantic space — vary the phrasing, domain, and structure.

## Creating a new classifier

1. **Define the taxonomy**: create a JSON file in `classifiers/` with your labels
   and 10+ anchors per label.
2. **Create a held-out dataset**: write 15+ prompts per label in JSONL format.
   These must be different from your anchors.
3. **Train**: `tools/train-classifier --classifier classifiers/yours.json --dataset evals/datasets/yours.jsonl`
4. **Review**: check the suitability verdict and per-label metrics. Iterate on
   anchors if needed.
5. **Deploy**: point the Rust server at the finetuned model with
   `LLM_D_SC_MODEL_DIR=artifacts/models/yours-ft`.
