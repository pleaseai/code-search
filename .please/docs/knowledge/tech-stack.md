# Tech Stack — `@pleaseai/csp`

> Deliberate technology choices and their rationale. Changes here precede implementation per `workflow.md`.

## Language & Runtime

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Language | **TypeScript** (strict + `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes` + `verbatimModuleSyntax`) | The port target is the JS/TS ecosystem; full type coverage matches the typed Python source we're porting from. Extra strict flags catch the classes of bugs (missing nullability, drifted optionals) that haunt search/index code. |
| Target | **ES2022** | `Set`/`Map` iteration order, `.at(-1)`, top-level await, error causes — everything the port needs without polyfills. |
| Module system | **ESM only** (`"type": "module"`) | Bun and Node 22+ are ESM-first. Avoids CJS/ESM dual-package hazards. `verbatimModuleSyntax` enforces explicit `import type`. |
| Primary runtime | **Bun ≥ 1.3.10** | Project-of-record runtime: fastest install, native `bun:test`, `Bun.spawn` for `git clone`, `Bun.file` for streaming reads. Pinned via `packageManager` in `package.json`. |
| Secondary runtime | **Node.js ≥ 22** | Distribution target so `npx @pleaseai/csp` and Node-based MCP harnesses work. CI runs both. |

## Build & Distribution

| Tool | Purpose | Notes |
|------|---------|-------|
| **tsdown** | Bundler / type emitter | Two entries (`src/index.ts`, `src/cli.ts`), `format: 'esm'`, `dts: true`, `unbundle: true`, `clean: true`. Mirrors `@pleaseai/eslint-config`'s build config for consistency. |
| **TypeScript 6.x** | Type checker (`tsc --noEmit` for `bun run typecheck`) | `moduleResolution: bundler` so we can import `.ts` directly during development. |
| Output | `dist/index.mjs`, `dist/cli.mjs`, matching `.d.mts` | `bin: { csp: ./dist/cli.mjs }` in `package.json`. |

## Quality Tooling

| Tool | Purpose | Config |
|------|---------|--------|
| **`@pleaseai/eslint-config`** v0.0.3+ | Lint + format (no Prettier) | Wraps `@antfu/eslint-config`: 2-space, single quotes, **no semicolons**. `lessOpinionated: true`, `top-level-function: 'error'`. Flat config at `eslint.config.ts` with `typescript.tsconfigPath` for type-aware rules. |
| **`bun:test`** | Test runner | No Jest, no Vitest. `bun test path/to/file.test.ts` for single-file. `bun test --watch` for TDD loops. |

## Runtime Dependencies (to be added during porting — placeholders)

> These are the **planned** runtime deps. Pin and document the resolved version when first imported.

| Upstream (Python) | TS port choice | Notes |
|-------------------|----------------|-------|
| `tree-sitter` + `tree-sitter-language-pack` | **`web-tree-sitter`** (WASM) + per-language `tree-sitter-*` wasm modules | Pure-WASM avoids native build steps in Bun and across platforms. Need a lazy-loader for the ~300 languages semble's pack covers — most users only need a handful at a time. |
| `bm25s` | **In-house BM25 implementation** | bm25s relies on SciPy sparse matrices; a typed TS port is small (a few hundred lines) and avoids a heavy dependency. Tokenizer logic stays in `src/tokens.ts`. |
| `model2vec` + `vicinity` | **`@huggingface/transformers`** (or `onnxruntime-web`) for the Model2Vec static lookup; **in-house cosine k-NN** for the vicinity equivalent | Model2Vec at query time is a vocab lookup + mean pool — no transformer forward pass. We need the tokenizer + embedding matrix, both downloadable from the HF hub. |
| `pathspec` (gitignore) | **`ignore`** (npm) | Mature, popular, handles gitignore semantics correctly. |
| `orjson` | Native `JSON` | Bun's `JSON.parse/stringify` is fast enough; orjson's main benefit (Python's slow JSON) doesn't translate. |
| `mcp` (Python SDK) | **`@modelcontextprotocol/sdk`** | Official TS SDK, matches the protocol semble already uses. |
| `watchfiles` | **`chokidar`** or `Bun.watch` | For local-path re-indexing in MCP server mode. |
| CLI parsing | **`cac`** or **`commander`** | Decision deferred; cac is smaller and matches Antfu-ecosystem norms. |

These names are **load-bearing in the public README and CLAUDE.md**; renaming requires updating both.

## Dev Dependencies (current)

- `@pleaseai/eslint-config` — lint
- `eslint` ^10.0.3
- `tsdown` ^0.21.5
- `typescript` ^6.0.2
- `@types/bun` — runtime types for `bun:test`, `Bun.*` APIs

## Repo Layout

Single package (not a monorepo) despite the `pleaseai/code-search` repo name. If a future split into `core` / `cli` / `mcp` packages becomes necessary, the seam is the README's API surface: `CspIndex` and types → core, `csp` binary → cli, MCP server → mcp.

## Out of scope

- **No Prettier** — ESLint handles formatting via the Antfu stylistic plugin.
- **No Jest / Vitest** — `bun:test` only.
- **No CJS build** — ESM-only.
- **No native add-ons** — pure JS / WASM only, to keep installs portable across Linux / macOS / Windows / containers without build toolchains.
