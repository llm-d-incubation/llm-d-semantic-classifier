# Sensitivity ModelCar Fixture

Source: `cnuland/semantic-routing-sensitivity`

Pinned Hugging Face revision:
`43f21d21ac48134464f8510a9ac9c95bdac7ba86`

Build:

```bash
podman build --format=oci \
  -f Containerfile \
  -t quay.io/<org>/semantic-routing-sensitivity:43f21d2 .
```

Then push and resolve the immutable OCI digest for promotion/system tests.

## Red Hat/OpenShift pattern

The final image uses UBI Micro, places model data under `/models`, owns copied files as root/root, and makes them readable/executable as appropriate for OpenShift random-user execution. The serving binary remains in a separate image.

## Taxonomy caution

The Hugging Face model is a SentenceTransformers embedding model trained over five labels. The public model card examples expose some category names but should not be used to invent an uncertain full label-index mapping.

Before golden **classification** assertions:
1. recover/commit the exact training taxonomy/index mapping;
2. commit versioned prototypes/anchors or equivalent classifier definition;
3. generate golden outputs with the pinned trusted Python reference implementation;
4. require Rust embedding/ranking parity within a documented tolerance.

Until that mapping is verified, tests may safely prove model loading, tokenization, pooling, embedding parity, and OCI delivery without pretending the unknown mapping is known.

## Disconnected proof

After the ModelCar is built/pushed, deny Hugging Face egress in the runtime namespace and verify llm-d-sc still starts solely from `/models`.
