# llm-d-sc Kubernetes service container: multi-stage build, no model baked in.
#
# The model is NOT in this image. It arrives via a ModelCar mount at /models
# (LLM_D_SC_MODEL_DIR), so the image is model-agnostic and a classifier revision
# change never requires rebuilding the service.
#
# The classifier DEFINITION (labels + anchors) IS compiled in: the three built-in
# taxonomies are small text files, and shipping them means an instance can always
# classify against a real taxonomy with no external file. A custom definition can
# still be supplied at runtime by pointing LLM_D_SC_CLASSIFIER at a path.

FROM rust:1-slim AS builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler build-essential \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
RUN cargo build --release \
      --bin llm-d-sc-server \
      --bin llm-d-sc-classify \
      --bin llm-d-sc-gateway-probe

FROM registry.access.redhat.com/ubi9/ubi-minimal AS runtime
ENV LLM_D_SC_LISTEN=0.0.0.0:50051 \
    LLM_D_SC_MODEL_DIR=/models \
    LLM_D_SC_CLASSIFIER=complexity

# Service binary.
COPY --from=builder /src/target/release/llm-d-sc-server /usr/local/bin/llm-d-sc
# Demo CLI, and the dummy-gateway RTT probe used for the topology evidence
# (P-030..P-033): the probe must run INSIDE the cluster to measure the real
# same-Pod and ClusterIP network paths.
COPY --from=builder /src/target/release/llm-d-sc-classify /usr/local/bin/llm-d-sc-classify
COPY --from=builder /src/target/release/llm-d-sc-gateway-probe /usr/local/bin/llm-d-sc-gateway-probe

# Deterministic synthetic fixtures for the weight-free pipeline. These are NOT a
# model; the real model is mounted at /models.
COPY --from=builder /src/tests/fixtures/modelcar/tokenizer.json \
     /src/tests/fixtures/modelcar/tokenizer.json
COPY --from=builder /src/tests/fixtures/modelcar/synthetic-prototypes.json \
     /src/tests/fixtures/modelcar/synthetic-prototypes.json

USER 65534
EXPOSE 50051
ENTRYPOINT ["/usr/local/bin/llm-d-sc"]
