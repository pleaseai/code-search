# Product Guide — `@pleaseai/csp`

> Stable product context. Reviewed and revised when scope or audience changes.

## Vision

`csp` (Code Search Please) gives AI coding agents instant, token-efficient access to any codebase through natural-language or symbol queries. It returns the precise code snippets the agent needs in milliseconds, on CPU, without API keys or external services — replacing the grep + read loop that dominates agent context windows today.

`csp` is a TypeScript/Bun port of [MinishLab/semble](https://github.com/MinishLab/semble), bringing the same hybrid (dense + BM25) code-search experience to the JavaScript / TypeScript ecosystem.

## Target Users

| User | What they need |
|------|----------------|
| **AI coding agents** (Claude Code, Cursor, Codex, OpenCode, Copilot CLI, Gemini CLI, Zed, VS Code, Kiro, Windsurf) | An MCP server that takes a natural-language query and returns ranked code snippets — no grep, no full-file reads. |
| **Developers writing agent harnesses / sub-agents** | A library API (`CspIndex.fromPath`, `.search`, `.findRelated`) to embed code search into custom flows. |
| **Developers writing scripts and CI tooling** | A standalone CLI (`csp search`, `csp index`, `csp find-related`) callable from any shell or pipeline. |
| **Open-source authors maintaining JS/TS-first projects** | A drop-in equivalent of `semble` that fits a Bun/Node toolchain (no Python or `uv` requirement). |

## Goals (success criteria)

1. **Token efficiency parity with semble**: ~98% fewer tokens than `grep + read` at equivalent recall.
2. **End-to-end latency under a second** for typical repos: indexing well under 1s, queries in single-digit ms.
3. **Zero external dependencies at runtime**: no API keys, no GPU, no remote inference. CPU-only via static Model2Vec embeddings.
4. **API surface compatibility with the published README** so MCP configs, AGENTS.md snippets, and library code shown to users work as documented from day one of the first feature-complete release.
5. **First-class Bun support** (lockfile, `bun:test`, `bunx` entrypoints) with Node.js 22+ as a tested fallback.

## Non-goals

- **Custom embedding models / training**: `csp` ships the upstream `potion-code-16M` static model; it is not a platform for arbitrary embedding pipelines.
- **A general-purpose vector database**: scope is in-process code search for a single repo (local or git URL), not multi-tenant retrieval.
- **Replacing grep**: grep remains the right tool for exhaustive literal matches; `csp` is the right tool for semantic / symbol / NL queries.
- **GPU acceleration**: ruled out — CPU-only is a feature, not a limitation to overcome.
- **GUI / web app**: CLI + MCP + library only.

## Constraints

- **Derivative work**: `csp` is MIT, derived from MIT-licensed `MinishLab/semble`. Algorithmic decisions, default parameters (`alpha=0.3/0.5`, RRF `k=60`, chunk length 1500, file-saturation decay 0.5, etc.) follow the upstream unless there is a clear reason to diverge — divergence must be recorded as an ADR.
- **README is the public contract**: `README.md` and `README.ko.md` document the intended MCP commands, CLI flags, library symbols, and stats path (`~/.csp/savings.jsonl`). Renames or signature changes require updating both READMEs in the same change.
- **Bilingual documentation**: every user-facing doc that exists in English must exist in Korean (or vice versa). README pair is the canonical example.
- **Algorithmic ports must read the original Python source**, not summaries or memory. Use `ask src github:MinishLab/semble@main` to fetch the upstream and read it before writing TS.

## Out-of-scope (for now, may revisit)

- IDE plugins beyond MCP integration.
- Multi-repo / monorepo-aware indexing where a single index spans multiple roots.
- Persistent server mode beyond the MCP session lifetime.
- Telemetry / analytics beyond local `~/.csp/savings.jsonl`.
