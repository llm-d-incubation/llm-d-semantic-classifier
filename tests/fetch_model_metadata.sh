#!/usr/bin/env bash
# Keep classifier metadata aligned with the exact source fetched by hack/fetch-model.
set -euo pipefail
cd "$(dirname "$0")/.."

readonly COST_REPO="cnuland/llm-d-sc-cost"
readonly COST_REVISION="685354650683112d7a13209a90ced7e5656d1245"

python3 - "$COST_REPO" "$COST_REVISION" <<'PY'
import json
import sys

repo, revision = sys.argv[1:]
with open("classifiers/cost.json", encoding="utf-8") as f:
    definition = json.load(f)
if definition["model_repo"] != repo or definition["model_revision"] != revision:
    raise SystemExit("cost classifier metadata does not match the expected source")
PY

cost_fetch_block="$(sed -n '/^  cost)/,/^  ;;$/p' hack/fetch-model)"
printf '%s\n' "$cost_fetch_block" | grep -Fq "REPO=\"$COST_REPO\""
printf '%s\n' "$cost_fetch_block" | grep -Fq "REV=\"$COST_REVISION\""

echo "[fetch-model-metadata] cost source matches classifier metadata"
