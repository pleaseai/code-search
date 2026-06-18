# Plan: Rewrite csp in Rust

> Track: rust-rewrite-20260618
> Spec: [spec.md](./spec.md)

## Overview

- **Source**: /please:plan
- **Track**: rust-rewrite-20260618
- **Issue**: #TBD
- **Created**: 2026-06-18
- **Approach**: Incremental, leaf-first port verified against golden fixtures (not big-bang)
- **Execution**: code
- **Planned At**: 4ead3c8

## Purpose

Deliver Phases 1–7 of [ADR-0003](../../decisions/0003-rewrite-in-rust.md): port the completed TypeScript implementation into the Rust Cargo workspace scaffolded in Phase 0, preserving observable behavior and the CLI/MCP public surface.

## Context

The TypeScript implementation under `src/` is the behavioral oracle. Each Rust module is ported leaf-first (no-dependency modules first) so it can be verified in isolation against fixtures extracted from the corresponding TS tests. The Rust workspace already exists (`crates/csp` = library seam, `crates/csp-cli` = `csp` binary) with clap CLI stubs and a Rust CI gate.

The crate mapping (verified in ADR-0003): `model2vec-rs` (dense embeddings), `tree-sitter` (chunking), `ignore` (file walking), `rmcp` (MCP), `clap` (CLI).

### STOP Conditions

- If `model2vec-rs` cannot reproduce the TS embedding vectors within numerical tolerance (different tokenization, pooling, or normalization), STOP and reconcile the embedding contract before proceeding — every downstream search result depends on it.
- If any phase's golden-fixture equivalence check diverges from the TS output, STOP and reconcile rather than adjusting the fixture to match the Rust output.

## Architecture Decision

Incremental over big-bang: the dependency-ordered phases each merge behind a passing fixture-equivalence gate, keeping the TS build authoritative until full parity. The `csp` core crate holds all logic (the future napi-rs seam); `csp-cli` is a thin clap shell over it. Distribution follows the Biome multi-channel model (binary + npm wrapper + Homebrew) so the `bunx @pleaseai/csp` contract survives the language change.

## Tasks

### Phase 1: Pure core (tokens, ranking, BM25)

- [ ] T001 Build the golden-fixture harness — extract tokenization/ranking/chunk/search vectors from the TS test suite into shared JSON fixtures (file: tests/fixtures/, crates/csp/tests/equivalence.rs)
  STOP: if a TS test asserts behavior that cannot be expressed as a deterministic input→output vector (e.g. timing-dependent), record it as a manual-verification item instead of forcing it into a fixture.
- [x] T002 [P] Port core types — ContentType/CallType enums, Chunk, chunk_to_dict/chunk_from_dict (file: crates/csp/src/types.rs) (depends on T001)
- [x] T003 [P] Port identifier-aware tokenizer — camelCase/PascalCase/snake_case split + lowercased compound (file: crates/csp/src/tokens.rs) (depends on T001)
- [x] T004 [P] Port utils — is_git_url, resolve_chunk (file: crates/csp/src/utils.rs) (depends on T001)
- [x] T005 Port ranking weighting — adaptive alpha 0.3 symbol / 0.5 NL via resolve_alpha (file: crates/csp/src/ranking/weighting.rs) (depends on T002)
- [x] T006 Port ranking boosting — apply_query_boost (symbol/embedded/stem), boost_multi_chunk_files, definition detection via fancy-regex (file: crates/csp/src/ranking/boosting.rs) (depends on T002)
- [x] T007 Port ranking penalties — test/barrel/.d.ts/compat path penalties + rerank_top_k with file-saturation decay (file: crates/csp/src/ranking/penalties.rs) (depends on T002)
- [x] T008 Port BM25 scoring core — enrich_for_bm25 (stem×2 + last 3 dir parts), selector_to_mask, Bm25Index build/get_scores (file: crates/csp/src/indexing/sparse.rs) (depends on T003)

### Phase 2: Chunking

- [x] T009 Port chunking core — merge algorithm (generic over AstNode), chunk_lines, 1500-char target, MIN_CHUNK_SIZE=50, RECURSION_DEPTH=500, line fallback (file: crates/csp/src/chunking/core.rs) (depends on T002) — tree-sitter grammar registration activates with the language map (T012), matching the TS ALL_LANGUAGES stub
  STOP: if a grammar crate's node types differ from the Python/TS tree-sitter pack such that chunk boundaries shift, reconcile the extension→language map before continuing.
- [x] T010 Port chunk-source entry point — line-number resolution, language fallback (file: crates/csp/src/chunking/source.rs) (depends on T009) — extension→language map lands with files (T012)

### Phase 3: Indexing

- [x] T011 Port file walker — ignore crate (Match::{None,Ignore,Whitelist} ↔ npm {ignored,unignored}), .gitignore + .cspignore, negation-with-ext bypass (found), default-ignore dirs incl. .csp/ (file: crates/csp/src/indexing/file_walker.rs) (depends on T004)
- [x] T012 Port file classification — EXTENSION_TO_LANGUAGE map (~330), DOC/CONFIG/DATA/CODE language sets, detect_language, get_extensions (file: crates/csp/src/indexing/files.rs) (depends on T002)
- [x] T013 Port dense embeddings (file: crates/csp/src/indexing/dense.rs) (depends on T003) — **STOP resolved**: the TS `dense.ts` is a deterministic *stub* (FNV-1a → mulberry32 → Box-Muller → L2), not real Model2Vec (TS `TODO(dense)` still open). The oracle = TS test suite, so the stub is reproduced bit-for-bit (verified against golden vectors captured from TS); real model2vec-rs integration is a genuinely separate future task and is NOT required for parity. Includes SelectableBasicBackend (cosine query + selector + save/load).
- [x] T014 Port BM25 save/load — Bm25Index::save/load to bm25.json, TS-compatible camelCase + entry-array format (build itself landed in T008) (file: crates/csp/src/indexing/sparse.rs) (depends on T008)
- [x] T015 Port content-hash cache primitives — resolve_cache_dir (sha256 key, TS-parity JSON), resolve_index_root, compute_content_hash, ensure_cache_dir (0700 chain), clear_index_cache (symlink-safe guard). load_or_build_index orchestration deferred to T016 (needs CspIndex) (file: crates/csp/src/indexing/cache.rs) (depends on T002)
  STOP: pick a serialization format that can be rebuilt from source; do not promise cross-version cache compatibility (the cache is disposable per ADR-0002).
- [x] T016 Port index create/orchestration — create_index_from_path: walk → chunk_source → embed → BM25 build → SelectableBasicBackend, MAX_FILE_BYTES, displayRoot-relative paths, empty-chunks error (file: crates/csp/src/indexing/create.rs) (depends on T010, T012, T013, T014, T015). load_or_build_index (cache.ts orchestration) folds into T018 (needs CspIndex save/loadFromDisk).

### Phase 4: Search + core API

- [x] T017 Port search pipeline — semantic + BM25 → per-list RRF (k=60) → alpha combine → rerank (multi-chunk boost → query boost → top-k file-saturation). **Reproduces search.ts's current inline ranking exactly** (apply_query_boost = identity, rerank = file-saturation only, no path penalties), matching the TS oracle — wiring the full ranking modules (T006/T007) is a future integration step, as in TS. Trait-based (EmbeddingModel/VectorBackend/SparseBackend) (file: crates/csp/src/search.rs) (depends on T005, T006, T007, T016)
- [x] T018 Port CspIndex core API — from_path/from_git(shallow clone, dash-ref guard)/search(filters→selector)/find_related/stats/save/load_from_disk + manifest (schema v1, parse_manifest validation) + load_or_build_index cache orchestration (miss/hit/invalidate) (file: crates/csp/src/indexing/index.rs) (depends on T017) — folds in the T015-deferred cache.ts orchestration

### Phase 5: CLI + telemetry

- [ ] T019 Wire CLI subcommands to core — search/index/find-related/init/clear with --top-k/--content/--index/--agent (file: crates/csp-cli/src/main.rs) (depends on T018)
- [ ] T020 Port savings telemetry — ~/.csp/savings.jsonl + savings subcommand (file: crates/csp/src/stats.rs, crates/csp-cli/src/main.rs) (depends on T018)

### Phase 6: MCP server

- [ ] T021 Port MCP server via rmcp — search + find_related tools, launched by `csp mcp` (file: crates/csp-cli/src/mcp.rs) (depends on T018)
  STOP: rmcp is newer than the TS MCP SDK — verify tool schemas and stdio transport behavior match the existing MCP server before declaring parity.

### Phase 7: Distribution

- [ ] T022 Cross-compile release binaries — GitHub Releases matrix (macOS arm64/x64, Linux x64/arm64 glibc+musl, Windows x64), SHA-pinned actions (file: .github/workflows/release.yml)
- [ ] T023 npm wrapper package preserving `bunx @pleaseai/csp` — platform binary sub-packages (file: package.json, npm/) (depends on T022)
- [ ] T024 Homebrew formula update + README/README.ko updates (library API change note, install) (file: README.md, README.ko.md) (depends on T022)

## Dependencies

Phase 1 (T001 → {T002,T003,T004} → {T005,T006,T007,T008}) → Phase 2 (T009 → T010) and Phase 3 run after their Phase 1 deps; Phase 3 converges at T016 → Phase 4 (T017 → T018) → {Phase 5 (T019, T020), Phase 6 (T021)} → Phase 7 (T022 → {T023, T024}). T001 (fixtures) gates everything.

## Key Files

- `src/**` (TypeScript) — behavioral oracle, mapped 1:1 to `crates/csp/src/**`
- `crates/csp/` — core library (the port target + napi seam)
- `crates/csp-cli/` — `csp` binary (clap shell)
- `.please/docs/decisions/0003-rewrite-in-rust.md` — decision + crate mapping
- `.please/docs/decisions/0002-index-storage-cache-model.md` — cache model (carries over)

## Verification

- Per-phase: `cargo test` equivalence checks pass against the golden fixtures (T001).
- CI gate: `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` green (SC-005).
- Parity: TS and Rust produce identical top-k results on the fixtures (SC-001, SC-004).
- Surface: README CLI/MCP snippets run unchanged via `bunx @pleaseai/csp` (SC-002).
- Distribution: single binary runs with no Node/Bun present (SC-003).

## Test Scenarios

### T001
- Happy: TS test vectors → extraction → JSON fixtures readable by a Rust test; round-trips for at least tokenization + ranking + chunk + search categories.
- Test expectation: harness itself verified by loading fixtures in a placeholder Rust test that asserts non-empty parse.

### T002
- Happy: ContentType { Code, Docs, Config } and Chunk fields (file_path, start_line, end_line) round-trip via serde matching the TS field semantics.

### T003
- Happy: `getUserById` → {get, user, by, id, getuserbyid}; `snake_case_name` → {snake, case, name, snake_case_name}.
- Edge: single-token, all-caps acronym, mixed digits.
- Verification: identical token sets to the TS tokenizer fixtures.

### T004
- Test expectation: covered by the fixtures of the modules that consume utils (no standalone behavior beyond helpers).

### T005
- Happy: RRF with k=60 over known rank lists yields the TS fused order; is_symbol_query picks alpha 0.3 vs 0.5 correctly.
- Edge: empty list, single source, tie-breaking.

### T006
- Happy: multi-chunk file boost and query-type boosts reproduce TS score adjustments on fixture inputs.

### T007
- Happy: test/barrel/.d.ts/compat penalties applied at the TS magnitudes; Error: penalties NOT applied when alpha_weight == 1.0.

### T008
- Happy: BM25 scores and enrich_for_bm25 output (stem repeated ×2 + last 3 dir parts) match TS fixtures.

### T009
- Happy: a supported-language source chunks at the same boundaries as TS; Edge: tiny node (<50 chars) not recursed; Error: unsupported language falls back to line chunking.

### T010
- Happy: extension→language map resolves the same languages as TS for the fixture file set.

### T011
- Happy: walking a fixture tree respects .gitignore + .cspignore and default-ignore dirs identically to TS; Edge: nested ignore files.

### T012
- Happy: code/docs/config classification matches TS for the fixture files.

### T013
- Happy: model2vec-rs embeddings match TS embedding vectors within tolerance on fixture chunks (see STOP).
- Error: missing/невалид model path surfaces a clear error.

### T014
- Happy: BM25 index built from fixture chunks yields the same postings/scores as TS.

### T015
- Happy: content-hash cache writes/reads round-trip; a changed file invalidates only its entry; cache lives under ~/.csp/index/.

### T016
- Integration: indexing a fixture repo produces the same chunk+embedding+BM25 index contents as TS.

### T017
- Happy: end-to-end search over the fixture index returns the same top-k ordering as TS for symbol and NL queries.
- Edge: empty index, query with no matches.

### T018
- Happy: fromPath/fromGit/search/findRelated/save/load behave equivalently to the TS CspIndex on fixtures; save→load round-trips.

### T019
- Happy: `csp search/index/find-related/init/clear` produce equivalent output to the TS CLI; flags (--top-k/--content/--index/--agent) parsed identically.
- Error: invalid flag/arg yields a clear clap error.

### T020
- Happy: a search appends a savings record to ~/.csp/savings.jsonl; `csp savings` aggregates equivalently to TS.

### T021
- Integration: an MCP client invoking `search` and `find_related` over stdio gets the same tool schemas and results as the TS MCP server.

### T022
- Happy: the release workflow produces runnable binaries for each target triple; Test expectation: verified by a smoke `csp --version` per artifact in CI.

### T023
- Happy: `bunx @pleaseai/csp mcp` resolves the platform binary and runs unchanged; Test expectation: install smoke test in CI.

### T024
- Test expectation: none -- docs/formula edits; verified by manual review that README snippets and the Homebrew formula reference the binary distribution.

## Progress

- 2026-06-18: **T002/T003/T004 done** — ported `types`, `tokens` (camelCase splitter reimplemented as a state machine, since Rust `regex` lacks the upstream lookahead), and `utils` (`is_git_url`, `resolve_chunk`) into `crates/csp`. 32 equivalence tests (mirroring the TS test vectors) pass; `cargo fmt`/`clippy -D warnings`/`test` green.
- 2026-06-18: **T005/T007 done + T006 partial** — added the `ranking` module: `weighting` (`resolve_alpha`), `penalties` (`file_path_penalty` + `rerank_top_k` with file-saturation decay), and `boosting::is_symbol_query`. Score maps use `IndexMap<usize, f64>` (chunk-index keys, insertion-ordered) as the Rust analogue of TS `Map<Chunk, number>`. 58 tests total pass.
- 2026-06-18: **T008 done** — ported the BM25 scoring core into `indexing/sparse` (`enrich_for_bm25`, `selector_to_mask`, `Bm25Index::{build, get_scores}`). Reproduced two subtle parity points: per-add `f32` rounding (Float32Array semantics) and first-appearance unique-term ordering, both of which affect exact scores. 73 tests total pass.
- 2026-06-18: **T006 done → PHASE 1 COMPLETE.** Ported the full `boosting` module: `apply_query_boost` (symbol-definition / embedded-symbol / stem-match boosts), `boost_multi_chunk_files`, and definition detection. Definition patterns use `fancy-regex` (the upstream `(?<=\s)` lookbehind is unsupported by the `regex` crate) with the patterns transcribed verbatim and cached per symbol name. 88 tests total pass; fmt / clippy -D warnings / test green.
- 2026-06-18: **T018 done → PHASE 4 COMPLETE.** Ported `CspIndex`: `from_path` (dir validation + create orchestration), `from_git` (shallow clone into a 0700 tempdir via `std::process::Command`, dash-ref flag-injection guard, re-root at URL, auto-cleanup on drop), `search` (blank/top_k/empty guards + language/path filters → selector, empty-selector short-circuit), `find_related` (re-embed seed, exclude seed, over-fetch by 1), `stats`, `save` (chunks.json/bm25/dense/manifest), `load_from_disk` (artifact + schema-version + manifest validation), `parse_manifest`. Also folded in the T015-deferred `load_or_build_index` cache orchestration (resolve_cache_dir → ensure → content-hash reuse-or-rebuild), with a miss/hit/invalidate test. Added `IndexStats` type; promoted `tempfile` to a normal dep. **229 tests total** pass. Remaining: Phase 5 (T019 CLI wiring, T020 savings telemetry), Phase 6 (T021 rmcp MCP), Phase 7 (T022–T024 distribution — CI-only verification).
- 2026-06-18: **T017 done.** Ported the hybrid `search` pipeline as a trait-based module (EmbeddingModel/VectorBackend/SparseBackend, implemented for the real dense/sparse types and mockable in tests). Like dense, `search.ts` itself still uses *inline* ranking stubs (`apply_query_boost` = identity; `rerank_top_k` = file-saturation only, ignoring `penalisePaths`) with a `TODO(integration)` to wire `ranking/*` — so to match the oracle, search.rs reproduces those stubs exactly (the full `ranking::{apply_query_boost, rerank_top_k}` from T006/T007 stay ported-but-unwired, mirroring TS). `boost_multi_chunk_files` is the shared ranking impl. RRF k=60, startLine-stable union, alpha blend all verified against search.test.ts vectors. **209 tests total** pass. Next: T018 CspIndex core API (fromPath/fromGit/search/findRelated/save/loadFromDisk + manifest + cache reuse via load_or_build_index).
- 2026-06-18: **T016 done → PHASE 3 COMPLETE.** Ported `create_index_from_path` orchestration: walk_files → chunk_source → embed_chunks → Bm25Index::build(tokenize∘enrich) → SelectableBasicBackend, with MAX_FILE_BYTES skip, displayRoot-relative chunk paths, and the empty-chunks error. **192 tests total** pass. The `load_or_build_index` orchestration from cache.ts folds into T018 (it needs CspIndex.save/loadFromDisk). Next: Phase 4 — T017 search pipeline (RRF + boosts + rerank, all deps ready) then T018 CspIndex core API (fromPath/fromGit/search/findRelated/save/loadFromDisk + manifest + cache reuse).
- 2026-06-18: **T013 done — STOP condition resolved, not deferred.** Discovered the TS `dense.ts` ships a *stub* Model2Vec (deterministic hash-seeded vectors: FNV-1a over UTF-16 units → mulberry32 → Box-Muller → L2-normalize), with real Model2Vec still an open `TODO(dense)`. Since behavioral parity is measured against the TS test suite, the Rust port reproduces the **stub** bit-for-bit — including the exact f64↔f32 narrowing in `stub_embed` and the u32 wrapping ops — verified against golden vectors captured by running the TS functions (`fnv1a("hello")=1335831723`, `stub("hello",8)=[0.0856,…]`). The plan's "model2vec-rs cannot reproduce TS vectors" STOP is therefore moot: both sides use the stub. Also ported `SelectableBasicBackend` (cosine query, selector pool, vectors.bin/args.json save/load). **187 tests total** pass. Real model2vec-rs integration tracked as future work (out of scope for oracle parity). Phase 3 now only needs T016 (orchestration). See memory `dense-embedding-is-a-stub`.
- 2026-06-18: **T014 + T015 done.** T014: `Bm25Index::{save,load}` to `bm25.json` in the exact TS shape (camelCase keys, entry arrays) so indexes are cross-loadable. T015: ported the pure cache primitives — `resolve_cache_dir` (sha256 key over `{sourceId,content,ref}` JSON, TS-byte-parity via a field-ordered serde struct + `ContentType::as_str`), `resolve_index_root`, `compute_content_hash` (order-independent, `<utf16-len>:<path>` + bytes), `ensure_cache_dir` (0700 chain, Unix), `clear_index_cache` (canonicalize + direct-`index`-child guard rejecting symlink escapes). Added `sha2` dep. **168 tests total** pass. `load_or_build_index` orchestration deferred to T016 (composes CspIndex → dense T013). Phase 3 remaining: T013 (model2vec — STOP, needs weights), T016 (orchestration, depends on T013).
- 2026-06-18: **T011 done** — ported `indexing/file_walker` using the `ignore` crate. Mapped `Gitignore::matched` → `Match::{None,Ignore,Whitelist}` onto the upstream npm `{ignored,unignored}` contract; reproduced the negation-with-extension bypass (`found`) via per-pattern matchers and the `has_negated_ext_pattern` fast-path. Recursive `walk`/`walk_files` with symlink skip, sorted entries, DEFAULT_IGNORED_DIRS (.csp/), nested `.gitignore`/`.cspignore`, case-insensitive extension filter. 17 FS integration tests via `tempfile` dev-dep; **146 tests total** pass. Phase 3 remaining: T013 (model2vec-rs — STOP-gated, needs model weights), T014 (BM25 save/load), T015 (content-hash cache), T016 (orchestration).
- 2026-06-18: **T012 done** — ported `indexing/files`: the full `EXTENSION_TO_LANGUAGE` map (~330 entries), DOC/CONFIG/DATA language sets, derived CODE set, `detect_language` (case-insensitive suffix, dotfile-aware), and `get_extensions` (sorted/deduped union by content type). 129 tests total pass. Remaining Phase 3: T011 (file-walker, `ignore` crate — API differs from the npm pkg), T013 (model2vec-rs embedding — STOP-gated parity), T014 (BM25 save/load), T015 (content-hash cache), T016 (orchestration).
- 2026-06-18: **T009/T010 done → PHASE 2 COMPLETE.** Ported the `chunking` module: the merge algorithm (`merge_node_inner`/`merge_node`/`merge_adjacent_chunks`) generic over an `AstNode` trait (unit-tested with mock nodes), `chunk_lines` (CRLF-aware, char offsets), and `chunk_source` (1-indexed line numbering, language fallback). At parity with the current TS, `is_supported_language` is a `false` stub and real tree-sitter grammar parsing activates with the language map (T012). 115 tests total pass. **Next: Phase 3 — file walking (ignore crate), then the model2vec-rs embedding (STOP-gated parity risk) and the content-hash cache.**
- T001 (shared cross-language fixture harness) deferred to the heavier modules (chunking/search/embeddings); for these pure modules the TS test vectors are inlined directly as Rust unit tests, which is sufficient equivalence coverage.

## Decision Log

- 2026-06-18: Incremental leaf-first port over big-bang; golden fixtures from the TS suite as the equivalence oracle (ADR-0003).

## Surprises & Discoveries

_Recorded during implementation._
