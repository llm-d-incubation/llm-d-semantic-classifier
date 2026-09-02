# 0.22 Cache/Session Optimization — Slice 1 Design

```text
Gateway supplies request
  ├─ FULL or UNSPECIFIED context -> ServiceCore -> cache -> classifier
  └─ DELTA context               -> ServiceCore -> ABSTAIN
```

`ContextCompleteness` is a typed field in the core input and an additive enum in
the protobuf request. The gRPC adapter maps between them. `ServiceCore` enforces
the rule before it constructs a cache key, so every runtime backend inherits the
same behavior and raw forwards cannot be accidentally invoked.

An abstention result carries the loaded runtime metadata for provenance, has an
empty ranking, and is not stored in the exact-result cache. No session id is
added to the cache key: it is correlation metadata, not semantic input.
