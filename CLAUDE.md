# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project context

`@pleaseai/csp` (binary: `csp`) is a TypeScript/Bun port of [MinishLab/semble](https://github.com/MinishLab/semble), a Python hybrid code-search library for agents. The current repo is an **initial scaffold only** — `src/index.ts` and `src/cli.ts` are placeholders. The README is the canonical spec for the intended public surface (MCP server, CLI, library).

When porting modules from semble, fetch the upstream source via `ask`:

```bash
ask src github:MinishLab/semble@main    # absolute path to the cached checkout
```

Read the Python source there directly — do not infer behavior from the README. Key upstream modules and their target TS counterparts live under `src/semble/` (Python): `types.py`, `tokens.py`, `chunking/`, `index/` (files, file_walker, dense, sparse, create, index), `ranking/` (boosting, penalties, weighting), `search.py`, `mcp.py`, `cli.py`, `cache.py`, `stats.py`, `utils.py`.

## Stack

- **Runtime / package manager**: Bun 1.3.10+ (`packageManager` pinned in `package.json`). Node.js 22+ supported.
- **Module system**: ESM only (`"type": "module"`). Use `.ts` imports with `verbatimModuleSyntax`.
- **Build**: `tsdown` — config at `tsdown.config.ts`, two entries (`src/index.ts`, `src/cli.ts`), `unbundle: true`, emits ESM + DTS into `dist/`.
- **Lint**: `@pleaseai/eslint-config` (wraps `@antfu/eslint-config`). Flat config at `eslint.config.ts`. No semicolons, single quotes, 2-space indent. Type-aware rules enabled via `tsconfigPath`.
- **TypeScript**: strict + `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes` + `verbatimModuleSyntax`. Target ES2022, `moduleResolution: bundler`.
- **Tests**: `bun:test` (no jest/vitest). Run with `bun test`.

## Commands

```bash
bun install        # install deps
bun run build      # tsdown build → dist/
bun run dev        # tsdown --watch
bun run typecheck  # tsc --noEmit
bun run lint       # eslint . --cache
bun run lint:fix   # eslint . --fix --cache
bun test           # bun:test runner
bun test path/to/file.test.ts   # single file
bun test --watch                # watch mode
```

`bunx @pleaseai/csp` is the published-package entrypoint referenced throughout the README (MCP/CLI setup snippets). Locally, use `bun run --bun src/cli.ts` or build first.

## Public API surface (target, from README)

These names are **load-bearing** — they appear in the README's MCP configs, CLI examples, and library usage block, and external users will install against them. Don't rename without updating both READMEs.

- **Library**: `CspIndex` (parallels `SembleIndex`) with `.fromPath()`, `.fromGit()`, `.search()`, `.findRelated()`, `.save()`, `.load()`. Enum `ContentType` with `CODE | DOCS | CONFIG`. Camel-cased fields: `chunk.filePath`, `chunk.startLine`, `chunk.endLine`.
- **CLI** (`csp`): `search`, `index`, `find-related`, `mcp`, `init`, `savings`. Flags: `--top-k`, `--content {code|docs|config|all}`, `--index <path>`, `--agent <claude|cursor|codex|opencode|copilot|kiro|gemini>`.
- **MCP tools**: `search`, `find_related`. Server launched via `bunx @pleaseai/csp mcp` (note `mcp` subcommand — semble uses the bare binary).
- **Stats path**: `~/.csp/savings.jsonl` (semble uses `~/.semble/`).

## Conventions to preserve from semble

- **Hybrid scoring**: dense (Model2Vec embeddings) + BM25, fused with **Reciprocal Rank Fusion** (`k=60`), not raw-score blending. `alpha` weights apply to RRF-normalized scores.
- **Adaptive alpha**: `is_symbol_query()` chooses `_ALPHA_SYMBOL=0.3` (BM25-leaning) vs `_ALPHA_NL=0.5` for NL queries.
- **BM25 enrichment**: chunk text is augmented with `{stem} {stem} {last 3 dir parts}` before tokenization (`enrich_for_bm25`). Stem is repeated twice to up-weight path matches.
- **Tokenization**: identifier-aware — `camelCase`/`PascalCase`/`snake_case` are split into sub-tokens *plus* the original lowercased compound. See `src/semble/tokens.py`.
- **Chunking**: tree-sitter AST-based with line-fallback when language is unsupported. Target chunk length is 1500 chars; `_MIN_CHUNK_SIZE=50` prevents recursion into tiny nodes; `_RECURSION_DEPTH=500` guards pathological ASTs.
- **Ranking pipeline order** (in `search.search`): semantic + BM25 → RRF → multi-chunk file boost → query-type boost (definition / stem / embedded-symbol) → top-k rerank with path penalties + file-saturation decay (`_FILE_SATURATION_DECAY=0.5` per extra chunk beyond 1 per file).
- **Path penalties**: test files (`_STRONG_PENALTY=0.3`), `__init__.py`/barrels (`_MODERATE_PENALTY=0.5`), `.d.ts` (`_MILD_PENALTY=0.7`), compat/examples dirs (`_STRONG_PENALTY`). Apply only when `alpha_weight < 1.0` (i.e., BM25 contributing).
- **File walking**: respect `.gitignore` *and* `.sembleignore` (port as `.cspignore`). Default-ignored dirs include `.git`, `node_modules`, `dist`, `build`, `.next`, plus add `.csp/` (replacement for `.semble/`).

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
<!-- /please:knowledge -->

