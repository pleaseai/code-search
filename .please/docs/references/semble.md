# Reference Analysis — MinishLab/semble → Rust port (`crates/csp`)

> Module-by-module analysis of [MinishLab/semble](https://github.com/MinishLab/semble) (the
> Python original) mapped to the **Rust port** under `crates/csp` (the library **and** the
> `csp` binary, `src/bin/csp/`), introduced by [ADR-0003](../decisions/0003-rewrite-in-rust.md). Each section
> captures the load-bearing algorithm + its constants, the Rust-specific structure/idioms, and
> where the port diverges.
>
> **Analyzed at**: upstream semble `136b6f7` (2026-06-18); Rust port baseline `2f2baa2`
> (PR #34 "Rust rewrite foundation"), since advanced by PR #37 (ranking wired + chunk 750).
> **Source of truth**: the **Python upstream** (`MinishLab/semble`) — the Rust port targets
> behavioral equivalence with it. The TS `src/` is **deprecated** (slated for deletion, kept
> only as a historical/reference implementation) and is no longer the parity oracle; its test
> suite remains usable as language-neutral golden fixtures for already-ported modules, but where
> the Rust port has moved past the old TS stubs (real dense embeddings, wired ranking, chunk
> length 750) the upstream Python is authoritative.
> **Upstream layout**: Python `src/semble/`. **Port layout**: `crates/csp/src/` (lib) +
> `crates/csp/src/bin/csp/` (the `csp` binary, behind the `cli` feature).

---

## 1. What semble is

A hybrid **dense + sparse** code-search engine for AI agents that runs entirely on CPU with no
API keys and no GPU. It indexes a local directory or a cloned git repo, then answers
natural-language or symbol queries in single-digit milliseconds. Two retrieval signals:

- **Dense**: [Model2Vec](https://github.com/MinishLab/model2vec) static embeddings
  (`minishlab/potion-code-16M`, 256-dim) — vocab→vector lookup + mean pooling, *not* a
  transformer forward pass. CPU-fast. The Rust port wires the official
  [`model2vec-rs`](https://crates.io/crates/model2vec-rs) `StaticModel`, with a deterministic
  stub fallback (see §4.6).
- **Sparse**: BM25 over identifier-aware tokens (semble uses `bm25s`; Rust ports BM25 directly).

They are fused with **Reciprocal Rank Fusion** and then reranked with code-specific priors
(definition boosts, path penalties, file-saturation decay).

---

## 2. Pipeline at a glance

```
                       INDEX (once, cached)                          SEARCH (per query)
  ┌────────────────────────────────────────────┐     ┌──────────────────────────────────────────┐
  walk_files  ──► detect_language ──► chunk_source     resolve_alpha(query)  (0.3 symbol / 0.5 NL)
   (.gitignore     (ext→lang map)     (tree-sitter         │
    + .cspignore)                      AST, line-          ├─► dense:  Model::encode → cosine kNN
       │                               fallback)           ├─► sparse: tokenize → BM25 get_scores
       ▼                                  │                │        (over-fetch top_k * 5 each)
  Vec<Chunk> ────────┬───────────────────┘                ▼
                     │                              RRF normalize each list  (1/(k+rank), k=60)
       ┌─────────────┴─────────────┐                       ▼
   embed_chunks                enrich_for_bm25      combined = α·rrf_dense + (1-α)·rrf_bm25
   (dense matrix)              ("{content} {stem}           ▼
       │                        {stem} {dir[-3:]}")  rerank (if CODE):
       ▼                        → tokenize → BM25       boost_multi_chunk_files   (wired)
  SelectableBasicBackend        index                  apply_query_boost          (wired)
  (cosine)                                             rerank_top_k               (wired, path penalties + saturation)
                                                              ▼
                                                       top_k SearchResult → ~/.csp/savings.jsonl
```

The full ranking in `ranking::{boosting,penalties}` is now **wired** into `search.rs`
(query-type boosts + path penalties + file saturation), matching the upstream
`search.search` pipeline order (see §4.10).

---

## 3. Module map (semble → Rust)

| Upstream `src/semble/` | Rust | Status | Purpose |
|---|---|---|---|
| `types.py` | `csp/src/types.rs` | ported | `Chunk`, `ContentType`, `CallType` enums; `ChunkDict`/`SearchResultDict` serde |
| `tokens.py` | `csp/src/tokens.rs` | ported | identifier-aware tokenizer (BM25 input) |
| `chunking/core.py` | `csp/src/chunking/core.rs` | ported (real tree-sitter) | node-merge + line-fallback boundary algorithm; `TsNode` bridge |
| `chunking/chunking.py` | `csp/src/chunking/source.rs` | ported | `chunk_source` → `Vec<Chunk>`; char↔line conversion (chunk length 750) |
| `index/file_walker.py` | `csp/src/indexing/file_walker.rs` | ported (`.cspignore`) | gitignore-aware recursive walk (`ignore` crate idioms) |
| `index/files.py` | `csp/src/indexing/files.rs` | ported | ext→language map, content-type sets, file status checks |
| `index/dense.py` | `csp/src/indexing/dense.rs` | ported (real + stub) | `Model` enum, `embed_chunks`, `SelectableBasicBackend` cosine |
| `index/sparse.py` + `index/bm25.py` | `csp/src/indexing/sparse.rs` | ported (incremental) | id-keyed incremental `Bm25Index` (add/remove/doc order, `bm25.json` v2), `enrich_for_bm25`, selector→mask |
| `index/types.py` | `csp/src/indexing/types.rs` | adapted (hash, not mtime) | `FileManifestEntry {hash,start,count}`, `PreviousIndex::try_new` alignment checks, `make_chunk_id` |
| `index/create.py` | `csp/src/indexing/create.rs` | ported (incremental) | build BM25 + dense + chunks + `files` manifest from a path, reusing a `PreviousIndex`'s unchanged files |
| `index/index.py` | `csp/src/indexing/index.rs` | ported | `CspIndex` orchestrator (from_path/from_git/search/find_related/save/load) + `load_or_build_index` |
| `cache.py` | `csp/src/indexing/cache.rs` + `cache_orchestrator.rs` | adapted | content-hash cache at `~/.csp/index/` (ADR-0002), 0700 perms; `try_reuse` + `load_previous_for_incremental` (ADR-0005) |
| `search.py` | `csp/src/search.rs` | ported (ranking wired) | hybrid RRF + alpha blend; trait seams |
| `ranking/weighting.py` | `csp/src/ranking/weighting.rs` | ported | adaptive alpha |
| `ranking/boosting.py` | `csp/src/ranking/boosting.rs` | ported (wired) | query-type detection + definition/stem/embedded boosts |
| `ranking/penalties.py` | `csp/src/ranking/penalties.rs` | ported (wired) | path penalties + file-saturation rerank |
| `stats.py` | `csp/src/stats.rs` | adapted | `~/.csp/savings.jsonl` read/write + report formatting |
| `mcp.py` | `csp/src/mcp.rs` (core) + `csp/src/bin/csp/mcp_server.rs` (rmcp transport) | ported | MCP `search` / `find_related` tools |
| `cli.py` | `csp/src/bin/csp/main.rs` | adapted (clap) | subcommands: search / find-related / index / savings / clear / init / mcp |
| `utils.py` | `csp/src/utils.rs` | ported | git-URL detection, `format_results` (snake_case wire), `resolve_chunk` |
| `installer/` | `csp/src/bin/csp/agents/*.md` (+ `init`) | adapted | agent config templates wired via `init` |
| — | `csp/src/lib.rs` | new | crate root / public re-exports |

---

## 4. Module deep-dives (algorithm + Rust idiom)

### 4.1 `types.rs` — domain model & two serde shapes

- `Chunk { content, file_path, start_line: u32, end_line: u32, language: Option<String> }`
  derives `PartialEq, Eq` (no `Hash` — score maps key by **index**, not by `Chunk`; see §4.10).
- `ContentType { Code, Docs, Config }` and `CallType { Search, FindRelated }` are serde enums;
  `ContentType::as_str()` yields the wire value (`"code"`…).
- **Two distinct dict representations** (the single most important port-time structural choice):
  - `ChunkDict` / `SearchResultDict` — **camelCase** (`filePath, startLine, endLine`; both are
    `#[serde(rename_all = "camelCase")]`), used for **on-disk persistence** (`chunk_to_dict` /
    `chunk_from_dict` / `search_result_to_dict`), matching the camelCase public API.
  - `utils::result_to_dict` / `format_results` — **snake_case** (`file_path, start_line,
    end_line`), the **CLI/MCP wire format** (built with the `json!` macro, mirroring the TS
    `SearchResult.toDict`).
- `chunk_from_dict` returns `Result<Chunk, ChunkFromDictError>` (`thiserror`) instead of throwing.
- `chunk_location(chunk)` → `"{file_path}:{start}-{end}"`.

### 4.2 `tokens.rs` — identifier-aware tokenization

Same contract as semble `tokens.py`:
- token regex `[A-Za-z_][A-Za-z0-9_]*`; camel/Pascal splitter
  `[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+`.
- `split_identifier`: snake → split on `_`; else camel/Pascal split. Returns `[lower]` or
  `[lower, *parts]` (≥2 parts). `HandlerStack → [handlerstack, handler, stack]`.
- `tokenize` flat-maps `split_identifier` over every token match. Original lowercased compound
  is preserved alongside sub-tokens (exact + partial match).

### 4.3 `chunking/` — AST chunking with line fallback

**`core.rs`** (boundary algorithm, byte-based; `RECURSION_DEPTH = 500`, `MIN_CHUNK_SIZE = 50`):
- The merge algorithm is generic over a node trait so tests can drive it with mock nodes; in
  production a **`TsNode` bridge** adapts `tree_sitter::Node` to it.
- `language_for(language)` returns a `tree_sitter::Language` from
  **`tree_sitter_language_pack`** (306 grammars — full upstream parity; semble uses the Python
  package of the same name). Parsers download from GitHub releases on first use and cache on disk;
  unknown language or an offline fetch failure → `None` → line fallback. `is_supported_language`
  uses `has_language` — a **metadata-only** check (no download) — so `chunk_source` gates AST
  chunking before paying for a fetch. 264/265 `EXTENSION_TO_LANGUAGE` names resolve (`wolfram` is
  the sole gap). See [ADR-0004](../decisions/0004-rust-grammar-coverage-language-pack.md) for the
  single-binary ↔ runtime-cache trade-off.
- `_merge_node_inner` (greedy pack), `_merge_adjacent_chunks`, `chunk_lines` fallback — same
  shape as semble. Byte offsets are converted to char offsets for multibyte safety.

**`source.rs`** (`chunk_source`):
- `DESIRED_CHUNK_LENGTH_CHARS = 750` (matches upstream `_DESIRED_CHUNK_LENGTH_CHARS`).
  The value is also recorded in the index manifest (`chunk_size`) so a cache built
  with a different target length is auto-invalidated (see §4.9/§4.14).
- AST chunking is gated by `is_supported_language(lang)` (metadata-only, no download); the
  subsequent `chunk(...)` may still return `None` (e.g. an offline grammar fetch failure),
  falling back to line chunking. Char offsets → 1-indexed line numbers; clamps end to avoid the
  zero-length off-by-one.

### 4.4 `indexing/file_walker.rs` — gitignore-aware walk

- Default-ignored dirs mirror semble (`.git/ node_modules/ dist/ build/ .next/ …`) with
  **`.csp/`** replacing semble's `.semble/`.
- Merges `.gitignore` **and `.cspignore`** per directory; skips symlinks; sorts entries for
  determinism. Files are yielded when the suffix matches the content-type extension set (plus the
  negation-pattern re-include rule from upstream).

### 4.5 `indexing/files.rs` — language detection & file gating

- `EXTENSION_TO_LANGUAGE` — `&[(&str, &str)]` table (~350 entries). `detect_language(name)`
  lowercases the suffix and looks it up. Since §4.3 now resolves grammars through
  `tree_sitter_language_pack` (306 grammars), 264/265 of these language names AST-chunk; only
  `wolfram` (and any extension the pack can't fetch) falls back to line chunking.
- Content-type partition: `DOC_LANGUAGES`, `CONFIG_LANGUAGES`, `DATA_LANGUAGES`; code = all minus
  those. `get_extensions(types, extra)` inverts the map; the **`extra`** param (custom extensions)
  is a small Rust-side API addition.
- File gating: `DEFAULT_MAX_FILE_BYTES = 1_000_000`, overridable per process via
  `CSP_MAX_FILE_BYTES` (`get_max_file_bytes()` — upstream `SEMBLE_MAX_FILE_BYTES`, #252; a
  malformed/non-positive value warns once and falls back). `create_index_from_path` collects the
  paths skipped for size and prints one stderr warning (count + first 5 paths). Empty/whitespace
  and too-new (mtime) files are skipped (`FileStatus`).

### 4.6 `indexing/dense.rs` — Model2Vec embeddings (real + stub)

- `Model` is an **enum**: `Static { inner: Arc<StaticModel>, dim }` (real `model2vec-rs`
  `StaticModel::from_pretrained` + `encode`) | `Stub { dim }` (offline/test, the bit-for-bit TS
  stub via `stub_embed`). `Model::encode` / `Model::dim` dispatch over the variant.
- `load_model` resolves `minishlab/potion-code-16M` (or env override) and **falls back to the
  stub with a stderr warning** if loading fails; `load_model_with` is the DI seam that keeps unit
  tests offline. `make_stub_model(dim)` for tests. `MODEL_CACHE` (`LazyLock<Mutex<HashMap>>`)
  memoizes loads.
- `SelectableBasicBackend` — cosine backend with an optional `selector` (subset of chunk
  indices) for language/path-filtered search; `query()` returns `Result` (errors degrade to
  empty results in the search path, never panic — see §4.10).
- **Status**: per the Rust track, dense + tree-sitter are **no longer stubs** (TD-001 resolved);
  the stub remains only as an offline fallback.

### 4.7 `indexing/sparse.rs` — BM25 + enrichment

- `enrich_for_bm25(chunk)` → `"{content} {stem} {stem} {dir[-3:]}"` — stem repeated twice to
  up-weight path matches; last 3 parent dir components. `selector_to_mask(selector, size)` →
  `Vec<u8>` mask.
- `Bm25Index` ports upstream's own incremental `BM25` class (`index/bm25.py`, #225): documents
  are keyed on a stable chunk id (`"{path}:{slot}"`), `add_document` rejects duplicates,
  `remove_document` drops a document's postings, and `set_doc_order(ids)` fixes the global
  chunk-list order that `get_scores(tokens, weight_mask)` output is aligned to (ids not in the
  corpus score 0). Internally ids are interned to `u32` slots (recycled on removal) and postings
  are `term → {slot → tf}` so a removal is `O(terms in doc)`. `bm25.json` (v2) persists
  `{documents: {id → {term → tf}}, docOrder}`; postings are rebuilt on load, and an order that
  does not describe exactly the persisted documents is rejected. `build(docs)` remains as a
  positional-id convenience for fixtures.

### 4.8 `indexing/create.rs` — index construction

`create_index_from_path(path, options, previous)`: walk → per file: `detect_language`, size
gate, read bytes, **sha256 the bytes**, store path **relative to `display_root`**. If `previous`
has a manifest entry for the path with the same hash, the file's chunks and (already-normalised)
vector rows are **moved** out of the previous index; otherwise the file is decoded, `chunk_source`d,
its old BM25 slots removed and new ones added (`reindex_file`), and its chunks embedded. Paths in
the previous manifest that the walk no longer yields have their postings removed. Finally
`set_doc_order(chunk_ids)` and the rows are wrapped via `SelectableBasicBackend::from_normalized`.
Returns chunks + BM25 + dense + a `files: FileManifest` (`{hash, start, count}` per file; a valid
file that yields no chunks gets `count = 0`). Empty → error. Divergence from upstream: the reuse
key is the content hash, not `mtime_ns` (ADR-0005); the "same vector layout → mutate in place"
special case is unnecessary because rows are moved, never copied.

### 4.9 `indexing/index.rs` — `CspIndex` orchestrator

The public façade (parallels `SembleIndex`):
- `from_path` (= `from_path_with_previous(path, options, None)`), `from_git` (git clone into a
  tempdir; repo-relative chunk paths), `search` (`QueryOptions`), `find_related` (semantic kNN
  on the seed, same-language, excludes seed), `save` / `load_from_disk` (persists chunks + bm25 +
  semantic + `manifest.json` incl. the per-file `files` manifest; `INDEX_SCHEMA_VERSION = 2`;
  load rejects other versions and chunk/vector/BM25-order count mismatches).
- `load_or_build_index` (`LoadOrBuildOptions`) — the cache-aware entry the CLI/MCP use: load from
  `~/.csp/index/<hash>` on a validated hit; on a miss for a local path, seed the rebuild with
  `load_previous_for_incremental` so unchanged files are reused (ADR-0005), then persist.
- Builds file→indices and language→indices maps for selectors and stats.

### 4.10 `search.rs` — hybrid retrieval & fusion

The heart of ranking. `search(query, model, semantic_index, bm25_index, chunks, top_k, options)`:
1. `resolve_alpha(query, options.alpha)`; `rerank = options.rerank.unwrap_or(true)`.
2. **Over-fetch** `candidate_count = top_k * 5` for both signals.
3. `search_semantic` — `Model::encode([query])` → backend kNN → `score = 1 - distance`.
4. `search_bm25` — `tokenize(query)` → `get_scores(mask)` → top-k via `sort_top_k`, drop ≤0.
5. **RRF** (`rrf_scores`): rank each list by raw score desc (`f64::total_cmp`, stable),
   `1/(RRF_K + rank)`, `RRF_K = 60`, rank from 1.
6. Union of indices, **sorted by `start_line`** to neutralize hash-iteration nondeterminism;
   `combined = α·rrf_semantic + (1-α)·rrf_bm25`.
7. If `rerank`: `boost_multi_chunk_files` → `ranking::boosting::apply_query_boost` →
   `ranking::penalties::rerank_top_k(.., penalise_paths = alpha_weight < 1.0)` — the real
   ranking functions, matching the upstream `search.search` order (path penalties apply only
   when BM25 contributes). Else plain sort + truncate.

**Rust idioms / structure**:
- **Trait seams** for testability: `EmbeddingModel`, `VectorBackend`, `SparseBackend`,
  implemented for the concrete `dense::Model`, `SelectableBasicBackend`, `sparse::Bm25Index`.
  Tests inject mocks.
- `Scores = IndexMap<usize, f64>` (see `ranking/mod.rs`) — keyed by **chunk index** into the
  canonical `&[Chunk]`, insertion-ordered. This is the Rust counterpart of the TS
  `Map<Chunk, number>` (object-identity keyed) whose iteration order the upstream relies on for
  tie-breaking. Rust can't hash `Chunk` cheaply, so it indexes instead.
- **Error degradation**: a backend `query` failure prints to stderr and returns empty rather
  than panicking — matters for the long-running MCP server.
- `SearchOptions` struct (`alpha`, `selector`, `rerank`) instead of Python kwargs.

> **TD-002 (resolved)**: `ranking::boosting::apply_query_boost` and
> `ranking::penalties::rerank_top_k` are now wired into `search.rs`, so the full ranking
> (query-type boosts + path penalties + file-saturation decay) runs in the search path.
> The duplicate inline `FILE_SATURATION_THRESHOLD`/`DECAY` stub constants in `search.rs` were
> removed; the canonical definitions live only in `ranking/penalties.rs`.

### 4.11 `ranking/weighting.rs` — adaptive alpha

`resolve_alpha(query, alpha)`: explicit wins; else `ALPHA_SYMBOL = 0.3` (BM25-leaning) for symbol
queries vs `ALPHA_NL = 0.5` for NL, decided by `is_symbol_query`.

### 4.12 `ranking/boosting.rs` — query-type detection & boosts (wired)

Ported faithfully (`LazyLock<Regex>` for the static patterns, `RefCell<HashMap>` LRU for
`definition_pattern` cache):
- `SYMBOL_QUERY_RE` / `EMBEDDED_SYMBOL_RE` — symbol vs NL classification.
- `apply_query_boost` (wired into `search.rs`): symbol → `_boost_symbol_definitions` (definition regex per
  keyword set: `class def fn func struct enum trait type …` case-sensitive + SQL DDL
  case-insensitive; `DEFINITION_BOOST_MULTIPLIER = 3.0`, ×1.5 on stem match); NL →
  `_boost_stem_matches` (`STEM_BOOST_MULTIPLIER = 1.0`, ≥0.10 ratio, prefix-match morphology) +
  `_boost_embedded_symbols` (`EMBEDDED_SYMBOL_BOOST_SCALE = 0.5`, `EMBEDDED_STEM_MIN_LEN = 4`).
- `boost_multi_chunk_files` (**wired** into search): top chunk per file boosted by
  `max_score * FILE_COHERENCE_BOOST_FRAC` (=0.2) × (file score sum / max file sum).

### 4.13 `ranking/penalties.rs` — path penalties & saturation rerank (wired)

`rerank_top_k(scores, chunks, top_k, penalise_paths)` ported and wired into `search.rs`:
- Path penalties (multiplicative): test files/dirs `STRONG_PENALTY = 0.3`; compat/legacy +
  examples/docs `0.3`; re-export barrels (`__init__.py`, `package-info.java`)
  `MODERATE_PENALTY = 0.5`; `.d.ts` `MILD_PENALTY = 0.7`.
- File-saturation decay: beyond `FILE_SATURATION_THRESHOLD = 1` per file, ×`FILE_SATURATION_DECAY
  = 0.5 ^ excess`; greedy with safe early-exit. Penalties apply only when `alpha_weight < 1.0`.

### 4.14 `indexing/cache.rs` — index cache (ADR-0002)

- Cache home `$HOME/.csp` (override via `CacheLocation`), index root `<home>/index`, per-source
  leaf `<home>/index/<sha256-key>`. `ensure_cache_dir` creates the chain with **0700** perms
  (NFR-003), tightening pre-existing dirs on Unix.
- **Cache key source identity** (`normalize_source`): local paths are made absolute with
  `std::path::absolute` and then path-normalized before hashing, so `.`, `./r/../r`, and
  `/abs/r` share one leaf and two repos searched with the default `.` no longer collide
  (#100; upstream `cache_key` uses `Path.resolve()`). It is **not** byte-identical to the
  manifest `sourceId`: `from_path` records bare `std::path::absolute(root)`, which keeps the
  `..` segments this collapses, so anything comparing the two must normalize both sides.
  The `\`→`/` rewrite is Windows-only — on Unix a backslash is an ordinary filename byte, and
  folding it would collide `/repos/a\b` with `/repos/a/b`. Git URLs stay verbatim.
- `clear_index_cache` removes only the index dir — never the `~/.csp` home (which also holds
  `savings.jsonl`).
- `clear_orphan_indexes` (← upstream `cli.py::_clear_orphans`, #243) removes per-source leaves
  whose manifest `sourceId` is a local path that is now `NotFound`. Same root guard as
  `clear_index_cache`; a leaf is considered only when its directory name has the cache-key
  shape (32 lowercase hex, mirroring upstream's `_SHA_256_REGEX`), so stray directories are
  never swept. **Drift note:** upstream guards with `cache_key(root_path) == dir name`; csp
  cannot, because `resolve_cache_dir` folds `CacheLocation::git_ref` into the key while
  `IndexManifest` has no `ref` field — a local entry built with `--ref` would never re-derive
  its own key (see `orphans_removes_local_entry_built_with_a_ref`). Since #100 the source
  identities *do* agree for a plain relative source; the ref (and `from_path` keeping `..`
  segments the key collapses) is what still blocks re-derivation. Git leaves are excluded by
  `is_git_url(sourceId)` (`from_git` re-roots the manifest to the URL, unlike upstream, which
  stores the temp clone dir). Relative `sourceId`s are skipped (they would resolve against the
  caller's cwd), and only a `NotFound` counts as gone — a source that errors for another
  reason (unreadable parent, stale network handle) keeps its cache; an unmounted volume's path
  is simply absent and is swept like upstream. Because `from_path` records an unnormalized
  absolute path, `source_is_gone` requires *both* the recorded spelling and its normalized form
  to be `NotFound`: the kernel resolves `..` component-by-component, so `csp search "q" ../b`
  records `/cwd/../b` and deleting only `/cwd` would otherwise sweep the live `/b` index.
  Traversal errors in the index root fail the sweep before anything is deleted; a per-entry
  removal failure is collected in `OrphanSweep::errors` and the sweep continues, so partial
  progress is still reported and one undeletable leaf cannot permanently block the rest.
  Exposed as `csp clear orphans`; not part of `clear all` (matches upstream).
- **Cache validity** (`try_reuse`): a cached index is reused only when the manifest's
  `chunk_size` equals the current `DESIRED_CHUNK_LENGTH_CHARS` (a manifest predating the field
  → `None` → rebuild) **and**, for local sources, the live source-file content hash matches.
  This mirrors upstream `_metadata_matches`, which gained a `chunk_size` check so the 1500→750
  change auto-invalidates stale caches. The same `manifest_compatible` check (schema version,
  chunk size, model id, model kind) gates `load_previous_for_incremental`, which then loads
  chunks + vectors + BM25 and runs `PreviousIndex::try_new` alignment checks (counts agree,
  manifest ranges tile the chunk list, chunk paths match their range, BM25 order == manifest-
  derived ids). Any failure → `None` → full rebuild (mirrors upstream fail-closed behaviour).
- **Divergence from upstream**: semble uses the OS cache dir (`~/Library/Caches/semble`, XDG,
  `%LOCALAPPDATA%`) + `SEMBLE_CACHE_LOCATION`; csp fixes a global `~/.csp/index/` per ADR-0002.

### 4.15 `stats.rs` — token-savings telemetry

- Appends JSONL `{ts, call, results, snippet_chars, file_chars}` to `~/.csp/savings.jsonl`
  (`now_secs`, `default_stats_file`). `format_savings_report` renders the colored ASCII report
  (Total saved, efficiency bar, By Period; By Call Type gated behind `--verbose`). `clear_savings`.
- **Divergence**: fixed `~/.csp/savings.jsonl` (not the OS cache dir); no `flock` (sub-4KB
  appends are atomic on POSIX); header is "Csp".
- **Divergence** (issue #90): `file_chars` sizes come from `indexing::file_sizes::FileSizes` —
  a local source root is read lazily per returned result with a memo (misses memoized too), where
  upstream `_compute_file_sizes` runs eagerly over every indexed file in `SembleIndex.__init__`.
  Git sources still capture eagerly at clone time (the temp checkout is gone by search time).
  Because the read now happens inside a live search, it is bounded by `get_max_file_bytes()` (the
  same ceiling the indexer applies) — upstream, running at construction time, has no such bound.
  Decoding matches upstream `read_file_text` (`errors="replace"`) and the csp indexer
  (`String::from_utf8_lossy`), so a non-UTF-8 file that got indexed still gets sized.

### 4.16 MCP — `csp/src/mcp.rs` (core) + `csp/src/bin/csp/mcp_server.rs` (rmcp transport)

Clean two-layer split:
- **`csp::mcp`** (lib) — the unit-tested tool **core**: `search` / `find_related` handler logic,
  in-process LRU `IndexCache` (`CACHE_MAX_SIZE = 10`, `Arc<CspIndex>` so indexes are `Send`
  across tasks), `_get_index` with git-transport guards.
- **Per-call `content`** (upstream #247): both tools take `content: Option<ContentSelection>`
  (`code | docs | config | all`); `resolve_content_selection` maps `None` → the server's
  `--content` default and `all` → every `ContentType` (upstream `_resolve_content_selection`).
  `IndexCache` is keyed by `CacheKey { source, content }` where `content` is normalized to enum
  order and de-duplicated (upstream `_CacheKey = (source_key, tuple[ContentType, ...])`), so one
  repo searched as `code` and as `docs` holds two independent session entries; `get` / `evict`
  and fingerprint revalidation all take the content slice. No on-disk change was needed —
  `indexing/cache.rs::resolve_cache_dir` already hashes `sourceId + content + ref` (upstream
  instead added `index-<scope>` sibling dirs in `cache.py`).
- **`csp` bin `mcp_server`** (bin) — **rmcp 1.7** stdio wiring: `CspMcpServer` with
  `#[tool_router]` + `#[tool]` async `search`/`find_related`, `#[tool_handler(router =
  self.tool_router)]` (routes through the stored field; the default `Self::tool_router()` would
  rebuild per call and trip clippy `dead_code`). `run_mcp(path, ref, content)` serves on a tokio
  runtime with `content` as the per-call default. Verified on the wire (initialize / tools/list /
  tools/call).

### 4.17 `csp/src/bin/csp/main.rs` — CLI (clap)

- `#[derive(Parser)]` with a `Command` `#[derive(Subcommand)]` enum: **search**, **find-related**,
  **index** (build + persist a standalone index), **savings**, **clear** (`all|index|savings|orphans`),
  **init** (write an agent file), **mcp** (run the stdio server).
- `search` / `find-related` route through `load_or_build_index` (or an explicit `--index` via
  `LoadOptions`). Output is the snake_case wire JSON (`utils::format_results`).
- **Divergence from upstream**: csp keeps **`init`** (not `install`/`uninstall`), exposes the MCP
  server under an explicit **`mcp`** subcommand (semble starts it from the bare binary), and adds
  `index` / `clear`.

### 4.18 `utils.rs` — helpers

- `is_git_url` (scheme prefixes + scp-style), `resolve_chunk(chunks, file_path, line) ->
  Option<&Chunk>` (interior match preferred, boundary fallback; `\\`/`/` separators compared
  as equal on both sides — upstream #244), `result_to_dict` /
  `format_results` (snake_case wire dict). Model name resolution honors the env override.

---

## 5. Load-bearing constants (semble vs Rust port)

| Constant | semble | Rust | Location |
|---|---|---|---|
| RRF k | `60` | `60` | `search.rs RRF_K` |
| α symbol / NL | `0.3` / `0.5` | `0.3` / `0.5` | `ranking/weighting.rs` |
| candidate over-fetch | `top_k * 5` | `top_k * 5` | `search.rs` |
| desired chunk length | `750` | `750` | `chunking/source.rs` |
| min chunk size | `50` | `50` | `chunking/core.rs` |
| recursion depth | `500` | `500` | `chunking/core.rs` |
| definition boost × | `3.0` | `3.0` | `ranking/boosting.rs` |
| embedded-symbol scale | `0.5` | `0.5` | `ranking/boosting.rs` |
| embedded stem min len | `4` | `4` | `ranking/boosting.rs` |
| stem boost × | `1.0` | `1.0` | `ranking/boosting.rs` |
| file-coherence frac | `0.2` | `0.2` | `ranking/boosting.rs` |
| strong / moderate / mild penalty | `0.3` / `0.5` / `0.7` | same | `ranking/penalties.rs` |
| file saturation threshold / decay | `1` / `0.5` | `1` / `0.5` | `ranking/penalties.rs` |
| max file bytes | `1_000_000` | `1_000_000` | `index/files.py` / `indexing/create.rs` |
| default model | `minishlab/potion-code-16M-v2` (semble#219) | same (real + stub); cache keyed on model_id | `utils.py` / `indexing/dense.rs` |
| MCP in-mem LRU | `10` | `10` | `mcp.py` / `csp::mcp` |
| cache dir mode | — | `0o700` | `indexing/cache.rs` |

---

## 6. Divergences & drift

### 6.1 Intentional adaptations (Rust port by design)

1. **Score maps keyed by index** — `Scores = IndexMap<usize, f64>` vs Python/TS `dict/Map`
   keyed by the `Chunk` object. Same semantics, different key type.
2. **Trait seams** — `EmbeddingModel`/`VectorBackend`/`SparseBackend` for DI/testability.
3. **`Model` enum** (real `model2vec-rs` `Static` + offline `Stub`), graceful stub fallback.
4. **Two serde shapes** — camelCase `ChunkDict`/`SearchResultDict` (disk persistence) vs
   snake_case `utils::result_to_dict`/`format_results` (CLI/MCP wire).
5. **Error handling** — `Result` + `thiserror`; backend errors degrade instead of panicking.
6. **MCP split** — testable core in `csp::mcp`, rmcp 1.7 transport in the `csp` bin `mcp_server`.
7. **Storage** — fixed `~/.csp/index/` (0700) + `~/.csp/savings.jsonl` (ADR-0002), not the OS
   cache dir / `SEMBLE_CACHE_LOCATION`. `.cspignore` (not `.sembleignore`).
8. **CLI** — clap; `init` (not `install`/`uninstall`); explicit `mcp` subcommand; adds `index`.
9. **Incremental reindex keyed on per-file content hash** (ADR-0005) — upstream #225 records
   `mtime_ns` per file; csp records the sha256 of the file bytes so the per-file decision uses
   the same oracle as the whole-tree `contentHash` fast path (ADR-0002).
10. **BM25 scoring unchanged** — csp keeps the Lucene `(k1+1)` numerator and de-duplicates
    query terms; upstream's own `BM25` (#225) dropped `(k1+1)` and multiplies each term's
    contribution by the query term frequency. The `(k1+1)` factor is a single global constant,
    so dropping it is rank-neutral. The query-tf factor is **not** — it reweights terms against
    one another whenever a query tokenises to a repeated token, which the identifier-aware
    tokenizer makes common (`getUserById getUser` repeats `get` and `user`). Ranks can
    therefore differ from upstream: for query tokens `[a, a, b]` with per-term contributions
    `a → 1.0` (doc1) and `b → 1.8` (doc2), upstream scores doc1 `2.0` > doc2 `1.8` while csp
    scores doc1 `1.0` < doc2 `1.8`. This is a **live parity gap**, not a rank-neutral
    adaptation, and it predates #225; it is tracked here rather than fixed in the incremental
    port.

### 6.2 Open stubs & gaps (verify before claiming runtime parity)

- ~~**TD-002 — ranking not wired**~~ — **closed** by [#37](https://github.com/pleaseai/code-search/pull/37).
  `search.rs` now calls the real `ranking::boosting::apply_query_boost` +
  `ranking::penalties::rerank_top_k` (`penalise_paths = alpha_weight < 1.0`), replacing the
  identity/saturation stubs. Search-ranking is wired end-to-end (parity remains fixture-level
  until the dense/BM25 backends are validated against upstream).
- ~~**Curated tree-sitter set**~~ — **closed** ([ADR-0004](../decisions/0004-rust-grammar-coverage-language-pack.md),
  [#38](https://github.com/pleaseai/code-search/issues/38)). `language_for` now resolves through
  `tree_sitter_language_pack` (306 grammars, full upstream parity; 264/265 `EXTENSION_TO_LANGUAGE`
  names AST-chunk, `wolfram` excepted). Trade-off recorded in ADR-0004: parsers download on first
  use and cache on disk, so AST chunking is no longer fully offline/self-contained — it degrades
  gracefully to line chunking when offline, exactly as an unsupported language already did.

### 6.3 Upstream drift since the review baseline (`eacbe43` → `136b6f7`)

- **Chunk length 750 (reconciled)** — the Rust port now uses `DESIRED_CHUNK_LENGTH_CHARS = 750`
  (was 1500), matching upstream `chunking/chunking.py`. The value is recorded in the index
  manifest as `chunk_size` and validated in `try_reuse`, so the change auto-invalidates stale
  caches (mirrors upstream's added metadata field + cache check). The TS source still uses 1500,
  but per the current direction Python upstream — not TS — is the source of truth.
- **Partial (incremental) reindexing (#225, `204ae4e`) — ported** ([#84](https://github.com/pleaseai/code-search/issues/84),
  ADR-0005): per-file `files` manifest, id-keyed incremental `Bm25Index`, `PreviousIndex`
  reuse in `create_index_from_path`, `load_previous_for_incremental` in the orchestrator,
  `INDEX_SCHEMA_VERSION` 1 → 2. See §4.7 / §4.8 / §4.9 / §4.14 and §6.1 items 9–10 for the
  two intentional deviations (hash vs mtime, scoring scale).

---

## 7. How to refresh this analysis

1. Update the upstream checkout and diff from the recorded baseline:
   `git -C <semble> log 136b6f7..main --oneline`.
2. Quality gate the Rust side: `cargo fmt --all && cargo clippy --all-targets --all-features --
   -D warnings && cargo test --workspace`.
3. Re-read any changed module and update the matching §4 section + §5 constants table.
4. When a stub gets wired (TD-002) or grammars are added, move the item out of §6.2 and update
   §4.10 / §4.3. Bump the baseline in `index.md` and this file's header.
5. Cross-check against the `upstream-semble-sync-baseline` and `rust-rewrite-track-status`
   memories and `CLAUDE.md` (Rust rewrite section).

---

*Related: [`ARCHITECTURE.md`](../../../ARCHITECTURE.md) (target architecture),
[ADR-0001](../decisions/0001-native-tree-sitter.md) (tree-sitter bindings),
[ADR-0002](../decisions/0002-index-storage-cache-model.md) (cache model),
[ADR-0003](../decisions/0003-rewrite-in-rust.md) (Rust rewrite),
[`../knowledge/tech-stack.md`](../knowledge/tech-stack.md).*
