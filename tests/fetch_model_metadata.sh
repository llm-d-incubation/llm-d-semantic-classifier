#!/usr/bin/env bash
# Keep classifier metadata aligned with the exact sources fetched by hack/fetch-model.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 <<'PY'
import json
import re
from pathlib import Path

fetch_model = Path("hack/fetch-model").read_text(encoding="utf-8")
errors = []

for path in sorted(Path("classifiers").glob("*.json")):
    definition = json.loads(path.read_text(encoding="utf-8"))
    classifier_id = definition["classifier_id"]
    expected_repo = definition["model_repo"]
    expected_revision = definition["model_revision"]

    block_match = re.search(
        rf"^  {re.escape(classifier_id)}\)\n(?P<block>.*?)^    ;;$",
        fetch_model,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not block_match:
        errors.append(f"{classifier_id}: missing fetch-model case block")
        continue

    block = block_match.group("block")
    repo_match = re.search(r'^\s+REPO="([^"]+)"$', block, flags=re.MULTILINE)
    revision_match = re.search(r'^\s+REV="([^"]+)"$', block, flags=re.MULTILINE)

    actual_repo = repo_match.group(1) if repo_match else None
    actual_revision = revision_match.group(1) if revision_match else None

    if actual_repo != expected_repo:
        errors.append(
            f"{classifier_id}: model_repo {expected_repo!r} does not match "
            f"fetch-model REPO {actual_repo!r}"
        )
    if actual_revision != expected_revision:
        errors.append(
            f"{classifier_id}: model_revision {expected_revision!r} does not match "
            f"fetch-model REV {actual_revision!r}"
        )

if errors:
    raise SystemExit("\n".join(errors))
PY

for classifier in classifiers/*.json; do
  classifier="${classifier#classifiers/}"
  classifier="${classifier%.json}"
  fetch_block="$(sed -n "/^  ${classifier})/,/^    ;;$/p" hack/fetch-model)"
  grep -Fq "TARGET=\"artifacts/models/${classifier}\"" <<<"$fetch_block"
done

echo "[fetch-model-metadata] classifier sources match fetch-model metadata"
