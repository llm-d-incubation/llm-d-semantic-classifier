#!/usr/bin/env bash
# Regression checks for Issue #5: a present artifact must match its source record.
set -euo pipefail

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/hack" "$root/bin" "$root/artifacts/models/complexity/1_Pooling"
cp "$(dirname "$0")/../hack/fetch-model" "$root/hack/fetch-model"
chmod +x "$root/hack/fetch-model"

cat > "$root/bin/python3" <<'SH'
#!/bin/sh
shift
repo="$1"; rev="$2"; target="$3"; shift 3
mkdir -p "$target"
for name in "$@"; do
  mkdir -p "$(dirname "$target/$name")"
  printf '%s@%s\n' "$repo" "$rev" > "$target/$name"
done
printf '%s@%s\n' "$repo" "$rev" > downloaded
SH
chmod +x "$root/bin/python3"

for file in model.safetensors tokenizer.json config.json modules.json 1_Pooling/config.json; do
  printf 'existing\n' > "$root/artifacts/models/complexity/$file"
done
printf '%s\n%s\n' \
  'cnuland/llm-d-sc-complexity' \
  'c5f55ef419d268ba843c544dc00988d1e9878044' \
  > "$root/artifacts/models/complexity/.fetch-model-source"

(cd "$root" && PATH="$root/bin:$PATH" ./hack/fetch-model > output)
grep -q 'skipping download' "$root/output"
test ! -e "$root/downloaded"

printf '%s\n%s\n' 'cnuland/llm-d-sc-complexity' 'old-revision' \
  > "$root/artifacts/models/complexity/.fetch-model-source"
(cd "$root" && PATH="$root/bin:$PATH" ./hack/fetch-model > output)
grep -q 'refreshing' "$root/output"
grep -qx 'cnuland/llm-d-sc-complexity@c5f55ef419d268ba843c544dc00988d1e9878044' "$root/downloaded"
grep -qx 'c5f55ef419d268ba843c544dc00988d1e9878044' \
  <(sed -n '2p' "$root/artifacts/models/complexity/.fetch-model-source")

rm -f "$root/downloaded" "$root/artifacts/models/complexity/.fetch-model-source"
(cd "$root" && PATH="$root/bin:$PATH" ./hack/fetch-model > output)
grep -q 'refreshing' "$root/output"
grep -qx 'cnuland/llm-d-sc-complexity@c5f55ef419d268ba843c544dc00988d1e9878044' "$root/downloaded"

echo '[fetch-model test] PASS'
