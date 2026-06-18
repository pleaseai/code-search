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
- [ ] T005 Port ranking weighting — RRF k=60, adaptive alpha 0.3 symbol / 0.5 NL, is_symbol_query (file: crates/csp/src/ranking/weighting.rs) (depends on T002)
- [ ] T006 Port ranking boosting — multi-chunk file boost, query-type boosts (file: crates/csp/src/ranking/boosting.rs) (depends on T002)
- [ ] T007 Port ranking penalties — test/barrel/.d.ts/compat path penalties, applied only when alpha_weight < 1.0 (file: crates/csp/src/ranking/penalties.rs) (depends on T002)
- [ ] T008 Port BM25 scoring math + enrich_for_bm25 (stem×2 + last 3 dir parts) (file: crates/csp/src/ranking/bm25.rs) (depends on T003)

### Phase 2: Chunking

- [ ] T009 Port chunking core — tree-sitter AST chunking, 1500-char target, MIN_CHUNK_SIZE=50, RECURSION_DEPTH=500, line fallback (file: crates/csp/src/chunking/core.rs) (depends on T002)
  STOP: if a grammar crate's node types differ from the Python/TS tree-sitter pack such that chunk boundaries shift, reconcile the extension→language map before continuing.
- [ ] T010 Port chunk-source + extension→language map (file: crates/csp/src/chunking/source.rs) (depends on T009)

### Phase 3: Indexing

- [ ] T011 Port file walker — ignore crate, .gitignore + .cspignore, default-ignore dirs incl. .csp/ (file: crates/csp/src/indexing/file_walker.rs) (depends on T004)
- [ ] T012 Port file classification — code/docs/config content typing (file: crates/csp/src/indexing/files.rs) (depends on T011, T002)
- [ ] T013 Port dense embeddings via model2vec-rs (file: crates/csp/src/indexing/dense.rs) (depends on T003)
  STOP: embedding parity is the single largest risk — verify vectors match the TS output before building any search result on top (see Context STOP Conditions).
- [ ] T014 Port sparse BM25 index build (file: crates/csp/src/indexing/sparse.rs) (depends on T008)
- [ ] T015 Port content-hash cache — global ~/.csp/index/, serde serialization (file: crates/csp/src/indexing/cache.rs) (depends on T002)
  STOP: pick a serialization format that can be rebuilt from source; do not promise cross-version cache compatibility (the cache is disposable per ADR-0002).
- [ ] T016 Port index create/orchestration (file: crates/csp/src/indexing/create.rs, crates/csp/src/indexing/mod.rs) (depends on T010, T012, T013, T014, T015)

### Phase 4: Search + core API

- [ ] T017 Port search pipeline — semantic + BM25 → RRF → multi-chunk boost → query-type boost → top-k rerank with path penalties + file-saturation decay 0.5 (file: crates/csp/src/search.rs) (depends on T005, T006, T007, T016)
- [ ] T018 Port CspIndex-equivalent core API — fromPath/fromGit/search/findRelated/save/load (file: crates/csp/src/lib.rs, crates/csp/src/index.rs) (depends on T017)

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
- T001 (shared cross-language fixture harness) deferred to the heavier modules (chunking/search/embeddings); for these pure modules the TS test vectors are inlined directly as Rust unit tests, which is sufficient equivalence coverage.

## Decision Log

- 2026-06-18: Incremental leaf-first port over big-bang; golden fixtures from the TS suite as the equivalence oracle (ADR-0003).

## Surprises & Discoveries

_Recorded during implementation._
