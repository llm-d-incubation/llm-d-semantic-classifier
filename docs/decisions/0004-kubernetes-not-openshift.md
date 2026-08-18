# ADR-0004: Target Kubernetes generically, never a named distribution

Date: 2026-08-18
Status: Accepted

## Decision

Every reference to "OpenShift" becomes "Kubernetes", or the vendor-neutral
mechanism it actually depends on. This applies to code comments, specs, evidence
files, the test matrix, fixtures, and docs.

## Rationale

llm-d-sc must run on any conformant Kubernetes. Naming one distribution in the
specs and test matrix implies a dependency that does not exist and narrows the
project's audience upstream. The behaviours we test are Kubernetes behaviours;
one distribution merely enforces some of them by default.

## Translations, not find-and-replace

Some references carry meaning that must survive the rename:

| Before | After | Why |
|---|---|---|
| "OpenShift system" (test tier) | "Kubernetes system" | tier name only |
| "OpenShift random UID / restricted context" | "arbitrary non-root UID under a restricted security context" | the property under test is running as an arbitrary UID the image does not control; that is `restricted` PodSecurity behaviour generally |
| "Red Hat/OpenShift-style ModelCar" | "ModelCar-style OCI model artifact" | the artifact pattern, not a vendor |
| "OpenShift AI" (as a product reference) | drop, or "a Kubernetes model-serving platform" | avoid product naming |
| "OpenShift cluster" | "Kubernetes cluster" | plain |

Test IDs are unchanged: `S-001`, `S-010`, and the rest remain stable evidence
anchors regardless of wording.

## Scope

63 lines across `src/`, `tests/`, `specs/`, and `docs/` at time of writing.
Executed as part of the mechanical rename slice alongside ADR-0003, under the
normal evidence gates, so the test count cannot silently drop.
