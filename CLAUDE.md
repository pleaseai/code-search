# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project context

`@pleaseai/csp` (binary: `csp`) is a **Rust** port of [MinishLab/semble](https://github.com/MinishLab/semble), a Python hybrid code-search library for agents. The implementation lives in `crates/csp` (library) + `crates/csp-cli` (`csp` binary). The README is the canonical spec for the public surface (MCP server, CLI, library).

The deprecated TypeScript implementation that formerly lived under `src/` has been **removed** — the Rust port is the only implementation. The root `package.json` / `tsconfig.json` / `eslint.config.ts` remain only as repo JS tooling (lint/typecheck of `npm/`) and the release-please version anchor. The **napi-rs native-binding SDK** binds the `crates/` Rust directly (it does not reintroduce a TS port).

**SDK packaging decision: keep the two distribution channels separate.** `npm/` stays the Biome-style optional-dependency launcher for the **standalone Rust binary** (this preserves the no-runtime Homebrew story; do NOT convert it to napi). It uses the esbuild **copy-over-shim**: a `postinstall` (`npm/csp/install.js`) copies the resolved platform binary over the `bin/csp.js` Node launcher so `.bin/csp` execs native code directly (no Node process on the hot path, ~10× faster startup); the Node launcher remains only as a fallback when postinstall is skipped (e.g. bun blocks it unless `@pleaseai/csp` is in `trustedDependencies`). Shared platform resolution lives in `npm/csp/lib/resolve.js` (never overwritten, so re-running postinstall is idempotent). The napi-rs SDK is a distinct concern: `crates/csp-node` holds `#[napi]` bindings over `crates/csp` and is shipped as its **own npm package** (`@pleaseai/csp-sdk`), an in-process native addon — not merged into `npm/`. Both build outputs share the one `crates/csp` core. The SDK is in place: `#[napi]` bindings (`fromPath`/`fromGit`/`loadFromDisk` are async on the libuv worker pool; `search`/`findRelated`/`save`/`stats` sync, with `inner` held behind `Arc` to enable a future async move), the `napi build` toolchain (`.node` + `index.js`; `index.d.ts` is the committed type surface), and the cross-compile + Trusted-Publishing release in `release-sdk.yml`. The remaining step is publish-only — a maintainer must configure the npm trusted publisher for `@pleaseai/csp-sdk` + its platform packages (see `crates/csp-node/README.md`).

### Rust port (ADR-0003)

**The Python upstream ([MinishLab/semble](https://github.com/MinishLab/semble)) is the source of truth** — the Rust port targets behavioral equivalence with the upstream Python.
- Quality gate before every Rust commit: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace`.
- Parity oracle = the **Python upstream** behavior (read the source directly — see the fetch note below). Dense embeddings are real (`model2vec-rs`), the ranking pipeline is wired (query boosts + path penalties + file saturation), and the chunk length is `750`.
- CLI/MCP output is a **snake_case** wire dict (`csp::utils::format_results`, mirroring upstream `SearchResult.to_dict`), distinct from the camelCase `ChunkDict` used for on-disk persistence.
- rmcp 1.7: the default `#[tool_handler]` rebuilds the router via `Self::tool_router()` and leaves a stored `tool_router` field unread (clippy `dead_code`) — use `#[tool_handler(router = self.tool_router)]`.

When porting modules from semble, fetch the upstream source and read the Python directly:

```bash
ask src github:MinishLab/semble@main    # absolute path to the cached checkout (if `ask` is installed)
# Fallback when `ask` is unavailable — fetch a raw file straight from GitHub:
curl -fsSL https://raw.githubusercontent.com/MinishLab/semble/main/src/semble/search.py
```

Read the Python source directly — do not infer behavior from the README. Key upstream modules (mapped to their `crates/csp` Rust counterparts in `.please/docs/references/semble.md`) live under `src/semble/` (Python): `types.py`, `tokens.py`, `chunking/`, `index/` (files, file_walker, dense, sparse, create, index), `ranking/` (boosting, penalties, weighting), `search.py`, `mcp.py`, `cli.py`, `cache.py`, `stats.py`, `utils.py`.

## Stack

The implementation is **Rust** (a Cargo workspace). A thin Node/Bun toolchain remains for repo-level JS lint/typecheck and the future napi-rs SDK.

- **Impl**: Rust, edition 2021. Cargo workspace (`crates/csp` lib + `crates/csp-cli` `csp` binary), toolchain pinned by `rust-toolchain.toml`. Single-binary release profile (`lto`, `codegen-units=1`, `strip`).
- **Tests**: `cargo test --workspace` (255+ lib + CLI tests). Network-gated grammar-fetch tests run with `-- --ignored` (see ADR-0004).
- **Distribution**: self-contained Rust binary via Homebrew (`pleaseai/homebrew-tap`) + an npm wrapper under `npm/` that preserves the `bunx @pleaseai/csp` entrypoint.
- **JS tooling** (no TS implementation): Bun ≥1.3.10 / Node ≥22 (the `engines` floor; `mise.toml` pins 1.3.14 / 24 for dev + CI). `@pleaseai/eslint-config` (wraps `@antfu/eslint-config`) lints `npm/` JS + `eslint.config.ts`; `tsc --noEmit` typechecks. No semicolons, single quotes, 2-space indent.
- **Toolchain manager**: `mise.toml` pins `node`/`bun` + `hk` (the git hook manager); the Rust channel stays owned by `rust-toolchain.toml`. `mise install` provisions tools and runs `hk install --mise`, wiring git hooks from `hk.pkl` (pre-commit: eslint on `npm/` JS + `rustfmt` on staged `.rs`; commit-msg: conventional-commit check). `mise run check` is the full local gate. On Intel macOS hk is pinned via the `cargo:` backend (aqua has no darwin-amd64).

## Commands

```bash
# Rust (the implementation)
cargo build --release                          # → target/release/csp
cargo run -p csp-cli -- search "query" .       # run the CLI locally
cargo test --workspace                         # test runner
cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings   # pre-commit gate

# JS tooling (lint/typecheck of npm/ + configs; no TS sources to build)
bun install        # install dev tooling
bun run typecheck  # tsc --noEmit
bun run lint       # eslint . --cache
bun run lint:fix   # eslint . --fix --cache
```

`bunx @pleaseai/csp` is the published-package entrypoint referenced throughout the README (MCP/CLI setup snippets); it resolves to the npm wrapper (`npm/`) that execs the Rust binary.

## Public API surface (target, from README)

These names are **load-bearing** — they appear in the README's MCP configs, CLI examples, and library usage block, and external users will install against them. Don't rename without updating both READMEs.

- **Library**: `CspIndex` (parallels `SembleIndex`) with `.fromPath()`, `.fromGit()`, `.search()`, `.findRelated()`, `.save()`, `.loadFromDisk()`. Enum `ContentType` with `CODE | DOCS | CONFIG`. Camel-cased fields: `chunk.filePath`, `chunk.startLine`, `chunk.endLine`.
- **CLI** (`csp`): `search`, `index`, `find-related`, `mcp`, `init`, `savings`. Flags: `--top-k`, `--content {code|docs|config|all}`, `--index <path>`, `--agent <claude|cursor|codex|opencode|copilot|kiro|gemini>`.
- **MCP tools**: `search`, `find_related`. Server launched via `bunx @pleaseai/csp mcp` (note `mcp` subcommand — semble uses the bare binary).
- **Stats path**: `~/.csp/savings.jsonl` (semble uses `~/.semble/`). The global index cache lives alongside it at `~/.csp/index/` (per [ADR 0002](.please/docs/decisions/0002-index-storage-cache-model.md)); `csp clear index` removes only that directory.

## Conventions to preserve from semble

- **Hybrid scoring**: dense (Model2Vec embeddings) + BM25, fused with **Reciprocal Rank Fusion** (`k=60`), not raw-score blending. `alpha` weights apply to RRF-normalized scores.
- **Adaptive alpha**: `is_symbol_query()` chooses `_ALPHA_SYMBOL=0.3` (BM25-leaning) vs `_ALPHA_NL=0.5` for NL queries.
- **BM25 enrichment**: chunk text is augmented with `{stem} {stem} {last 3 dir parts}` before tokenization (`enrich_for_bm25`). Stem is repeated twice to up-weight path matches.
- **Tokenization**: identifier-aware — `camelCase`/`PascalCase`/`snake_case` are split into sub-tokens *plus* the original lowercased compound. See `src/semble/tokens.py`.
- **Chunking**: tree-sitter AST-based with line-fallback when language is unsupported. Target chunk length is 750 chars (`DESIRED_CHUNK_LENGTH_CHARS`, matching upstream `chunking/chunking.py`; the deprecated TS `src/` still carries the older 1500); `_MIN_CHUNK_SIZE=50` prevents recursion into tiny nodes; `_RECURSION_DEPTH=500` guards pathological ASTs.
- **Ranking pipeline order** (in `search.search`): semantic + BM25 → RRF → multi-chunk file boost → query-type boost (definition / stem / embedded-symbol) → top-k rerank with path penalties + file-saturation decay (`_FILE_SATURATION_DECAY=0.5` per extra chunk beyond 1 per file).
- **Path penalties**: test files (`_STRONG_PENALTY=0.3`), `__init__.py`/barrels (`_MODERATE_PENALTY=0.5`), `.d.ts` (`_MILD_PENALTY=0.7`), compat/examples dirs (`_STRONG_PENALTY`). Apply only when `alpha_weight < 1.0` (i.e., BM25 contributing).
- **File walking**: respect `.gitignore` *and* `.sembleignore` (port as `.cspignore`). Default-ignored dirs include `.git`, `node_modules`, `dist`, `build`, `.next`, plus add `.csp/` (replacement for `.semble/`). Note: the canonical **index cache** is no longer repo-local — per [ADR 0002](.please/docs/decisions/0002-index-storage-cache-model.md) it moved to the global `~/.csp/index/`. The repo-local `.csp/` entry stays in the default-ignore list for any local artifacts, but the index cache itself is global.

## README is bilingual

`README.md` (English) and `README.ko.md` (한국어) must stay in sync. Both link to each other at the top. The Korean version is not a literal translation — it preserves the same structure and code blocks but reads naturally. When editing one, edit the other.

## Credits / licensing

This is a derivative work. Keep the "Credits" section and the Semble Zenodo citation intact in both READMEs. Both projects are MIT — root `LICENSE` covers csp; the README explicitly attributes the upstream.

## Repo layout note

Single package, not a monorepo (despite the `pleaseai/code-search` repo name). If a future split into core / cli / mcp packages is needed, the README's API surface is the seam — `CspIndex` and types go to core, `csp` binary to cli, MCP server to mcp.

<!-- please:knowledge v1 -->
## Project Knowledge

Consult these files for project context before exploring the codebase.
For full file listing with workspace artifacts, use `Skill("please:project-knowledge")`.

### Project Documents
- `README.md` / `README.ko.md` — Public spec for MCP / CLI / library surface (bilingual; must stay in sync)

### Domain Knowledge (.please/docs/knowledge/)
- `product.md` — Product vision, target users, goals, non-goals
- `product-guidelines.md` — Voice, CLI UX, API style, attribution rules
- `tech-stack.md` — Technology choices with rationale (TS / Bun / tsdown / `@pleaseai/eslint-config`)
- `workflow.md` — Task lifecycle, TDD, quality gates, dev commands, stacked PR strategy

### Decision Records
- `.please/docs/decisions/` — Architecture Decision Records (ADR)

### Reference Analyses (.please/docs/references/)
- `index.md` — index of upstream-library analyses (scales as more libraries are adopted)
- `semble.md` — module-by-module analysis of MinishLab/semble mapped to the **Rust port** (`crates/csp`), with algorithms, constants, and drift tracking
<!-- /please:knowledge -->

