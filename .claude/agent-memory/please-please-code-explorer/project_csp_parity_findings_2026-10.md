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
- Resolved (#126): csp has a direct rayon dep, and `create_index_from_path` (create.rs) parallelizes file read+hash+chunk+tokenize in `INDEX_BATCH_SIZE` batches, then applies index mutations serially in walk order (#295 analog).
- Cache validity = sha256 over every source file on each `load_or_build_index` (cache_orchestrator.rs `source_fingerprint`) and again per-file in `create_index_from_path` on a stale rebuild (double read+hash). ADR-0005 accepts this cost.
- `Bm25Index::set_doc_order` (indexing/sparse.rs) is lenient; upstream #284 made it raise unless the order lists every indexed doc exactly once.
- Resolved (#123): csp `find_related` (indexing/index.rs) restricts candidates to the non-empty seed language and searches unrestricted for absent or empty languages, matching upstream #29.

**Why:** avoids re-deriving these when triaging future upstream sync batches.
**How to apply:** re-verify file:line before citing; code may have moved.
