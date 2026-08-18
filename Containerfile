# llm-d-sc OpenShift service container: multi-stage build, no model baked in.
#
# The real model is NOT in this image; it arrives via a ModelCar mount at /models
# (LLM_D_SC_MODEL_DIR). The served pipeline loads and warms that real model.
# KNOWN GAP: the classifier DEFINITION (prototypes/taxonomy) is still read from
# committed fixtures rather than the artifact — see README integration gap 3.

FROM rust:1-slim AS builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler build-essential \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
RUN cargo build --release --bin llm-d-sc-server

FROM registry.access.redhat.com/ubi9/ubi-minimal AS runtime
ENV LLM_D_SC_LISTEN=0.0.0.0:50051 \
    LLM_D_SC_MODEL_DIR=/models

# Service binary.
COPY --from=builder /src/target/release/llm-d-sc-server /usr/local/bin/llm-d-sc

# Deterministic synthetic fixtures (tokenizer + prototypes) required to boot.
# These are NOT a model — the real model is mounted at /models.
COPY --from=builder /src/tests/fixtures/modelcar/tokenizer.json \
     /src/tests/fixtures/modelcar/tokenizer.json
COPY --from=builder /src/tests/fixtures/modelcar/synthetic-prototypes.json \
     /src/tests/fixtures/modelcar/synthetic-prototypes.json

USER 65534
EXPOSE 50051
ENTRYPOINT ["/usr/local/bin/llm-d-sc"]
