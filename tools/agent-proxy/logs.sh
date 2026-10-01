#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="${AGENT_PROXY_LOG_DIR:-$SCRIPT_DIR/.logs}"

case "${1:-all}" in
    classifier|llm-d-sc)
        logs=("$LOG_DIR/llm-d-sc.log")
        ;;
    proxy)
        logs=("$LOG_DIR/proxy.log")
        ;;
    all)
        logs=("$LOG_DIR/llm-d-sc.log" "$LOG_DIR/proxy.log")
        ;;
    *)
        echo "usage: $0 [all|classifier|proxy]" >&2
        exit 2
        ;;
esac

echo "Following logs; press Ctrl-C to stop."
tail -n 0 -F "${logs[@]}"
