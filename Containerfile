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

# The image is built WITH `redis-semantic` so the L2 semantic cache can be
# switched on at runtime via `LLM_D_SC_CACHE=redis-semantic` (default `exact`,
# i.e. off) without needing a different image. The crate's DEFAULT build stays
# dependency-light to hold MSRV 1.75; this builder is a separate toolchain
# (rust:1-bookworm >= 1.80), so that intent is unaffected.
FROM rust:1-bookworm AS builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler build-essential \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Layer 1: copy manifests so dependency compilation is cacheable.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin \
 && echo 'fn main() {}' > src/lib.rs \
 && echo 'fn main() {}' > src/bin/server.rs \
 && echo 'fn main() {}' > src/bin/classify.rs \
 && echo 'fn main() {}' > src/bin/gateway-probe.rs \
 && cargo build --release --features redis-semantic \
        --bin llm-d-sc-server \
        --bin llm-d-sc-classify \
        --bin llm-d-sc-gateway-probe \
 && rm -rf target/release/deps target/release/.fingerprint target/release/build

# Layer 2: copy source; only the final build is incremental.
COPY . .
RUN cargo build --release --features redis-semantic \
      --bin llm-d-sc-server \
      --bin llm-d-sc-classify \
      --bin llm-d-sc-gateway-probe

# Runtime base MUST match the builder's glibc. The previous pairing was
# `rust:1-slim` (current Debian, glibc 2.39+) against ubi9/ubi-minimal
# (glibc 2.34), which produces an image that builds and pushes cleanly and then
# dies instantly with:
#   /usr/local/bin/llm-d-sc: /lib64/libc.so.6: version `GLIBC_2.39' not found
# A successful build proves nothing about a dynamically linked binary, because
# the build never executes it. Both stages are pinned to bookworm so the glibc
# the binary is linked against is the glibc it runs on.
#
# FOLLOW-UP: a Red Hat project should ship a UBI runtime. That needs a UBI-based
# Rust builder (ubi9/rust-toolset) or a static musl build, and is tracked
# separately rather than blocking the topology evidence.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
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
