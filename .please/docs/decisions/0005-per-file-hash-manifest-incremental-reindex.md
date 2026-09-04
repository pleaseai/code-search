# ADR 0005 — Incremental reindexing keyed on a per-file content hash (not `mtime_ns`)

- **Status**: Accepted
- **Date**: 2026-09-04
- **Deciders**: csp maintainers
- **Context**: [Issue #84](https://github.com/pleaseai/code-search/issues/84) — parity with upstream semble [#225](https://github.com/MinishLab/semble/pull/225) (partial / incremental reindexing)
- **Builds on**: [ADR 0002](0002-index-storage-cache-model.md) (global `~/.csp/index/` content-hash cache)

## Context

Upstream semble #225 added **partial reindexing**: when a cached index is stale, only the files
that changed since it was built are re-chunked and re-embedded; unchanged files keep their
chunks, their rows in the vector matrix, and their BM25 postings. Upstream detects change per
file with `mtime_ns` recorded in a `files` manifest (`{mtime_ns, start, count}`), rebuilt the
sparse side around an own incremental `BM25` class (`add_document` / `remove_document` /
`set_doc_order`, JSON persistence of per-document term counts + document order), and
validates the cached artifacts' alignment in `load_previous_for_incremental` before reusing them.

Before this decision csp rebuilt the **whole** index whenever the whole-tree content hash in the
manifest mismatched the live tree (ADR 0002). csp never used mtimes: the cache-validity oracle
is a sha256 over `(path, bytes)` of every indexable file, chosen in ADR 0002 because mtimes are
unreliable across `git checkout`, copies, CI caches, and container mounts.

## Decision

Port the incremental machinery from #225, but key the per-file manifest on a **per-file content
hash** instead of `mtime_ns`:

- `IndexManifest.files: {indexed_path → {hash, start, count}}` where `hash` is the sha256 (hex)
  of the file bytes at index time (`indexing::cache::sha256_hex`). `start`/`count` are the
  file's chunk range in the global chunk list, exactly as upstream.
- The whole-tree `contentHash` stays as the **fast path**: when it matches, the cache is loaded
  as before with no per-file work. Only on a mismatch does `load_or_build_index` call
  `load_previous_for_incremental` and seed `CspIndex::from_path_with_previous` with the
  previous chunks / vectors / manifest / BM25 index.
- `create_index_from_path` reads every file once (it already had to for the whole-tree hash),
  hashes it, and reuses the previous rows when the hash matches the manifest entry; otherwise it
  re-chunks, re-embeds, and replaces that file's BM25 postings. Files missing from the new walk
  have their postings removed. Rows are **moved** out of the previous index (no vector copies),
  and reused rows are not re-normalised, so they stay bit-identical.
- `Bm25Index` becomes the id-keyed incremental index from upstream `bm25.py` with stable chunk
  ids `"{indexed_path}:{slot}"` (`indexing::types::make_chunk_id`). `bm25.json` now persists
  `{version: 2, documents, docOrder}`; postings are rebuilt on load, and a document order that
  does not describe exactly the persisted documents is rejected.
- `INDEX_SCHEMA_VERSION` is bumped to **2**. `load_from_disk` rejects any other version (it
  already did) and additionally rejects component count mismatches between chunks, vectors, and
  the BM25 document order. `load_previous_for_incremental` fails closed on any structural
  inconsistency (missing/empty `files`, non-tiling or overlapping ranges, chunk paths that do
  not match their range, BM25 order ≠ manifest-derived ids, model/chunk-size/schema mismatch),
  so a full rebuild is always the fallback.

## Alternatives considered

1. **`mtime_ns` like upstream** — rejected: it contradicts the ADR 0002 oracle and would make
   the per-file decision disagree with the whole-tree decision (a `git checkout` that restores
   identical bytes bumps mtimes and would force needless re-embedding; a same-second edit could
   be missed). One oracle, one answer.
2. **Reuse the whole-tree hash only (status quo)** — rejected: any single-file edit re-embeds
   the entire repository, which is the cost #225 exists to remove.
3. **Store per-file hashes only, drop the whole-tree hash** — rejected: the whole-tree hash is a
   single string compare on the hit path and avoids loading chunks/vectors/BM25 at all when
   nothing changed; keeping both costs one extra hex string per file in the manifest.

## Consequences

- A stale cache now costs roughly `O(changed files)` embedding work plus one hash pass over the
  tree, instead of a full re-embed. The hit path is unchanged.
- Existing v1 caches are rebuilt once (schema bump), then benefit from incremental reuse.
- The BM25 **scoring** is intentionally unchanged from the previous csp implementation (Lucene
  IDF with the `(k1+1)` numerator, de-duplicated query terms). Upstream's own class dropped the
  `(k1+1)` factor and weights repeated query terms by their query frequency; both differences
  scale scores without changing ranks, and ranks are all that Reciprocal Rank Fusion consumes.
  Recorded as an intentional adaptation in `.please/docs/references/semble.md` §6.1.
- Git sources (`from_git`) are URL+ref keyed and never take the incremental path, matching
  upstream (which only seeds `from_path`).
