---
name: csp-loadfromdisk-model-dim-alignment
description: On index load, the reloaded stub model is fixed 256-dim regardless of modelId — align it to the persisted backend dim or query() throws a dim mismatch
metadata:
  type: project
---

`CspIndex.loadFromDisk` (src/indexing/index.ts) reloads the embedding model via
`loadModel(manifest.modelId)`, but the **stub** `loadModel` (dense.ts) ignores modelId and always
returns a fixed **256-dim** model (`_DEFAULT_STUB_DIM=256`).

**Why it bites:** `index.test.ts`'s `buildIndex` fixture pairs `makeStubModel(4)` with hand-made
4-dim vectors. A naive reload then makes `search` encode a 256-dim query and call
`SelectableBasicBackend.query` against 4-dim stored vectors → `Query vector dimension mismatch` throw.

**How to apply:** After reloading, if `model.dim !== semanticIndex.dim`, rebuild the query model with
`makeStubModel(semanticIndex.dim)`. This is a stub-era-only correction: the real Model2Vec model has a
weight-fixed dim, and `fromPath` always embeds+queries with the same `loadModel` instance so dims
already agree (the branch never fires in the real pipeline). Future cache work (T009/T010 loadOrBuildIndex)
that restores indexes from disk must preserve this alignment.

Related: [[csp-dense-roundtrip-no-drift]] (the dense backend itself round-trips bit-stable;
the dim issue is purely about the *separately reloaded* query model, not the stored vectors).
