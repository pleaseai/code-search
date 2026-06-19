# ADR 0004 — Rust grammar coverage via `tree-sitter-language-pack` (downloaded parsers)

- **Status**: Accepted
- **Date**: 2026-06-20
- **Deciders**: csp maintainers
- **Relates to**: [ADR 0001](0001-native-tree-sitter.md) (native tree-sitter bindings, the TS side), [ADR 0003](0003-rewrite-in-rust.md) (Rust rewrite & single-binary distribution)
- **Closes**: [#38](https://github.com/pleaseai/code-search/issues/38) — "Rust port: expand tree-sitter grammar coverage to match upstream language pack"

## Context

The Rust port's chunker (`crates/csp/src/chunking/core.rs`) resolved a tree-sitter grammar through `language_for(language)`, which statically linked a **curated set of ~14 grammars** (rust, python, javascript, typescript, tsx, go, java, c, cpp, ruby, json, bash, html, css) via individual `tree-sitter-*` crates.

Upstream semble parses with the Python `tree-sitter-language-pack` (≈all languages). Meanwhile the Rust port's `EXTENSION_TO_LANGUAGE` table (`crates/csp/src/indexing/files.rs`, ~350 entries / 265 distinct language names) recognizes far more languages than the curated grammar set. The effect was a **real behavioral narrowing vs upstream**: a file in a recognized-but-uncurated language (kotlin, swift, php, scala, lua, …) was still walked and indexed, but fell through to line-based chunking instead of AST chunking — coarser, less semantically-aligned chunk boundaries than upstream produces.

The options for closing the gap:

1. **Expanded curated set** — hand-pick and statically link ~30–40 more `tree-sitter-*` crates. Keeps a self-contained offline binary, but only partial parity, grows build time / binary size, and requires a per-language audit (not every grammar is published to crates.io at a compatible version).
2. **`tree-sitter-language-pack` crate (dynamic loading + download)** — adopt the Rust crate published by the same project semble uses. 306 languages, `get_language(name)` maps 1:1 onto `language_for`, and 264 of csp's 265 `EXTENSION_TO_LANGUAGE` names resolve (only `wolfram` is absent). Parsers are fetched from GitHub releases on first use and cached on disk.

## Decision

**Adopt `tree-sitter-language-pack` = "1.9"** with its default features (`dynamic-loading` + `download`) as the canonical grammar source for the Rust chunker. Replace the 14 individual `tree-sitter-*` grammar crates with it.

- `language_for(language)` → `tree_sitter_language_pack::get_language(language).ok()`. Downloads the parser on first use, caches it, then hits the in-process registry. Unknown language or an offline fetch failure → `None` → line fallback (the exact pre-existing degradation contract).
- `is_supported_language(language)` → `tree_sitter_language_pack::has_language(language)`. A **metadata-only** lookup (bundled manifest + aliases) that does **not** download, so `chunk_source` gates AST chunking cheaply before paying for a fetch.
- `tree-sitter` stays a direct dependency (the port drives `Parser`/`Node`/`Language` itself). The crate resolves to the same `tree-sitter 0.26.x`, so the returned `Language` is ABI-compatible.

### Trade-off: single binary vs. runtime grammar cache

ADR-0003 motivation #1 is single-binary distribution. This decision **narrows** that property: the `csp` binary is still a single executable that runs with no Node/Bun present, but it is **no longer fully self-contained / offline** for AST chunking — grammars are fetched from GitHub releases on first use and cached under the OS cache dir (`tree_sitter_language_pack::cache_dir()`; `dirs`-based, e.g. `~/Library/Caches/...` or `~/.cache/...`).

We accept this because:

- **Parity is the point of this work.** A statically-linked subset cannot reach upstream's ≈full coverage without an unbounded crate-audit treadmill; the language pack tracks 306 grammars maintained by the same upstream semble depends on.
- **Degradation is graceful, not fatal.** Offline or fetch-failed → line chunking, exactly what an unsupported language already did. No language regresses below the previous behavior; the previously-curated 14 also just download once and cache.
- **Binary size shrinks** (grammars no longer compiled in) at the cost of a one-time per-language network fetch.

The negatives: first-use latency and a network/GitHub-availability dependency for never-before-seen languages; a writable cache dir is required for AST chunking. These are documented and considered acceptable for a developer tool. A future offline/air-gapped mode could pre-seed the cache via `tree_sitter_language_pack::download(&[...])` or `download_all()`, or pin a `download`-disabled build that links a static subset — out of scope here.

## Consequences

### Positive

- Full upstream-parity AST chunking: 264/265 recognized languages now AST-chunk instead of line-falling-back.
- One dependency replaces 14; coverage tracks upstream without a per-language audit.
- Smaller binary (no compiled-in grammars).

### Negative

- AST chunking now requires a one-time network fetch per language and a writable cache dir; fully offline runs degrade those languages to line chunking until the cache is seeded.
- `cargo test` for real-parse tests needs network → those tests are `#[ignore]`d (run with `cargo test -- --ignored`); the default suite stays offline-green via metadata-only (`has_language`) and fallback assertions.
- One language in csp's extension table (`wolfram`) has no pack grammar and stays on line fallback.

### Neutral

- `chunk_source`'s gate (`is_supported_language` then `chunk`) is unchanged in shape; only the resolver backing it changed.
- An offline/static build mode remains a future option (feature-gated `download`-off build, or cache pre-seeding).

## Alternatives considered

- **Expanded curated static set.** Rejected: partial parity only, ongoing crate-audit burden, and larger build/binary, for a coverage ceiling still well below upstream.
- **Hybrid (static subset + optional language-pack feature).** Rejected for now: doubles the chunker's resolver paths and the test matrix for little benefit over the download model, whose offline degradation already matches the old static fallback. Kept as a future option if an air-gapped build becomes a requirement.

## References

- Issue [#38](https://github.com/pleaseai/code-search/issues/38).
- `tree-sitter-language-pack` — <https://crates.io/crates/tree-sitter-language-pack>, <https://github.com/kreuzberg-dev/tree-sitter-language-pack>.
- Upstream: `src/semble/chunking/core.py`, `src/semble/index/files.py` in [MinishLab/semble](https://github.com/MinishLab/semble).
- `.please/docs/references/semble.md` §4.3 (chunking) and §6.2 (open gaps).
