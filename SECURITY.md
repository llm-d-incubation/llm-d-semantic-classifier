# Security Policy

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Report privately using [GitHub Security Advisories](../../security/advisories/new)
for this repository, or contact the maintainer (@cnuland) directly. llm-d-sc
participates in the wider llm-d security process; reports affecting llm-d as a
whole may also be sent to the llm-d security reporting list.

Please include:

- a description of the issue and why you believe it is a security problem
- affected version or commit SHA
- reproduction steps or a proof of concept
- any suggested mitigation

You will receive an acknowledgement, and we will keep you informed as the report
is investigated. Please give us reasonable time to address the issue before
public disclosure.

## Supported Versions

llm-d-sc is **pre-1.0**. Only the latest `0.x` release and `main` receive
security fixes. There are no long-term support branches yet.

## Scope notes for this project

llm-d-sc sits in the inference request path and handles untrusted request
content. Reports are particularly welcome in these areas:

- **Classifier artifact handling**, a malicious or malformed model artifact
  (`/models`) causing memory unsafety, path escape, or code execution.
- **Input handling**, tokenizer or tensor construction faults on adversarial
  input, including oversized or pathological Unicode.
- **Cache identity**, any path by which a cached classification could be served
  under a different classifier, model, tokenizer, or taxonomy revision than the
  one that produced it.
- **Telemetry leakage**, raw prompt or session content appearing in logs,
  metrics, or traces (the design retains only hashes and identifiers).
- **Resource exhaustion**, bypassing bounded inference admission, or unbounded
  memory growth under load.

## Non-scope

- Routing, policy, and guardrail decisions: llm-d-sc returns signals and never
  selects an endpoint. Those concerns belong to the calling gateway.
- The semantic accuracy of a classifier's labels is a model quality issue, not a
  vulnerability.
