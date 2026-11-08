---
name: csp-parity-findings-2026-10
description: Non-obvious csp-vs-semble facts found in the 2026-10-14 parity pass (#260-#295); where csp already matches vs real gaps
metadata:
  type: project
---

Facts verified by reading code on 2026-10-14 (upstream semble HEAD 0e5e028, #295):

- csp `Model::encode` (crates/csp/src/indexing/dense.rs:107) calls model2vec-rs `encode()`, which hardcodes `max_length=Some(512)` + batch 1024. Upstream #264 passes `max_length=None` for chunk embedding only (queries stay truncated). Fix = `StaticModel::encode_with_args(texts, None, 1024)` in the chunk-embedding path only.
- csp embeds fresh rows in ONE batch (create.rs `embed_fresh_rows`) so the #260 double-embedding bug never existed.
- csp `chunk_source` (chunking/source.rs) already resolves line numbers in one forward pass (#266 is moot), but `byte_to_char` in chunking/core.rs:102 rescans `text[..b]` per boundary (quadratic on huge files; upstream has same pattern).
- csp has no direct rayon dep (only transitive via tokenizers/model2vec-rs); all file read+hash+chunk+tokenize is serial in `create_index_from_path` (#295 analog).
- Cache validity = sha256 over every source file on each `load_or_build_index` (cache_orchestrator.rs `source_fingerprint`) and again per-file in `create_index_from_path` on a stale rebuild (double read+hash). ADR-0005 accepts this cost.
- `Bm25Index::set_doc_order` (indexing/sparse.rs) is lenient; upstream #284 made it raise unless the order lists every indexed doc exactly once.
- csp `find_related` (indexing/index.rs:259) passes `selector=None`; upstream restricts to same-language chunks (since upstream #29). Unverified whether csp intends this.

**Why:** avoids re-deriving these when triaging future upstream sync batches.
**How to apply:** re-verify file:line before citing; code may have moved.
