# Architecture

> Agent-first ARCHITECTURE.md for `@pleaseai/csp` (binary: `csp`) — a TypeScript / Bun port of [MinishLab/semble](https://github.com/MinishLab/semble).
>
> **Status**: this document describes the **target architecture** for the port. The current tree (`src/index.ts`, `src/cli.ts`) is scaffolding; modules marked *(planned)* below have not been ported yet. The shape is committed: the README is the public contract and this document is the internal contract that backs it.

## System Overview

**Purpose**: Give AI coding agents instant, token-efficient access to any codebase through natural-language or symbol queries, returning ranked code snippets in milliseconds on CPU — without API keys, GPU, or external services.

**Primary users**:

- **AI coding agents** (Claude Code, Cursor, Codex, OpenCode, Copilot CLI, Gemini CLI, Zed, VS Code, Kiro, Windsurf) — via the **MCP server** transport.
- **Developers writing agent harnesses or scripts** — via the **CLI** (`csp search`, `csp index`, `csp find-related`).
- **Developers embedding code search into their own TypeScript code** — via the **library** API (`CspIndex.fromPath`, `.search`, `.findRelated`).

**Core workflow** (the `search` happy path):

1. **Resolve input**: caller (MCP tool / CLI / library) supplies a local directory or a git URL plus a query string.
2. **Index (cached)**: walk files honouring `.gitignore` / `.cspignore`, chunk each file with tree-sitter (line-fallback when the language has no parser), build a BM25 index over identifier-aware tokens and a dense Model2Vec embedding matrix in parallel.
3. **Score**: run BM25 and dense retrieval against the query, over-fetch candidates (`top_k * 5`), normalize each list with **Reciprocal Rank Fusion** (`k = 60`), and blend with adaptive `alpha` (`0.3` for symbol queries, `0.5` for natural-language queries).
4. **Rerank**: apply multi-chunk file boost, query-type boost (definition / stem / embedded symbol), then path-penalised top-k rerank with file-saturation decay.
5. **Return** the top-k `SearchResult` records; the MCP / CLI layer formats them and writes savings telemetry to `~/.csp/savings.jsonl`.

**Key constraints**:

- **CPU only**: no GPU, no remote inference, no transformer forward pass at query time. Model2Vec is a vocab → embedding lookup + pooled aggregation.
- **End-to-end under a second** on typical repos: index in well under 1s, queries in single-digit ms.
- **No network at query time**: after one-off model download, indexing and searching are fully offline.
- **API surface stability**: every exported name, CLI flag, and MCP tool listed in `README.md` / `README.ko.md` is load-bearing; renames touch both READMEs in the same change.

## Dependency Layers

Dependencies flow downward only. Lower layers must not import upper layers. The MCP server and CLI are siblings on the Interface layer; neither imports the other.

```
┌─────────────────────────────────────────────────────────────────┐
│                       Interface Layer                            │
│                                                                  │
│   src/cli.ts            src/mcp/                                 │
│   (csp subcommands)     (MCP server: search, find_related tools) │
├─────────────────────────────────────────────────────────────────┤
│                      Application Layer                           │
│                                                                  │
│   src/index.ts          CspIndex orchestration                   │
│   (public API barrel)   (fromPath / fromGit / save / load)       │
├─────────────────────────────────────────────────────────────────┤
│                        Domain Layer                              │
│                                                                  │
│   src/types.ts          Chunk, SearchResult, ContentType         │
│   src/search.ts         Hybrid RRF + alpha blend                 │
│   src/ranking/          weighting, boosting, penalties           │
│   src/tokens.ts         identifier-aware tokenizer (BM25 input)  │
│   src/chunking/         tree-sitter chunking + line fallback     │
├─────────────────────────────────────────────────────────────────┤
│                    Infrastructure Layer                          │
│                                                                  │
│   src/indexing/         file walker, BM25 build, dense embed     │
│   src/cache.ts          disk persistence (hash-keyed)            │
│   src/stats.ts          ~/.csp/savings.jsonl                     │
│   src/utils.ts          git URL detection, chunk resolution      │
└─────────────────────────────────────────────────────────────────┘
```

**Invariant — domain has no I/O**: `src/search.ts`, `src/ranking/*`, `src/tokens.ts`, and `src/chunking/*` are pure: they take in-memory inputs (`Chunk[]`, embedding matrix, BM25 index handle, query string) and return scored results. They do not read files, spawn processes, or call the network. This makes them unit-testable with fixtures and keeps the ranking pipeline auditable against the upstream Python source.

**Invariant — interfaces do not bypass application**: `src/cli.ts` and `src/mcp/` only call into `CspIndex`. They never import `src/search.ts` or `src/ranking/*` directly. Telemetry (`stats.ts`) is invoked by `CspIndex`, not by the interface layer.

## Entry Points

For **understanding the public API and where everything starts**:

- **`README.md`** / **`README.ko.md`** — The public contract: MCP configs, CLI commands, library snippets, stats path. Read this before reading code; the README defines the names and shapes that the rest of the codebase exists to fulfil.
- **`src/index.ts`** — The library barrel. Re-exports `CspIndex`, `Chunk`, `SearchResult`, `ContentType`, and `version`. The shortest path from "what does the package expose?" to the answer.

For **understanding a `search` call end-to-end**:

- **`src/cli.ts` → `csp search` subcommand** — Shows argument parsing, content-type resolution, path/URL dispatch, and the call into `CspIndex.fromPath(...).search(...)`.
- **`src/indexing/index.ts` → `CspIndex` *(planned)*** — Orchestrator: holds `model`, `bm25Index`, `semanticIndex`, `chunks`, builds the file/language mapping, and runs `search()` + `findRelated()`. This is the seam between "I have a repo" and "I have ranked results".
- **`src/search.ts` *(planned)*** — The hybrid scoring pipeline: dense retrieval + BM25 → RRF normalize → alpha-weighted blend → multi-chunk file boost → query boost → path-penalised top-k rerank. Mirrors `src/semble/search.py` in the upstream.

For **understanding indexing**:

- **`src/indexing/create.ts` *(planned)*** — Walks files, chunks with tree-sitter (or line fallback), embeds, and builds BM25. The `bm25_index, semantic_index, chunks = create_index_from_path(...)` tuple in `src/semble/index/create.py` is the model.
- **`src/chunking/core.ts` *(planned)*** — The 1500-char tree-sitter chunker with recursion-depth guards and `_MIN_CHUNK_SIZE` floor.

For **understanding ranking decisions** (the heart of why csp beats grep):

- **`src/ranking/weighting.ts` *(planned)*** — Adaptive `alpha` resolution (`is_symbol_query()` → 0.3 / 0.5).
- **`src/ranking/boosting.ts` *(planned)*** — Definition boost, identifier-stem boost, embedded-symbol boost, file-coherence boost.
- **`src/ranking/penalties.ts` *(planned)*** — Path penalties (test files, barrels, compat/legacy, examples, `.d.ts`) + file-saturation decay during top-k selection.

## Module Reference

| Module                       | Purpose                                                                            | Key Files                                       | Depends On                                  | Depended By                                |
| ---------------------------- | ---------------------------------------------------------------------------------- | ----------------------------------------------- | ------------------------------------------- | ------------------------------------------ |
| `src/index.ts`               | Public library barrel: re-exports the documented API surface.                      | `index.ts`                                      | `types`, `indexing`                         | (consumers via `import '@pleaseai/csp'`)   |
| `src/cli.ts`                 | `csp` binary entrypoint. Subcommands: `search`, `index`, `find-related`, `mcp`, `init`, `savings`. | `cli.ts`                            | `indexing`, `mcp` *(planned)*, `stats`      | (end users via `bin: csp`)                 |
| `src/mcp/` *(planned)*       | MCP server exposing `search` and `find_related` tools over stdio.                  | `mcp/server.ts`                                 | `indexing`, `@modelcontextprotocol/sdk`     | `cli.ts` (`csp mcp` subcommand)            |
| `src/types.ts` *(planned)*   | Public types: `Chunk`, `SearchResult`, `ContentType`, `IndexStats`, `CallType`.    | `types.ts`                                      | —                                           | every other module                         |
| `src/tokens.ts` *(planned)*  | Identifier-aware tokenizer for BM25: camel/Pascal/snake splitting + original compound. | `tokens.ts`                                 | —                                           | `indexing`, `search`, `ranking/boosting`   |
| `src/chunking/` *(planned)*  | Tree-sitter AST chunking (1500-char target) with line fallback when no parser.     | `chunking/core.ts`, `chunking/chunk-source.ts`  | `web-tree-sitter`, language wasm modules    | `indexing/create.ts`                       |
| `src/indexing/` *(planned)*  | `CspIndex` orchestration: file walker, language detection, BM25 build, dense embed, persistence. | `indexing/index.ts`, `indexing/create.ts`, `indexing/files.ts`, `indexing/file-walker.ts`, `indexing/dense.ts`, `indexing/sparse.ts` | `chunking`, `tokens`, `ignore`, embedding backend | `cli.ts`, `mcp/`                  |
| `src/search.ts` *(planned)*  | Hybrid RRF + alpha-weighted blend of dense and BM25 results.                       | `search.ts`                                     | `types`, `ranking`, `tokens`                | `indexing/index.ts`                        |
| `src/ranking/` *(planned)*   | Code-aware reranking: weighting, query/file boosting, path penalties.              | `ranking/weighting.ts`, `ranking/boosting.ts`, `ranking/penalties.ts` | `types`, `tokens`     | `search.ts`                                |
| `src/cache.ts` *(planned)*   | On-disk index cache (hash-keyed by repo state); invalidation on file change.       | `cache.ts`                                      | `indexing` (for serialisation), `node:fs`   | `cli.ts`, `mcp/`                           |
| `src/stats.ts` *(planned)*   | Token-savings telemetry — append-only `~/.csp/savings.jsonl`; `csp savings` reader. | `stats.ts`                                     | `node:fs`                                   | `indexing/index.ts` (write), `cli.ts` (read) |
| `src/utils.ts` *(planned)*   | `isGitUrl`, `resolveChunk(file, line)`, `formatResults`.                           | `utils.ts`                                      | `types`                                     | `cli.ts`, `mcp/`                           |
| `src/agents/` *(planned)*    | Sub-agent prompts shipped to `csp init` (per-harness markdown).                    | `agents/claude.md`, `agents/cursor.md`, …       | —                                           | `cli.ts` (`csp init`)                      |

## Architecture Invariants

**Hybrid scoring uses RRF, not raw-score blending.** Both the dense and BM25 result lists are converted to `1 / (k + rank)` scores with `k = 60` *before* the alpha blend. This is what makes `alpha` independent of raw-score magnitudes across queries. Do NOT replace this with a min-max normalization or a raw-score weighted sum — the alpha defaults (0.3 / 0.5) are tuned against RRF.

**Public field names are camelCase, not snake_case.** The upstream Python uses `chunk.file_path`, `start_line`, `end_line`. The TypeScript port exposes `chunk.filePath`, `startLine`, `endLine`. The READMEs document these explicitly. Do NOT introduce snake_case at the public boundary "for parity"; only port-internal helpers may match upstream names verbatim.

**Adaptive alpha is the default; explicit alpha is opt-in.** `resolveAlpha(query, alpha)` returns the user-supplied alpha when provided, else `0.3` for symbol-like queries and `0.5` for NL queries. Do NOT remove the auto-detection — the upstream benchmarks assume it.

**Path penalties only apply when BM25 is contributing.** In `rerankTopK`, the `penalisePaths` argument must be `alpha < 1.0`. Pure-semantic queries (`alpha = 1.0`) skip path penalties because they cannot signal file-type intent. This mirrors the upstream behavior.

**File-saturation decay caps results per file at 1 before damping.** `_FILE_SATURATION_THRESHOLD = 1`, `_FILE_SATURATION_DECAY = 0.5`. The second chunk from the same file is multiplied by 0.5, the third by 0.25, and so on. Do NOT raise the threshold to "show more from the best file" — file coherence is already boosted earlier in the pipeline.

**MCP tool descriptions tell the agent *when* to call, not *how* the algorithm works.** Reference: `src/semble/mcp.py`. Long descriptions waste agent context.

**Algorithmic ports must read the original Python source, not memory.** Use `ask src github:MinishLab/semble@main` and read the relevant `src/semble/*.py` before writing TypeScript. When porting a non-trivial function, leave a `// Port of src/semble/<path>::<name>` comment so reviewers can diff against the source of truth.

**No native add-ons.** Tree-sitter must be `web-tree-sitter` (WASM), not `node-tree-sitter`. This keeps installs portable across Linux / macOS / Windows / containers without C toolchains, and works under Bun where many node-gyp packages still misbehave.

**Bilingual README must stay in sync.** Any public-API change (CLI flag, library symbol, MCP tool, config option, stats path) updates **both** `README.md` and `README.ko.md` in the same commit. The CLAUDE.md captures this as load-bearing.

**No `Common Development Tasks`, `Tips`, `Support` filler in docs.** Per `.please/docs/knowledge/product-guidelines.md`: anything non-obvious goes into `.please/docs/knowledge/gotchas.md`, not into README chatter.

## Cross-Cutting Concerns

**Error handling**:

- **User-input errors** (path missing, invalid content type, malformed query) surface as concrete messages — e.g. `Path does not exist: ./foo` — at the interface layer (CLI / MCP). The interface layer translates exceptions into exit codes (CLI) or JSON-RPC error responses (MCP).
- **Library callers** see typed exceptions: `Error` subclasses with descriptive names (`PathNotFoundError`, `GitCloneError`, `InvalidIndexError`). The Python upstream uses `FileNotFoundError`, `NotADirectoryError`, `RuntimeError`; the TS port may collapse these into a smaller set of `csp`-specific errors but the messages stay informative.
- **Domain layer never swallows errors**: ranking and search must propagate, not "return empty results" silently. Empty inputs (`!chunks.length || !query.trim()`) return `[]` explicitly at the top of `CspIndex.search()`.

**Logging**:

- The library logs **nothing by default** — `console.log` and `process.stderr.write` are reserved for the interface layer.
- The CLI prints human-friendly output to stdout and progress/diagnostics to stderr. Stdout stays machine-parseable when piped; reserve TTY formatting for `process.stdout.isTTY`.
- The MCP server uses **stderr only** for diagnostics (per MCP spec — stdout is the JSON-RPC channel).
- No structured logger / `pino` / `winston` dependency. If a project debug mode is needed later, a tiny `debug`-style namespace via `node:util.debuglog('csp:*')` is the path; it stays optional and zero-cost when disabled.

**Testing**:

- **Runner**: `bun:test` exclusively. No Jest, no Vitest. `bun test path/to/file.test.ts` runs a single file; `bun test --watch` for TDD loops.
- **Layout**: co-located `*.test.ts` next to sources (`src/tokens.test.ts` next to `src/tokens.ts`). End-to-end / integration tests under `tests/` with fixture repos in `tests/fixtures/`.
- **Domain tests are pure**: build a `Chunk[]` array in-memory, run the pipeline, assert on the ordered output. No filesystem, no model loading.
- **Indexing tests** use small fixture repos (`tests/fixtures/sample-ts-project/`). One repo per content-type matrix entry (code / docs / config).
- **MCP integration tests** spawn the server via `Bun.spawn`, write JSON-RPC frames to stdin, assert responses on stdout.
- **Coverage target**: >80% for new code per `.please/docs/knowledge/workflow.md`. `bun test --coverage`.

**Configuration**:

- **No runtime config file.** Behavior is controlled by CLI flags, MCP server args (`--content code docs`), and library options. This matches semble and keeps `csp` deployable without filesystem state beyond `~/.csp/savings.jsonl`.
- **Environment variables**: `SEMBLE_CLONE_TIMEOUT` (60s default for `git clone`) ports to `CSP_CLONE_TIMEOUT`. Add new env vars sparingly and document them next to where they are read, not in a central config module.
- **Model cache** lives where the embedding backend places it (HuggingFace cache for `@huggingface/transformers`). This is intentional — sharing cache across tools (`csp`, other transformers-based projects) is a feature, not leakage.
- **Stats file** (`~/.csp/savings.jsonl`) is append-only, JSON-Lines, one record per search/find_related call. The reader (`csp savings`) tolerates missing or partial files.

## Quality Notes

**Well-tested (will be)**:

- `src/tokens.ts` and `src/ranking/*` are pure functions with small, deterministic outputs — the easiest places to land high coverage and the highest-leverage places to *have* high coverage because they drive ranking decisions.
- `src/types.ts` is type-only; covered transitively by every test that touches `Chunk` / `SearchResult`.

**Fragile (handle with care)**:

- **`src/chunking/core.ts`**: tree-sitter integration straddles WASM lifecycle, language-pack download, and AST recursion. Mirror the upstream's `_RECURSION_DEPTH = 500` and `_MIN_CHUNK_SIZE = 50` guards exactly; deviations have cascade effects on ranking input.
- **`src/indexing/file-walker.ts`**: gitignore semantics (`pathspec.GitIgnoreSpec.from_lines(..., backend="simple")` upstream) have edge cases around negation patterns and directory vs file matching. Port the `_is_ignored` "negation + extension suffix → bypass extension filter" rule verbatim and unit-test it against the same fixtures the upstream uses.
- **`src/search.ts` candidate over-fetch**: `topK * 5` is load-bearing for rerank-after-blend to have room to move. Changing this multiplier without re-running benchmarks risks recall regressions that won't show up in unit tests.

**Technical debt** (current):

- The current `src/index.ts` and `src/cli.ts` are placeholders that satisfy the `tsdown` build but do not implement anything. They will be replaced as modules land.
- No CI yet — `.github/workflows/` needs to land before the first port PR.
- No CHANGELOG — the README references "0.x: public API may change between minor versions; each minor release notes breaking changes in CHANGELOG" but the file does not exist yet.
- The bilingual README sync is a manual discipline; a lint or test that diffs section anchors between `README.md` and `README.ko.md` would close the gap.

---

_Last updated: 2026-05-28 — initial ARCHITECTURE.md alongside scaffolding._

_Related project context:_

- `README.md` / `README.ko.md` — Public contract (MCP / CLI / library)
- `CLAUDE.md` — Project context for AI agents working on this repo
- `.please/docs/knowledge/product.md` — Vision, target users, goals
- `.please/docs/knowledge/tech-stack.md` — Technology choices with rationale
- `.please/docs/knowledge/workflow.md` — TDD, quality gates, dev commands
- `.please/docs/decisions/` — ADRs (none yet; document divergence-from-semble decisions here as they arise)
