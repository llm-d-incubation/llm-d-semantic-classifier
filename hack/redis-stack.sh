#!/usr/bin/env bash
# Start a local Redis Stack (RediSearch) for the semantic-cache integration tests.
set -euo pipefail
exec docker run --rm -p 6379:6379 redis/redis-stack-server:latest
