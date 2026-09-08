"""Contrastive finetuning and evaluation for anchor-based classifiers.

Two modes:
  Training (default):  finetune a sentence-transformer on classifier anchors.
  Post-eval (--post-eval):  enrich a Rust eval-classifier JSON report with
                            per-label metrics and a suitability verdict.
"""

import argparse
import itertools
import json
import os
import random
import sys
import time


MODELCAR_FILES = [
    "model.safetensors",
    "tokenizer.json",
    "config.json",
    "modules.json",
    "1_Pooling/config.json",
]

DEFAULT_BASE_MODEL = "sentence-transformers/all-MiniLM-L6-v2"


def parse_args():
    p = argparse.ArgumentParser(description="Train or evaluate an anchor-based classifier.")

    sub = p.add_subparsers(dest="mode")

    # --- training mode (default when no subcommand) ---
    train_p = sub.add_parser("train", help="Contrastive finetuning on classifier anchors")
    train_p.add_argument("--classifier", required=True, help="Path to classifier definition JSON")
    train_p.add_argument("--labels", default=None, help="Comma-separated subset of labels to train on")
    train_p.add_argument("--base-model", default=None, help=f"Base model (default: {DEFAULT_BASE_MODEL})")
    train_p.add_argument("--output", default=None, help="Output directory (default: artifacts/models/<id>-ft)")
    train_p.add_argument("--epochs", type=int, default=10)
    train_p.add_argument("--batch-size", type=int, default=16)
    train_p.add_argument("--warmup-ratio", type=float, default=0.1)

    # --- post-eval mode ---
    eval_p = sub.add_parser("post-eval", help="Enrich a Rust eval-classifier JSON report")
    eval_p.add_argument("--raw-report", required=True, help="Path to eval-classifier JSON output")
    eval_p.add_argument("--output", required=True, help="Path to write enriched report")
    eval_p.add_argument("--trained-labels", default=None, help="Comma-separated labels that were trained")

    args = p.parse_args()

    if args.mode is None:
        p.print_help()
        sys.exit(1)

    return args


def load_classifier(path):
    with open(path) as f:
        return json.load(f)


def validate_labels(requested, available):
    unknown = [l for l in requested if l not in available]
    if unknown:
        print(f"error: unknown labels: {unknown}", file=sys.stderr)
        print(f"available: {available}", file=sys.stderr)
        sys.exit(1)
    if len(requested) < 2:
        print("error: MNR loss requires at least 2 labels for in-batch negatives", file=sys.stderr)
        sys.exit(1)
    return requested


def generate_pairs(anchors, labels):
    from sentence_transformers import InputExample

    pairs = []
    for label in labels:
        texts = anchors[label]
        if len(texts) < 2:
            print(f"error: label '{label}' has {len(texts)} anchor(s), need at least 2", file=sys.stderr)
            sys.exit(1)
        for a, b in itertools.combinations(texts, 2):
            pairs.append(InputExample(texts=[a, b]))
            pairs.append(InputExample(texts=[b, a]))

    random.seed(42)
    random.shuffle(pairs)
    return pairs


def train(model_name, pairs, output_dir, epochs, batch_size, warmup_ratio):
    from sentence_transformers import SentenceTransformer
    from sentence_transformers.losses import MultipleNegativesRankingLoss
    from torch.utils.data import DataLoader

    model = SentenceTransformer(model_name)

    if len(pairs) < batch_size:
        batch_size = max(2, len(pairs))
        print(f"  adjusted batch size to {batch_size} (fewer pairs than requested)")

    loader = DataLoader(pairs, shuffle=True, batch_size=batch_size)
    loss = MultipleNegativesRankingLoss(model)
    warmup_steps = int(len(loader) * epochs * warmup_ratio)

    print(f"  batches/epoch: {len(loader)}")
    print(f"  warmup steps:  {warmup_steps}")
    print()

    t0 = time.time()
    model.fit(
        train_objectives=[(loader, loss)],
        epochs=epochs,
        warmup_steps=warmup_steps,
        output_path=output_dir,
        show_progress_bar=True,
    )
    elapsed = time.time() - t0
    print(f"\n  training time: {elapsed:.1f}s")


def verify_modelcar(output_dir):
    missing = [f for f in MODELCAR_FILES if not os.path.exists(os.path.join(output_dir, f))]
    return missing


def compute_per_label(labels, confusion):
    per_label = {}
    for i, label in enumerate(labels):
        tp = confusion[i][i]
        fp = sum(confusion[j][i] for j in range(len(labels))) - tp
        fn = sum(confusion[i][j] for j in range(len(labels))) - tp
        support = sum(confusion[i])

        precision = tp / (tp + fp) if (tp + fp) > 0 else 0.0
        recall = tp / (tp + fn) if (tp + fn) > 0 else 0.0
        f1 = 2 * precision * recall / (precision + recall) if (precision + recall) > 0 else 0.0

        per_label[label] = {
            "precision": round(precision, 4),
            "recall": round(recall, 4),
            "f1": round(f1, 4),
            "support": support,
        }
    return per_label


def compute_suitability(macro_f1, per_label):
    min_f1 = min(v["f1"] for v in per_label.values())
    if macro_f1 >= 0.85 and min_f1 >= 0.75:
        return "STRONG"
    if macro_f1 >= 0.70 and min_f1 >= 0.50:
        return "GOOD"
    return "WEAK"


def suitability_reason(verdict, macro_f1, per_label):
    min_label = min(per_label, key=lambda l: per_label[l]["f1"])
    min_f1 = per_label[min_label]["f1"]
    if verdict == "STRONG":
        return f"macro F1 {macro_f1:.3f} >= 0.85 and all classes >= 0.75 F1"
    if verdict == "GOOD":
        return f"macro F1 {macro_f1:.3f} >= 0.70 and no class below 0.50 F1 (min: {min_label} at {min_f1:.3f})"
    return f"macro F1 {macro_f1:.3f} < 0.70 or {min_label} F1 {min_f1:.3f} < 0.50"


def enrich_report(raw_report, trained_labels=None):
    labels = raw_report["labels"]
    confusion = raw_report["confusion"]
    macro_f1 = raw_report["macro_f1"]

    per_label = compute_per_label(labels, confusion)
    verdict = compute_suitability(macro_f1, per_label)

    report = dict(raw_report)
    report["per_label"] = per_label
    report["suitability"] = verdict
    if trained_labels:
        report["trained_labels"] = trained_labels
    return report


def print_results(report):
    labels = report["labels"]
    per_label = report["per_label"]
    n = report["n"]
    accuracy = report["accuracy"]
    macro_f1 = report["macro_f1"]
    correct = int(round(accuracy * n))

    print("\nlabel            precision  recall      f1   support")
    print("---------------------------------------------------")
    for label in labels:
        m = per_label[label]
        print(f"{label:<15} {m['precision']:>9.3f} {m['recall']:>7.3f} {m['f1']:>7.3f} {m['support']:>9}")

    print(f"\naccuracy         {accuracy:.4f}  ({correct}/{n})")
    print(f"macro f1         {macro_f1:.4f}")

    if "hard_case_accuracy" in report and report["hard_case_accuracy"] > 0:
        print(f"hard-case acc    {report['hard_case_accuracy']:.4f}")

    if "latency_p50_ms" in report:
        print(f"latency p50/p99  {report['latency_p50_ms']:.2f} ms / {report['latency_p99_ms']:.2f} ms")

    verdict = report["suitability"]
    reason = suitability_reason(verdict, macro_f1, per_label)
    print(f"\nsuitability:     {verdict}")
    print(f"  {reason}")

    if "trained_labels" in report:
        print(f"\ntrained labels:  {', '.join(report['trained_labels'])}")


def do_train(args):
    clf = load_classifier(args.classifier)
    classifier_id = clf["classifier_id"]
    all_labels = clf["labels"]
    anchors = clf["anchors"]

    if args.labels:
        selected = validate_labels([l.strip() for l in args.labels.split(",")], all_labels)
    else:
        selected = list(all_labels)

    base_model = args.base_model or clf.get("model_repo") or DEFAULT_BASE_MODEL
    output_dir = args.output or f"artifacts/models/{classifier_id}-ft"

    print("=== train-classifier ===\n")
    print(f"  classifier:      {args.classifier}")
    print(f"  base model:      {base_model}")
    if len(selected) < len(all_labels):
        print(f"  training labels: {', '.join(selected)} ({len(selected)} of {len(all_labels)})")
    else:
        print(f"  training labels: all ({len(selected)})")

    pairs = generate_pairs(anchors, selected)
    print(f"  training pairs:  {len(pairs)}")
    print(f"  epochs:          {args.epochs}")
    print(f"  batch size:      {args.batch_size}")
    print(f"  output:          {output_dir}")

    os.makedirs(output_dir, exist_ok=True)
    train(base_model, pairs, output_dir, args.epochs, args.batch_size, args.warmup_ratio)

    missing = verify_modelcar(output_dir)
    if missing:
        print(f"\n  WARNING: missing ModelCar files: {missing}")
        print("  the Rust eval/server may not load this model")
    else:
        print(f"\n  model saved to {output_dir}")
        print("  all ModelCar files present")


def do_post_eval(args):
    with open(args.raw_report) as f:
        raw = json.load(f)

    trained_labels = None
    if args.trained_labels:
        trained_labels = [l.strip() for l in args.trained_labels.split(",")]

    report = enrich_report(raw, trained_labels)

    print("\n=== evaluation ===")
    print_results(report)

    with open(args.output, "w") as f:
        json.dump(report, f, indent=2)
    print(f"\nreport saved to {args.output}")


def main():
    args = parse_args()
    if args.mode == "train":
        do_train(args)
    elif args.mode == "post-eval":
        do_post_eval(args)


if __name__ == "__main__":
    main()
