# 0.22 Cache/Session Optimization — Research

## Repository facts

- `ClassificationInput` carries session metadata as passthrough data; semantic
  classification is currently determined by supplied text.
- The exact-result cache is versioned by runtime identity and normalized text,
  and is intentionally disposable.
- The public protobuf already defines `ABSTAIN`, although production behavior
  currently returns successful rankings for every non-error request.
- The 0.1 specification assigns routing and session authority to the AI Gateway.

## Decision

This first slice adds an explicit context-completeness declaration and abstains
on `DELTA`. It does not add a session cache or make the classifier stateful; an
isolated follow-up is insufficient context when disposable cache state is absent.
