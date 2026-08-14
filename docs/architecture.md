# Architecture notes (promoted truths live here)

## Target service (context; not yet built)
llm-d-sc: Rust semantic-classifier runtime. Signals: domain, complexity, sensitivity —
ranked, calibrated, with confidence and abstention. Classifies; never routes (Praxis owns
routing: policy, sessions, stickiness, fallback, capacity). Stack: tokio + tonic/axum,
Candle + ModernBERT first backend (pluggable), moka cache, bounded inference workers,
resident model/tokenizer, warmup before readiness, sub-20ms uncached budget, p99 discipline.
Reference research: the maintainer's research corpus (pipeline-agentic-research.md,
rust-service-research.md, "llm-d-sc Classifier Runtime Service").

## Cluster
Namespace `llm-d-sc` on the ironman OpenShift cluster hosts future dev/validation
deployments. The DSV4 worker model serves from namespace `homelab-maas` (llama-server-ds4).
