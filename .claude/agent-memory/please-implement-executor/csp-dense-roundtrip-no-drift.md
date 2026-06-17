---
name: csp-dense-roundtrip-no-drift
description: SelectableBasicBackend save→load is bit-stable (no float drift) — re-normalization of unit vectors is idempotent; NFR-002 roundtrip safe
metadata:
  type: project
---

`SelectableBasicBackend` (src/indexing/dense.ts) save→load roundtrip is **bit-stable, no float drift**.

**Why:** The constructor L2-normalizes vectors in place. `save` writes the already-normalized
vectors to `vectors.bin` (Float32). `load` reconstructs via the constructor, which **re-normalizes**
— but re-normalizing a unit-length vector divides by ≈1.0 (idempotent). Measured with an isolated
probe: `maxDiff(b1.vectors, loaded.vectors) = 0`, second roundtrip also 0, query ranking identical.

**How to apply:** This settles the T006/T007 STOP condition (dense float drift breaking NFR-002
roundtrip equivalence) — it does NOT trigger. T007 `loadFromDisk` can reuse
`SelectableBasicBackend.load` directly; no "save unnormalized" or `skipNormalize` workaround needed.
Related: [[csp-test-files-ahead-of-impl]] (T007 loadFromDisk tests already exist in index.test.ts).

The five persisted index artifacts have mutually distinct names — no collision:
`manifest.json` + `chunks.json` (CspIndex.save) / `bm25.json` (Bm25Index.save) /
`vectors.bin` + `args.json` (SelectableBasicBackend.save).
