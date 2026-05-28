# ADR 0001 — Use Native Tree-sitter Bindings via `@kreuzberg/tree-sitter-language-pack`

- **Status**: Accepted
- **Date**: 2026-05-28
- **Deciders**: csp maintainers
- **Supersedes**: the "No native add-ons" guideline previously stated in `ARCHITECTURE.md`

## Context

`@pleaseai/csp` ports MinishLab/semble from Python to TypeScript / Bun. Semble parses source code with tree-sitter via the Python `tree-sitter-language-pack`, which exposes a few hundred pre-built grammars through native bindings. The chunker (`src/semble/chunking/core.py`) depends on this coverage — every supported language has an entry in `EXTENSION_TO_LANGUAGE`, and missing a grammar degrades silently to line-based fallback chunking.

For the TypeScript port we considered two options:

1. **`web-tree-sitter`** (WASM). Portable across Linux / macOS / Windows / containers, no native build step. This was the original `ARCHITECTURE.md` guidance ("No native add-ons").
2. **`@kreuzberg/tree-sitter-language-pack`** (NAPI). Native bindings, ships pre-compiled binaries for macOS / Linux / Windows. Closer parity with the upstream Python implementation.

The trade-off is portability (WASM wins) vs. coverage + startup cost + parity (NAPI wins).

## Decision

**Adopt `@kreuzberg/tree-sitter-language-pack`** as the canonical tree-sitter binding for csp.

Rationale:

- **Parity with semble.** Upstream uses the same multi-grammar `tree-sitter-language-pack` package. Matching the same set of grammars means the TypeScript port can adopt semble's extension → language map (`src/semble/index/files.py`) without per-language audit.
- **Coverage.** 305 languages out of the box (per the package's published description). Sourcing equivalent WASM grammars individually for each language would be a multi-week chore and add a runtime fetcher.
- **Startup cost.** Native parsers load once when the process boots; WASM grammars must be fetched/instantiated per language, which slows the first index of a polyglot repo.
- **Pre-built binaries.** `@kreuzberg/tree-sitter-language-pack` publishes prebuilds for macOS (arm64 / x64), Linux (arm64 / x64, glibc + musl) and Windows (x64). Most users get a binary download, not a node-gyp compile.

## Consequences

### Positive

- Day-1 support for the same language set as semble.
- No WASM loader, no asset bundling for grammars.
- Hot-path parsing runs at native speed (a measurable factor for `csp index` on large repos).

### Negative

- Installs now require a supported platform. Users on exotic targets (FreeBSD, Alpine arm64 without musl prebuilds, sandboxed environments without binary loading) may fail to install. The csp README will list supported platforms explicitly.
- Bun must support loading the NAPI prebuild on the install target. Tested on Bun ≥ 1.3.10 (macOS arm64 / Linux x64).
- The package adds ~50–100 MB to `node_modules` (native binaries × language grammars). This is acceptable for a developer tool but should be documented.

### Neutral

- Future work may introduce an optional WASM fallback for browser / sandboxed environments. That is out of scope for this ADR — the primary distribution path remains native.
- Other native add-ons remain discouraged. Any additional NAPI / node-gyp dependency requires its own ADR.

## Alternatives considered

- **`web-tree-sitter` + curated grammar list.** Rejected: coverage gap vs. semble (would need ~50+ separate `tree-sitter-*-wasm` packages, none of which is a maintained drop-in) and per-language loader complexity.
- **Bun-only tree-sitter via FFI.** Rejected: ties the project to Bun-only, loses Node.js 22+ support promised in `engines`.
- **Wait and write our own chunker without tree-sitter.** Rejected: semble's chunk quality (definition-bounded, comment-attached) depends on AST awareness; a line-window chunker would regress search precision measurably.

## References

- Upstream: `src/semble/chunking/core.py`, `src/semble/index/files.py` in the upstream [MinishLab/semble](https://github.com/MinishLab/semble) repository.
- `@kreuzberg/tree-sitter-language-pack` — <https://www.npmjs.com/package/@kreuzberg/tree-sitter-language-pack>
- Previously stated guideline in `ARCHITECTURE.md` ("No native add-ons") is amended in the same commit that introduces this ADR.
