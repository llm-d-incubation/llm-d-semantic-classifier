# 0.1 MVP Specification

## Problem
Praxis needs a low-latency semantic-classification callout without embedding classifier runtime/model lifecycle into the router.

## Desired behavior
A long-lived Rust service accepts a classification request, invokes a resident classifier through a runtime abstraction, and returns versioned semantic evidence. Praxis remains responsible for routing/enforcement.

## In scope
- Rust service;
- gRPC classification API;
- `ClassifierRuntime` abstraction;
- Candle first backend;
- model/tokenizer residency;
- supplied sensitivity embedding model fixture;
- Red Hat-style OCI ModelCar delivery;
- dummy Praxis integration;
- exact-result cache;
- bounded MVP queue;
- warmup/readiness;
- latency decomposition metrics;
- OpenShift sidecar and ClusterIP benchmark.

## Non-goals
Routing policy, stickiness, distributed state, training/SDG/fine-tuning, custom kernels, vLLM backend implementation, multi-signal orchestration, production control plane, universal hard 20 ms SLA.

## State
Authoritative: active classifier/model/tokenizer/runtime revision and metadata required to reproduce a result.

Disposable: exact-result cache and future feature/session caches.

Routing/session authority remains Praxis.

## Failure contract
- missing/corrupt model -> not ready;
- full queue -> explicit resource exhausted;
- expired queued request -> do not infer;
- runtime error -> explicit unavailable/error, never fabricated label;
- insufficient context where required -> abstain.

## Acceptance criteria
- AC-001 clean Rust build/server lifecycle.
- AC-002 not-ready before model load/warmup.
- AC-003 ModelCar supplies required files with no runtime HF fetch.
- AC-004 pinned sensitivity model matches trusted reference embedding/ranking fixtures.
- AC-005 model/tokenizer load once per active revision.
- AC-006 cache hit bypasses tokenizer/model forward.
- AC-007 identical concurrent misses do not create unbounded forwards.
- AC-008 queue bounded; overload explicit.
- AC-009 dummy Praxis consumes response over persistent gRPC.
- AC-010 response contains signals, not final route.
- AC-011 OpenShift sidecar/ClusterIP RTT distributions captured.
- AC-012 queue/tokenize/forward/total latency visible.
- AC-013 restart + complete context recomputes correctly.
- AC-014 default telemetry contains no raw prompt/session text.
