# 0.1 MVP Design

```text
Dummy Praxis
   | gRPC
   v
API/tonic
   |
normalize + semantic fingerprint
   |
exact result cache
   | miss
bounded scheduler
   |
classifier registry
   |
ClassifierRuntime
   |
Candle backend
   |
tokenizer + transformer + pooling/prototype adapter
```

Tokio owns network I/O; model execution is bounded separately.

The cache key fingerprints every semantic input to the classification result: classifier/model/tokenizer/taxonomy/preprocessing revision plus normalized supplied context. Do not use a raw prompt string as the sole cache identity.

The model fixture is SentenceTransformers-style embedding inference. The adapter loads tokenizer/weights/pooling config, computes the embedding, and applies a versioned classifier definition (for example prototypes/anchors) only after the exact taxonomy is verified. Rust outputs are compared against trusted Python-reference golden fixtures.

The service and model remain separate OCI images. The ModelCar materializes `/models`; inference uses local read-only model data.
