#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"

CLASSIFIER="${LLM_D_SC_CLASSIFIER:-complexity}"
MODEL_DIR="${LLM_D_SC_MODEL_DIR:-$REPO_ROOT/artifacts/models/$CLASSIFIER}"
CLASSIFIER_LISTEN="${LLM_D_SC_LISTEN:-127.0.0.1:50051}"
PROXY_HOST="${AGENT_PROXY_HOST:-127.0.0.1}"
PROXY_PORT="${AGENT_PROXY_PORT:-8000}"
LOG_DIR="${AGENT_PROXY_LOG_DIR:-$SCRIPT_DIR/.logs}"
CLASSIFIER_LOG="$LOG_DIR/llm-d-sc.log"
PROXY_LOG="$LOG_DIR/proxy.log"

if ! command -v uv >/dev/null 2>&1; then
    echo "error: uv is required: https://docs.astral.sh/uv/getting-started/installation/" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is required: https://rustup.rs/" >&2
    exit 1
fi
if ! command -v codex >/dev/null 2>&1; then
    echo "error: codex is required: https://learn.chatgpt.com/codex/cli" >&2
    exit 1
fi
if [[ -z "${OPENAI_API_KEY:-}" ]]; then
    echo "error: OPENAI_API_KEY must be set for the routed Codex session" >&2
    exit 1
fi
if [[ "$CLASSIFIER_LISTEN" != *:* ]]; then
    echo "error: LLM_D_SC_LISTEN must be in host:port form" >&2
    exit 1
fi

CLASSIFIER_HOST="${CLASSIFIER_LISTEN%:*}"
CLASSIFIER_PORT="${CLASSIFIER_LISTEN##*:}"
CLASSIFIER_TARGET_HOST="$CLASSIFIER_HOST"
if [[ "$CLASSIFIER_TARGET_HOST" == "0.0.0.0" ]]; then
    CLASSIFIER_TARGET_HOST="127.0.0.1"
fi
PROXY_TARGET_HOST="$PROXY_HOST"
if [[ "$PROXY_TARGET_HOST" == "0.0.0.0" ]]; then
    PROXY_TARGET_HOST="127.0.0.1"
fi

classifier_pid=""
proxy_pid=""
cleanup() {
    local status=$?
    trap - EXIT
    for pid in "$proxy_pid" "$classifier_pid"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    for pid in "$proxy_pid" "$classifier_pid"; do
        if [[ -n "$pid" ]]; then
            wait "$pid" 2>/dev/null || true
        fi
    done
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$LOG_DIR"
: > "$CLASSIFIER_LOG"
: > "$PROXY_LOG"

echo "==> Syncing the Python environment with uv"
uv sync --locked --project "$SCRIPT_DIR"

echo "==> Generating Python gRPC bindings"
mkdir -p "$SCRIPT_DIR/.generated"
uv run --locked --project "$SCRIPT_DIR" python -m grpc_tools.protoc \
    -I "$REPO_ROOT/proto" \
    --python_out "$SCRIPT_DIR/.generated" \
    --grpc_python_out "$SCRIPT_DIR/.generated" \
    "$REPO_ROOT/proto/classify.proto"

if [[ ! -s "$MODEL_DIR/model.safetensors" ]]; then
    if [[ "$MODEL_DIR" != "$REPO_ROOT/artifacts/models/$CLASSIFIER" ]]; then
        echo "error: model artifact is missing from $MODEL_DIR" >&2
        exit 1
    fi
    echo "==> Fetching the $CLASSIFIER classifier model"
    uv run --locked --project "$SCRIPT_DIR" \
        "$REPO_ROOT/hack/fetch-model" --classifier "$CLASSIFIER"
fi

echo "==> Building the Rust classifier"
PROTOC="$SCRIPT_DIR/protoc" \
    cargo build --release --bin llm-d-sc-server --manifest-path "$REPO_ROOT/Cargo.toml"

echo "==> Starting the Rust classifier on $CLASSIFIER_LISTEN"
LLM_D_SC_MODEL_DIR="$MODEL_DIR" \
LLM_D_SC_CLASSIFIER="$CLASSIFIER" \
LLM_D_SC_LISTEN="$CLASSIFIER_LISTEN" \
    "$REPO_ROOT/target/release/llm-d-sc-server" \
    >>"$CLASSIFIER_LOG" 2>&1 &
classifier_pid=$!

echo "==> Waiting for the classifier to become ready"
if ! uv run --locked --project "$SCRIPT_DIR" python - \
    "$CLASSIFIER_TARGET_HOST" "$CLASSIFIER_PORT" "$classifier_pid" <<'PY'
import os
import socket
import sys
import time

host, port, pid = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
deadline = time.monotonic() + 120
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        raise SystemExit(2)
    try:
        with socket.create_connection((host, port), timeout=1):
            raise SystemExit(0)
    except OSError:
        time.sleep(0.25)
raise SystemExit(1)
PY
then
    if ! kill -0 "$classifier_pid" 2>/dev/null; then
        wait "$classifier_pid" || true
        echo "error: classifier exited before becoming ready" >&2
    else
        echo "error: classifier did not become ready within 120 seconds" >&2
    fi
    exit 1
fi

echo "==> Starting the agent proxy at http://$PROXY_HOST:$PROXY_PORT"
LLM_D_SC_TARGET="$CLASSIFIER_TARGET_HOST:$CLASSIFIER_PORT" \
LLM_D_SC_CLASSIFIER="$CLASSIFIER" \
PYTHONUNBUFFERED=1 \
    uv run --locked --project "$SCRIPT_DIR" \
    uvicorn --app-dir "$SCRIPT_DIR" proxy:app \
    --host "$PROXY_HOST" --port "$PROXY_PORT" \
    >>"$PROXY_LOG" 2>&1 &
proxy_pid=$!

echo "==> Waiting for the agent proxy to become ready"
if ! uv run --locked --project "$SCRIPT_DIR" python - \
    "$PROXY_TARGET_HOST" "$PROXY_PORT" "$proxy_pid" <<'PY'
import os
import socket
import sys
import time

host, port, pid = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
deadline = time.monotonic() + 30
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        raise SystemExit(2)
    try:
        with socket.create_connection((host, port), timeout=1):
            raise SystemExit(0)
    except OSError:
        time.sleep(0.25)
raise SystemExit(1)
PY
then
    if ! kill -0 "$proxy_pid" 2>/dev/null; then
        wait "$proxy_pid" || true
        echo "error: agent proxy exited before becoming ready" >&2
    else
        echo "error: agent proxy did not become ready within 30 seconds" >&2
    fi
    exit 1
fi

PROXY_BASE_URL="http://$PROXY_TARGET_HOST:$PROXY_PORT/v1"
echo "==> Launching Codex through the semantic router"
echo "    provider: $PROXY_BASE_URL"
echo "    service logs: $SCRIPT_DIR/logs.sh"
echo "    exiting Codex will stop the proxy and classifier"

codex \
    --config 'model_provider="semantic_proxy"' \
    --config 'model_providers.semantic_proxy.name="Local semantic router"' \
    --config "model_providers.semantic_proxy.base_url=\"$PROXY_BASE_URL\"" \
    --config 'model_providers.semantic_proxy.env_key="OPENAI_API_KEY"' \
    --config 'model_providers.semantic_proxy.wire_api="responses"' \
    "$@"
