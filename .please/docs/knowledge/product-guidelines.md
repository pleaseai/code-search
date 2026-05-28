# Product Guidelines — `@pleaseai/csp`

> Style, voice, and product-level conventions. Applies to user-facing surfaces (README, CLI output, MCP descriptions, library JSDoc) and design decisions.

## Voice & Tone

- **Direct and concrete.** State what the tool does, the inputs it takes, and the outputs it returns. Avoid hype words ("blazing", "revolutionary").
- **Numbers when available.** Token savings, latency, recall — quote concrete figures with the caveat that they originate from the upstream semble benchmarks until we re-run them.
- **Symmetric bilingual.** Korean (`README.ko.md`) is not a literal translation — it preserves structure, code blocks, and CLI examples while reading naturally. Update both in the same change.
- **Attribute upstream openly.** Every README references `MinishLab/semble` as the origin and keeps the Zenodo citation. This is not optional decoration; it is the license & ethics floor.

## CLI UX

- **Subcommand naming follows semble** to ease porting of existing AGENTS.md / CLAUDE.md snippets: `search`, `index`, `find-related`, `mcp`, `init`, `savings`.
- **Notable divergence**: `csp mcp` (subcommand) where semble uses the bare binary. README is explicit about this.
- **Flag names follow semble** (`--top-k`, `--content`, `--index`, `--agent`); only diverge with a recorded reason.
- **Default to silence on success.** Print results, not progress chatter, when stdout is piped. Reserve human-friendly formatting for TTY.
- **Errors include the failed input.** "Path does not exist: ./foo" — not "Path not found".
- **Stable JSON output mode** for scripting (`--json` / structured stdout when invoked as a tool).

## API & Code Style

- **Camel-case at the public boundary.** Python's `chunk.file_path / start_line / end_line` becomes `chunk.filePath / startLine / endLine` in the TS surface — this is the explicit, documented translation and external code in the README depends on it.
- **Enums as TS string-literal unions or `as const` objects**, not classes — keep the bundle small and tree-shakeable.
- **No semicolons, single quotes, 2-space indent** — enforced by `@pleaseai/eslint-config`. Don't fight the linter.
- **Comments explain *why*, not *what***. Match the spirit of the upstream semble comments (e.g. the `# Vicinity returns cosine distance; convert to similarity` style — short, load-bearing).
- **No emoji in code, READMEs, or commit messages** unless a user explicitly asks. (Aligned with global Claude Code instructions.)

## Documentation

- **README is the public contract.** When code drifts from README, fix the code, not the README — unless the change is intentional and updated in both.
- **JSDoc on every exported symbol**, mirroring the level of detail in semble's docstrings. The library section of the README is the integration test for the API shape.
- **MCP tool descriptions are concise** (one line per tool) and tell the agent *when* to call, not *how* the algorithm works.
- **No `Common Development Tasks`, `Tips`, `Support` filler sections.** If something is non-obvious, it goes into `gotchas.md`; otherwise it stays out.

## Licensing & Attribution

- MIT for `csp`; the root `LICENSE` covers the project. The README's "Credits" section and Zenodo citation are load-bearing — do not remove or relegate.
- When porting a non-trivial algorithm, leave a comment pointer to the upstream file (e.g. `// Port of src/semble/ranking/boosting.py::_boost_symbol_definitions`) so reviewers can diff against the source of truth.

## Versioning

- **Pre-1.0** (`0.x`): public API may change between minor versions; each minor release notes breaking changes in CHANGELOG.
- **Match `csp` CLI version, library version, and MCP server version** — single `package.json` version is the source of truth across all entrypoints (`csp --version`, `import { version }`, MCP `serverInfo.version`).
