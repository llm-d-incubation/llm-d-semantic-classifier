# ADR-0003: The routing peer is "the AI Gateway", not a named product

Date: 2026-08-18
Status: Accepted

## Decision

Every reference to "Praxis" is replaced by "the AI Gateway" (or "gateway") in
code, specs, docs, tests, and fixtures. Identifiers rename accordingly:

| Before | After |
|---|---|
| `src/dummy_praxis.rs` | `src/dummy_gateway.rs` |
| `DummyPraxis`, `DummyRequest` | `DummyGateway`, `GatewayRequest` |
| `tests/DUMMY_PRAXIS.md` | `tests/DUMMY_GATEWAY.md` |
| "Praxis owns routing" | "the AI Gateway owns routing" |

## Rationale

llm-d-sc is being prepared for upstream as a community project that must work
with ANY inference gateway, not one named implementation. Naming a specific
product in the wire contract, the specs, and the test fixtures implies a
coupling that does not exist and would not be accepted upstream: the boundary
we actually assert is "this service classifies; the gateway routes".

This is a boundary rename, not a behaviour change. The architectural rule is
unchanged and still enforced by the schema (ADR-0001: no route field) and by
the dummy client applying its own policy AFTER classification.

## Scope

Mechanical across the repository (38 files at time of writing) plus three
commit messages, which are handled during the upstream history rewrite
(docs/UPSTREAM-STRATEGY.md §4) rather than by rewriting local history now.

The dummy client keeps its role exactly: a minimal gateway stand-in that
proves routing authority lives outside llm-d-sc. Only its name changes.
