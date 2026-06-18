# Rewrite csp in Rust

> Track: rust-rewrite-20260618
> Type: refactor (language rewrite / migration)
> Origin decision: [ADR-0003](../../decisions/0003-rewrite-in-rust.md)

## Overview

`@pleaseai/csp` currently exists as a complete TypeScript/Bun implementation (~5,900 LOC) ported from MinishLab/semble. Per ADR-0003, the project is being rewritten in Rust to gain single-binary distribution, better indexing/embedding performance and memory footprint, and a more natural fit with the native Rust ecosystem (`model2vec-rs`, `tree-sitter`, `ignore`, `rmcp`).

This track covers **Phases 1–7** of the ADR-0003 roadmap. Phase 0 (Cargo workspace scaffold, clap CLI stubs, Rust CI, pinned toolchain) is already committed on branch `feat/rust-rewrite`. The defining constraint is **behavioral equivalence**: the Rust build must reproduce the existing implementation's observable behavior (tokenization, ranking order, chunk boundaries, search results, CLI/MCP contracts), verified by reusing the TypeScript test suite as language-neutral golden fixtures. The TypeScript `src/` remains the source of truth until the Rust line reaches parity, then is retired.

## Scope

The rewrite is delivered in dependency-ordered phases (leaf-first, each verifiable against golden fixtures):

- **P1 — Pure core**: identifier-aware tokenization (camelCase/PascalCase/snake_case split + lowercased compound), ranking (weighting, boosting, penalties), and BM25 scoring math. RRF fusion (`k=60`), adaptive alpha (`0.3` symbol / `0.5` NL).
- **P2 — Chunking**: tree-sitter AST chunking with line-fallback (1500-char target, `MIN_CHUNK_SIZE=50`, `RECURSION_DEPTH=500`), and the extension→language map.
- **P3 — Indexing**: dense embeddings via `model2vec-rs`, file walking via the `ignore` crate (`.gitignore` + `.cspignore`, default-ignore dirs), BM25 sparse index, and the content-hash cache in the global `~/.csp/index/` (per ADR-0002).
- **P4 — Search**: the hybrid pipeline (semantic + BM25 → RRF → multi-chunk file boost → query-type boost → top-k rerank with path penalties + file-saturation decay `0.5`) and the `CspIndex`-equivalent core API (`fromPath`/`fromGit`/`search`/`findRelated`/`save`/`load`).
- **P5 — CLI**: the `csp` binary subcommands (`search`/`index`/`find-related`/`mcp`/`init`/`savings`/`clear`) with flags (`--top-k`/`--content`/`--index`/`--agent`), plus `~/.csp/savings.jsonl` telemetry.
- **P6 — MCP**: the MCP server via `rmcp`, exposing the `search` and `find_related` tools, launched by `csp mcp`.
- **P7 — Distribution**: Biome-style multi-channel distribution — cross-compiled release binaries (GitHub Releases), an npm wrapper package preserving the `bunx @pleaseai/csp` entrypoint, and the Homebrew tap; plus README/README.ko updates.

## Success Criteria

- [ ] **SC-001**: For every behavior covered by the TypeScript test suite, the Rust build produces identical results (tokenization output, ranking order, chunk boundaries, search result ordering) on the shared golden fixtures.
- [ ] **SC-002**: A user can run every README CLI snippet and MCP configuration against the Rust build via `bunx @pleaseai/csp …` with no change to the documented commands.
- [ ] **SC-003**: The tool is installable and runnable as a single self-contained binary with no Node.js/Bun runtime present on the machine.
- [ ] **SC-004**: Indexing a representative repository completes at least as fast as the TypeScript build, with no regression in result quality (same top-k results on the fixtures).
- [ ] **SC-005**: The Rust CI gate (`fmt` + `clippy -D warnings` + `test`) passes on every phase's merge.

## Constraints

- **No behavioral change** relative to semble / the TypeScript port — observable outputs must match (this is a rewrite, not a redesign).
- **Public CLI + MCP surface is preserved**: subcommand names, flags, MCP tool names, the `bunx @pleaseai/csp` entrypoint, the `~/.csp/` paths, and the global index-cache model (ADR-0002) carry over unchanged.
- **Phased, parity-gated delivery**: each phase merges only when its golden-fixture equivalence checks pass; the TypeScript implementation stays authoritative until full parity.
- **GitHub Actions third-party actions remain SHA-pinned**; the Rust toolchain is pinned via `rust-toolchain.toml`.

## Out of Scope

- The JS-importable library API (`import { CspIndex }`) — deferred behind a future napi-rs seam; the `csp` core crate is designed as that seam (ADR-0003).
- Any new search/ranking features or behavior improvements beyond what the TypeScript implementation already does.
- Removal/retirement of the TypeScript `src/` — happens in a separate cleanup once parity is confirmed, not within this track.
- New language grammars or embedding models beyond those the current implementation supports.
