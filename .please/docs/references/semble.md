# Reference Analysis — MinishLab/semble → Rust port (`crates/csp`)

> Module-by-module analysis of [MinishLab/semble](https://github.com/MinishLab/semble) (the
> Python original) mapped to the **Rust port** under `crates/csp` (library) and `crates/csp-cli`
> (`csp` binary), introduced by [ADR-0003](../decisions/0003-rewrite-in-rust.md). Each section
> captures the load-bearing algorithm + its constants, the Rust-specific structure/idioms, and
> where the port diverges.
>
> **Analyzed at**: upstream semble `136b6f7` (2026-06-18); Rust port at repo `2f2baa2`
> (PR #34 "Rust rewrite foundation"). **Parity oracle**: the TS `src/` test suite reused as
> golden fixtures — Rust reproduces the TS module behavior bit-for-bit, so "parity" is
> *fixture-level*, not full-runtime. The TS `src/` stays the source of truth until Rust reaches
> parity (per ADR-0003).
> **Upstream layout**: Python `src/semble/`. **Port layout**: `crates/csp/src/` (lib) +
> `crates/csp-cli/src/` (bin).

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
  SelectableBasicBackend        index                  apply_query_boost          (⚠ identity stub)
  (cosine)                                             rerank_top_k               (⚠ saturation-only stub)
                                                              ▼
                                                       top_k SearchResult → ~/.csp/savings.jsonl
```

⚠ = TD-002: the full ranking lives in `ranking::{boosting,penalties}` but is **not yet wired**
into `search.rs`, mirroring the TS source's current state (see §4.10/§6).

---

## 3. Module map (semble → Rust)

| Upstream `src/semble/` | Rust | Status | Purpose |
|---|---|---|---|
| `types.py` | `csp/src/types.rs` | ported | `Chunk`, `ContentType`, `CallType` enums; `ChunkDict`/`SearchResultDict` serde |
| `tokens.py` | `csp/src/tokens.rs` | ported | identifier-aware tokenizer (BM25 input) |
| `chunking/core.py` | `csp/src/chunking/core.rs` | ported (real tree-sitter) | node-merge + line-fallback boundary algorithm; `TsNode` bridge |
| `chunking/chunking.py` | `csp/src/chunking/source.rs` | ported (⚠ param drift) | `chunk_source` → `Vec<Chunk>`; char↔line conversion |
| `index/file_walker.py` | `csp/src/indexing/file_walker.rs` | ported (`.cspignore`) | gitignore-aware recursive walk (`ignore` crate idioms) |
| `index/files.py` | `csp/src/indexing/files.rs` | ported | ext→language map, content-type sets, file status checks |
| `index/dense.py` | `csp/src/indexing/dense.rs` | ported (real + stub) | `Model` enum, `embed_chunks`, `SelectableBasicBackend` cosine |
| `index/sparse.py` | `csp/src/indexing/sparse.rs` | ported | `Bm25Index`, `enrich_for_bm25`, selector→mask |
| `index/create.py` | `csp/src/indexing/create.rs` | ported | build BM25 + dense + chunks from a path |
| `index/index.py` | `csp/src/indexing/index.rs` | ported | `CspIndex` orchestrator (from_path/from_git/search/find_related/save/load) + `load_or_build_index` |
| `cache.py` | `csp/src/indexing/cache.rs` | adapted | content-hash cache at `~/.csp/index/` (ADR-0002), 0700 perms |
| `search.py` | `csp/src/search.rs` | ported (⚠ ranking stub) | hybrid RRF + alpha blend; trait seams |
| `ranking/weighting.py` | `csp/src/ranking/weighting.rs` | ported | adaptive alpha |
| `ranking/boosting.py` | `csp/src/ranking/boosting.rs` | ported (boost_multi wired; others unwired) | query-type detection + definition/stem/embedded boosts |
| `ranking/penalties.py` | `csp/src/ranking/penalties.rs` | ported (unwired) | path penalties + file-saturation rerank |
| `stats.py` | `csp/src/stats.rs` | adapted | `~/.csp/savings.jsonl` read/write + report formatting |
| `mcp.py` | `csp/src/mcp.rs` (core) + `csp-cli/src/mcp_server.rs` (rmcp transport) | ported | MCP `search` / `find_related` tools |
| `cli.py` | `csp-cli/src/main.rs` | adapted (clap) | subcommands: search / find-related / index / savings / clear / init / mcp |
| `utils.py` | `csp/src/utils.rs` | ported | git-URL detection, `format_results` (snake_case wire), `resolve_chunk` |
| `installer/` | `csp-cli/agents/*.md` (+ `init`) | adapted | agent config templates wired via `init` |
| — | `csp/src/lib.rs` | new | crate root / public re-exports |

---

## 4. Module deep-dives (algorithm + Rust idiom)

### 4.1 `types.rs` — domain model & two serde shapes

- `Chunk { content, file_path, start_line: u32, end_line: u32, language: Option<String> }`
  derives `PartialEq, Eq` (no `Hash` — score maps key by **index**, not by `Chunk`; see §4.10).
- `ContentType { Code, Docs, Config }` and `CallType { Search, FindRelated }` are serde enums;
  `ContentType::as_str()` yields the wire value (`"code"`…).
- **Two distinct dict representations** (the single most important port-time structural choice):
  - `ChunkDict` — **camelCase** (`filePath, startLine, endLine`), used for **on-disk
    persistence** (`chunk_to_dict` / `chunk_from_dict`), matching the camelCase public API.
  - `SearchResultDict` — **snake_case** (`file_path, start_line…`), the **CLI/MCP wire format**
    (`search_result_to_dict`, mirroring the TS `SearchResult.toDict`).
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
- `language_for(language)` returns a statically-linked `tree_sitter::Language` for a **curated
  grammar set**: rust, python, javascript, typescript, tsx, go, java, c, cpp, ruby, json, bash,
  html, css. Unsupported languages → `None` → line fallback. ⚠ This is **narrower than upstream**,
  which uses `tree_sitter_language_pack` (≈all languages); see §6.
- `_merge_node_inner` (greedy pack), `_merge_adjacent_chunks`, `chunk_lines` fallback — same
  shape as semble. Byte offsets are converted to char offsets for multibyte safety.

**`source.rs`** (`chunk_source`):
- `DESIRED_CHUNK_LENGTH_CHARS = 1500` (⚠ upstream is now **750** — see §6).
- AST chunking when `language_for(lang).is_some()`, else line fallback. Char offsets → 1-indexed
  line numbers; clamps end to avoid the zero-length off-by-one.

### 4.4 `indexing/file_walker.rs` — gitignore-aware walk

- Default-ignored dirs mirror semble (`.git/ node_modules/ dist/ build/ .next/ …`) with
  **`.csp/`** replacing semble's `.semble/`.
- Merges `.gitignore` **and `.cspignore`** per directory; skips symlinks; sorts entries for
  determinism. Files are yielded when the suffix matches the content-type extension set (plus the
  negation-pattern re-include rule from upstream).

### 4.5 `indexing/files.rs` — language detection & file gating

- `EXTENSION_TO_LANGUAGE` — `&[(&str, &str)]` table (~350 entries). `detect_language(name)`
  lowercases the suffix and looks it up. Note: this recognizes far more extensions than the
  curated tree-sitter set in §4.3 — recognized-but-unparsed languages still get **walked and
  line-chunked**.
- Content-type partition: `DOC_LANGUAGES`, `CONFIG_LANGUAGES`, `DATA_LANGUAGES`; code = all minus
  those. `get_extensions(types, extra)` inverts the map; the **`extra`** param (custom extensions)
  is a small Rust-side API addition.
- File gating: `MAX_FILE_BYTES = 1_000_000` (in `create.rs`); empty/whitespace and too-new
  (mtime) files are skipped (`FileStatus`).

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
  up-weight path matches; last 3 parent dir components. `Bm25Index` ports the BM25 scoring
  (`get_scores(tokens, weight_mask)`). `selector_to_mask(selector, size)` → `Vec<u8>` mask.

### 4.8 `indexing/create.rs` — index construction

`create_index_from_path`: walk → per file: `detect_language`, size/empty gate, read text, store
path **relative to `display_root`**, `chunk_source`. Then `embed_chunks` → dense matrix; build
`Bm25Index` over `tokenize(enrich_for_bm25(chunk))`; wrap dense in `SelectableBasicBackend`.
Empty → error.

### 4.9 `indexing/index.rs` — `CspIndex` orchestrator

The public façade (parallels `SembleIndex`):
- `from_path`, `from_git` (git clone into a tempdir; repo-relative chunk paths), `search`
  (`QueryOptions`), `find_related` (semantic kNN on the seed, same-language, excludes seed),
  `save` / `load_from_disk` (persists chunks + bm25 + semantic + metadata).
- `load_or_build_index` (`LoadOrBuildOptions`) — the cache-aware entry the CLI/MCP use: load from
  `~/.csp/index/<hash>` on a validated hit, else build and persist.
- Builds file→indices and language→indices maps for selectors and stats.

### 4.10 `search.rs` — hybrid retrieval & fusion (with TD-002 stub)

The heart of ranking. `search(query, model, semantic_index, bm25_index, chunks, top_k, options)`:
1. `resolve_alpha(query, options.alpha)`; `rerank = options.rerank.unwrap_or(true)`.
2. **Over-fetch** `candidate_count = top_k * 5` for both signals.
3. `search_semantic` — `Model::encode([query])` → backend kNN → `score = 1 - distance`.
4. `search_bm25` — `tokenize(query)` → `get_scores(mask)` → top-k via `sort_top_k`, drop ≤0.
5. **RRF** (`rrf_scores`): rank each list by raw score desc (`f64::total_cmp`, stable),
   `1/(RRF_K + rank)`, `RRF_K = 60`, rank from 1.
6. Union of indices, **sorted by `start_line`** to neutralize hash-iteration nondeterminism;
   `combined = α·rrf_semantic + (1-α)·rrf_bm25`.
7. If `rerank`: `boost_multi_chunk_files` (**wired**, shared impl) → `apply_query_boost_identity`
   (⚠ stub) → `rerank_top_k_saturation` (⚠ stub: file-saturation decay only, path penalties
   **not applied**, `penalise_paths` ignored). Else plain sort + truncate.

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

> **TD-002**: `ranking::boosting::apply_query_boost` and `ranking::penalties::rerank_top_k` are
> fully ported (with tests) but **not wired** into `search.rs` — exactly as in the TS source,
> which still uses the inline stubs (`TODO(integration)`). So search-ranking parity is
> fixture-level. `FILE_SATURATION_THRESHOLD`/`DECAY` are therefore defined **twice** (the inline
> stub in `search.rs` and the real one in `ranking/penalties.rs`).

### 4.11 `ranking/weighting.rs` — adaptive alpha

`resolve_alpha(query, alpha)`: explicit wins; else `ALPHA_SYMBOL = 0.3` (BM25-leaning) for symbol
queries vs `ALPHA_NL = 0.5` for NL, decided by `is_symbol_query`.

### 4.12 `ranking/boosting.rs` — query-type detection & boosts (mostly unwired)

Ported faithfully (`LazyLock<Regex>` for the static patterns, `RefCell<HashMap>` LRU for
`definition_pattern` cache):
- `SYMBOL_QUERY_RE` / `EMBEDDED_SYMBOL_RE` — symbol vs NL classification.
- `apply_query_boost` (unwired): symbol → `_boost_symbol_definitions` (definition regex per
  keyword set: `class def fn func struct enum trait type …` case-sensitive + SQL DDL
  case-insensitive; `DEFINITION_BOOST_MULTIPLIER = 3.0`, ×1.5 on stem match); NL →
  `_boost_stem_matches` (`STEM_BOOST_MULTIPLIER = 1.0`, ≥0.10 ratio, prefix-match morphology) +
  `_boost_embedded_symbols` (`EMBEDDED_SYMBOL_BOOST_SCALE = 0.5`, `EMBEDDED_STEM_MIN_LEN = 4`).
- `boost_multi_chunk_files` (**wired** into search): top chunk per file boosted by
  `max_score * FILE_COHERENCE_BOOST_FRAC` (=0.2) × (file score sum / max file sum).

### 4.13 `ranking/penalties.rs` — path penalties & saturation rerank (unwired)

`rerank_top_k(scores, chunks, top_k, penalise_paths)` ported but unwired:
- Path penalties (multiplicative): test files/dirs `STRONG_PENALTY = 0.3`; compat/legacy +
  examples/docs `0.3`; re-export barrels (`__init__.py`, `package-info.java`)
  `MODERATE_PENALTY = 0.5`; `.d.ts` `MILD_PENALTY = 0.7`.
- File-saturation decay: beyond `FILE_SATURATION_THRESHOLD = 1` per file, ×`FILE_SATURATION_DECAY
  = 0.5 ^ excess`; greedy with safe early-exit. Penalties apply only when `alpha_weight < 1.0`.

### 4.14 `indexing/cache.rs` — index cache (ADR-0002)

- Cache home `$HOME/.csp` (override via `CacheLocation`), index root `<home>/index`, per-source
  leaf `<home>/index/<sha256-key>`. `ensure_cache_dir` creates the chain with **0700** perms
  (NFR-003), tightening pre-existing dirs on Unix.
- `clear_index_cache` removes only the index dir — never the `~/.csp` home (which also holds
  `savings.jsonl`).
- **Divergence from upstream**: semble uses the OS cache dir (`~/Library/Caches/semble`, XDG,
  `%LOCALAPPDATA%`) + `SEMBLE_CACHE_LOCATION`; csp fixes a global `~/.csp/index/` per ADR-0002.

### 4.15 `stats.rs` — token-savings telemetry

- Appends JSONL `{ts, call, results, snippet_chars, file_chars}` to `~/.csp/savings.jsonl`
  (`now_secs`, `default_stats_file`). `format_savings_report` renders the colored ASCII report
  (Total saved, efficiency bar, By Period; By Call Type gated behind `--verbose`). `clear_savings`.
- **Divergence**: fixed `~/.csp/savings.jsonl` (not the OS cache dir); no `flock` (sub-4KB
  appends are atomic on POSIX); header is "Csp".

### 4.16 MCP — `csp/src/mcp.rs` (core) + `csp-cli/src/mcp_server.rs` (rmcp transport)

Clean two-layer split:
- **`csp::mcp`** (lib) — the unit-tested tool **core**: `search` / `find_related` handler logic,
  in-process LRU `IndexCache` (`CACHE_MAX_SIZE = 10`, `Arc<CspIndex>` so indexes are `Send`
  across tasks), `_get_index` with git-transport guards.
- **`csp-cli::mcp_server`** (bin) — **rmcp 1.7** stdio wiring: `CspMcpServer` with
  `#[tool_router]` + `#[tool]` async `search`/`find_related`, `#[tool_handler(router =
  self.tool_router)]` (routes through the stored field; the default `Self::tool_router()` would
  rebuild per call and trip clippy `dead_code`). `run_mcp(path, ref, content)` serves on a tokio
  runtime. Verified on the wire (initialize / tools/list / tools/call).

### 4.17 `csp-cli/src/main.rs` — CLI (clap)

- `#[derive(Parser)]` with a `Command` `#[derive(Subcommand)]` enum: **search**, **find-related**,
  **index** (build + persist a standalone index), **savings**, **clear** (`all|index|savings`),
  **init** (write an agent file), **mcp** (run the stdio server).
- `search` / `find-related` route through `load_or_build_index` (or an explicit `--index` via
  `LoadOptions`). Output is the snake_case wire JSON (`utils::format_results`).
- **Divergence from upstream**: csp keeps **`init`** (not `install`/`uninstall`), exposes the MCP
  server under an explicit **`mcp`** subcommand (semble starts it from the bare binary), and adds
  `index` / `clear`.

### 4.18 `utils.rs` — helpers

- `is_git_url` (scheme prefixes + scp-style), `resolve_chunk(chunks, file_path, line) ->
  Option<&Chunk>` (interior match preferred, boundary fallback), `result_to_dict` /
  `format_results` (snake_case wire dict). Model name resolution honors the env override.

---

## 5. Load-bearing constants (semble vs Rust port)

| Constant | semble | Rust | Location |
|---|---|---|---|
| RRF k | `60` | `60` | `search.rs RRF_K` |
| α symbol / NL | `0.3` / `0.5` | `0.3` / `0.5` | `ranking/weighting.rs` |
| candidate over-fetch | `top_k * 5` | `top_k * 5` | `search.rs` |
| desired chunk length | **`750`** | **`1500`** ⚠ | `chunking/source.rs` |
| min chunk size | `50` | `50` | `chunking/core.rs` |
| recursion depth | `500` | `500` | `chunking/core.rs` |
| definition boost × | `3.0` | `3.0` | `ranking/boosting.rs` |
| embedded-symbol scale | `0.5` | `0.5` | `ranking/boosting.rs` |
| embedded stem min len | `4` | `4` | `ranking/boosting.rs` |
| stem boost × | `1.0` | `1.0` | `ranking/boosting.rs` |
| file-coherence frac | `0.2` | `0.2` | `ranking/boosting.rs` |
| strong / moderate / mild penalty | `0.3` / `0.5` / `0.7` | same | `ranking/penalties.rs` |
| file saturation threshold / decay | `1` / `0.5` | `1` / `0.5` (defined **twice**, see §4.10) | `search.rs` + `ranking/penalties.rs` |
| max file bytes | `1_000_000` | `1_000_000` | `index/files.py` / `indexing/create.rs` |
| default model | `minishlab/potion-code-16M` | same (real + stub) | `utils.py` / `indexing/dense.rs` |
| MCP in-mem LRU | `10` | `10` | `mcp.py` / `csp::mcp` |
| cache dir mode | — | `0o700` | `indexing/cache.rs` |

---

## 6. Divergences & drift

### 6.1 Intentional adaptations (Rust port by design)

1. **Score maps keyed by index** — `Scores = IndexMap<usize, f64>` vs Python/TS `dict/Map`
   keyed by the `Chunk` object. Same semantics, different key type.
2. **Trait seams** — `EmbeddingModel`/`VectorBackend`/`SparseBackend` for DI/testability.
3. **`Model` enum** (real `model2vec-rs` `Static` + offline `Stub`), graceful stub fallback.
4. **Two serde shapes** — camelCase `ChunkDict` (disk) vs snake_case `SearchResultDict` (wire).
5. **Error handling** — `Result` + `thiserror`; backend errors degrade instead of panicking.
6. **MCP split** — testable core in `csp::mcp`, rmcp 1.7 transport in `csp-cli::mcp_server`.
7. **Storage** — fixed `~/.csp/index/` (0700) + `~/.csp/savings.jsonl` (ADR-0002), not the OS
   cache dir / `SEMBLE_CACHE_LOCATION`. `.cspignore` (not `.sembleignore`).
8. **CLI** — clap; `init` (not `install`/`uninstall`); explicit `mcp` subcommand; adds `index`.

### 6.2 Open stubs & gaps (verify before claiming runtime parity)

- **TD-002 — ranking not wired**: `search.rs` uses `apply_query_boost_identity` +
  `rerank_top_k_saturation`; the real `ranking::{boosting::apply_query_boost,
  penalties::rerank_top_k}` are ported but unwired (matches the TS source). Search-ranking parity
  is fixture-level only. Saturation constants are duplicated as a result.
- **Curated tree-sitter set** — only ~14 grammars are statically linked (`language_for`); upstream
  uses `tree_sitter_language_pack` (≈all languages). Languages outside the curated set are
  recognized by the extension map but **line-chunked**, not AST-chunked. This is a real behavioral
  narrowing vs upstream.

### 6.3 Upstream drift since the review baseline (`eacbe43` → `136b6f7`)

> ⚠ **Action item**: reconcile before claiming parity.

- **Chunk length changed to 750** upstream (`chunking/chunking.py`), while the Rust port (and the
  TS source it mirrors) still use **1500**. Upstream also added a `chunk_size` field to index
  metadata + cache validation so the change auto-invalidates stale caches. Decide whether csp
  follows 750 (and adds the metadata field to both TS and Rust) or documents 1500 as a deliberate
  divergence.

---

## 7. How to refresh this analysis

1. Update the upstream checkout and diff from the recorded baseline:
   `git -C <semble> log 136b6f7..main --oneline`.
2. Quality gate the Rust side: `cargo fmt --all && cargo clippy --all-targets --all-features --
   -D warnings && cargo test --workspace`.
3. Re-read any changed module and update the matching §4 section + §5 constants table.
4. When a stub gets wired (TD-002) or grammars are added, move the item out of §6.2 and update
   §4.10 / §4.3. Bump the baseline in `index.md` and this file's header.
5. Cross-check against the [upstream-semble-sync-baseline] and [rust-rewrite-track-status]
   memories and `CLAUDE.md` (Rust rewrite section).

---

*Related: [`ARCHITECTURE.md`](../../../ARCHITECTURE.md) (target architecture),
[ADR-0001](../decisions/0001-native-tree-sitter.md) (tree-sitter bindings),
[ADR-0002](../decisions/0002-index-storage-cache-model.md) (cache model),
[ADR-0003](../decisions/0003-rewrite-in-rust.md) (Rust rewrite),
[`../knowledge/tech-stack.md`](../knowledge/tech-stack.md).*
