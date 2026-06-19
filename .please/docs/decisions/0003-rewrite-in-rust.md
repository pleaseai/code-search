# ADR 0003 — Rewrite `@pleaseai/csp` from TypeScript/Bun to Rust

- **Status**: Proposed
- **Date**: 2026-06-18
- **Deciders**: csp maintainers
- **Relates to**: [ADR 0001](0001-native-tree-sitter.md) (native tree-sitter bindings), [ADR 0002](0002-index-storage-cache-model.md) (global index cache)

## Context

`@pleaseai/csp` is a hybrid code-search tool ported from [MinishLab/semble](https://github.com/MinishLab/semble) (Python). The TypeScript/Bun port is **effectively complete** — roughly 5,900 LOC of source plus tests covering the full surface: identifier-aware tokenization, BM25 + Model2Vec dense embeddings, RRF fusion, the ranking pipeline (boosting / penalties / weighting), tree-sitter AST chunking, the `CspIndex` orchestrator, the `csp` CLI, the MCP server, and the global `~/.csp/index/` cache.

Despite the port being done, we are reconsidering the implementation language. The motivations (all four confirmed by the maintainer):

1. **Single static-binary distribution** — ship one self-contained binary with no Node/Bun runtime dependency, removing the install friction documented in [ADR 0001](0001-native-tree-sitter.md) (NAPI prebuilds, ~50–100 MB `node_modules`, platform-loader caveats).
2. **Indexing / embedding performance** — faster large-repo indexing, higher embedding throughput, lower memory footprint.
3. **Ecosystem fit** — the three load-bearing dependencies have first-class Rust crates, several authored by the upstream/relevant communities (see verification below). The TypeScript port had to *work around* the embedding layer; Rust makes it native.
4. **Maintainer preference / learning.**

### Crate availability (verified 2026-06-18 via crates.io)

| Concern | Current (TS) | Rust crate | Version | Notes |
|---------|--------------|------------|---------|-------|
| Dense embeddings (Model2Vec) | `@huggingface/transformers` (ONNX workaround) | **`model2vec-rs`** | 0.2.1 | "Official Rust Implementation of Model2Vec" — by upstream MinishLab |
| AST chunking | `@kreuzberg/tree-sitter-language-pack` (NAPI) | **`tree-sitter`** + grammar crates | 0.26.9 | tree-sitter's native ecosystem |
| File walking / ignore | `ignore` (npm) | **`ignore`** | 0.4.26 | ripgrep's crate, best-in-class |
| MCP server | `@modelcontextprotocol/sdk` | **`rmcp`** | 1.7.0 | official Rust MCP SDK, mature |
| CLI | `commander` | **`clap`** | 4.6.x | mature |
| BM25 / sparse | hand-written | (port as-is) | — | pure algorithm, trivial |

The decisive factor is `model2vec-rs`: the part of the port that was *most* awkward in TypeScript becomes the *cleanest* in Rust, maintained by the same authors as semble itself.

## Decision

**Rewrite csp in Rust**, structured as a Cargo workspace with a `csp` core crate as the library seam, a `clap`-based CLI binary, and an `rmcp`-based MCP server.

### Distribution: the Biome model

To reconcile "single binary" with the existing `bunx @pleaseai/csp` contract (every MCP/CLI snippet in the README depends on it), distribute the same Rust core through three channels, as [Biome](https://biomejs.dev) does:

- **Rust binary** — `cargo install`, GitHub Releases prebuilt binaries, and the existing Homebrew tap (see commit `0278323`).
- **npm wrapper package** — a thin `@pleaseai/csp` package with platform-specific binary sub-packages, so `bunx @pleaseai/csp mcp` and all README setup snippets keep working unchanged.

This preserves the entire **CLI + MCP** public surface. The only contract that breaks is JS-side `import { CspIndex }`.

### Library contract: defer, keep the seam

csp is a young project with effectively no external JS library consumers. Therefore:

- **Remove** the JS-importable library API for now; document the change in both READMEs ("changed in the Rust rewrite; may return via napi-rs on demand").
- **Design the `csp` core crate as the future napi-rs seam** — if real demand appears, a napi layer can be added on top without touching the core.

Adding napi-rs *now* would directly conflict with motivation #1 (single binary), so it is explicitly deferred rather than adopted.

## Consequences

### Positive

- Single self-contained binary; no Node/Bun runtime, no NAPI prebuild dance, smaller install.
- Native tree-sitter, native Model2Vec (`model2vec-rs`), native gitignore (`ignore`) — removes the TS embedding workaround and the heavy `node_modules`.
- Expected gains in indexing speed, embedding throughput, and memory.
- CLI + MCP public surface (and README snippets) preserved via the npm wrapper.

### Negative

- **Throws away a finished, working ~5,900 LOC implementation.** Real cost, justified only by the four motivations above.
- JS library API (`CspIndex` import) is dropped until/unless napi-rs is added.
- New toolchain and CI: cross-compilation matrix, GitHub Releases binaries, npm wrapper publishing, Homebrew formula update.
- `rmcp` is comparatively newer than the TS MCP SDK; MCP parity needs explicit verification.
- Behavioral equivalence with semble/the TS port must be re-proven from scratch.

### Neutral

- [ADR 0001](0001-native-tree-sitter.md)'s native-vs-WASM tension dissolves — tree-sitter is a native Rust crate. ADR 0001 stays accepted for the TS lineage but no longer constrains the Rust line.
- [ADR 0002](0002-index-storage-cache-model.md)'s global `~/.csp/index/` cache model is language-agnostic and carries over unchanged.
- The existing TS test suite becomes **golden fixtures** for verifying the Rust rewrite's behavioral equivalence, then is retired with the TS code.

## Alternatives considered

- **Stay on TypeScript/Bun.** Rejected: does not deliver single-binary distribution and leaves the embedding workaround in place. Lowest cost, but fails motivations #1–#3.
- **Adopt napi-rs now (Rust core + JS bindings as the primary artifact).** Rejected for the initial rewrite: conflicts with single-binary distribution and doubles distribution complexity. Kept as a *future* option layered on the core crate.
- **Partial / hot-path-only rewrite (FFI from TS into a Rust embedding/chunking core).** Rejected: keeps the Node/Bun runtime dependency (fails #1), adds an FFI boundary, and yields a more complex system than either pure option.

## References

- Upstream: [MinishLab/semble](https://github.com/MinishLab/semble)
- `model2vec-rs` — <https://crates.io/crates/model2vec-rs>
- `rmcp` (Rust MCP SDK) — <https://crates.io/crates/rmcp>
- `tree-sitter`, `ignore`, `clap` — crates.io
- Distribution precedent: Biome (Rust core, multi-channel npm/Homebrew/binary distribution)
