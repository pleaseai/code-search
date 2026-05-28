<h2 align="center">
  csp — Code Search Please<br/>
  Fast and Accurate Code Search for Agents<br/>
  <sub>Uses ~98% fewer tokens than grep+read</sub>
</h2>

<div align="center">
  <h2>
    <a href="https://www.npmjs.com/package/@pleaseai/csp"><img src="https://img.shields.io/npm/v/@pleaseai/csp?color=%23007ec6&label=npm%20package" alt="Package version"></a>
    <a href="https://github.com/pleaseai/code-search/blob/main/LICENSE">
      <img src="https://img.shields.io/badge/license-MIT-green" alt="License - MIT">
    </a>
  </h2>

English | [한국어](./README.ko.md)

[Quickstart](#quickstart) •
[MCP Server](#mcp-server) •
[AGENTS.md](#agentsmd) •
[CLI](#cli) •
[How it works](#how-it-works)

</div>

> **TypeScript port.** `csp` is a TypeScript/Bun port of [MinishLab/semble](https://github.com/MinishLab/semble) — an excellent code search library originally written in Python. All credit for the algorithm and design goes to the Semble authors. This port focuses on shipping the same capabilities to the JavaScript/TypeScript ecosystem.

`csp` is a code search library built for agents. It returns the exact code snippets they need instantly, using ~98% fewer tokens than grep+read. Indexing and searching a full codebase end-to-end takes under a second. Everything runs locally on CPU with no API keys, GPU, or external services. Run it as an [MCP server](#mcp-server) or call it from the shell via [AGENTS.md](#agentsmd) and any agent (Claude Code, Cursor, Codex, OpenCode, etc.) gets instant access to any repo.

## Quickstart

Your agent queries `csp` in natural language (e.g. `"How is authentication handled?"`) and gets back only the relevant code snippets, without grepping or reading full files.

`csp` has three complementary setup paths. The recommended setup is using all three (but you can pick and choose based on your needs):

- **[MCP server](#mcp-server)**: an MCP server for your agent.
- **[AGENTS.md](#agentsmd)**: an AGENTS.md snippet with instructions for calling `csp` via the CLI.
- **[Sub-agent](#sub-agent-setup)**: a dedicated `csp-search` sub-agent for harnesses that support it.

### MCP

Expose `csp` as a native tool via MCP so your agent can call it directly. Add it to Claude Code:

```bash
claude mcp add csp -s user -- bunx @pleaseai/csp mcp
```

See [MCP Server](#mcp-server) below for other harnesses (Cursor, Codex, OpenCode, etc.).

### AGENTS.md

Add `csp` usage instructions to your agent's context so it knows when and how to call the CLI. Install the `csp` CLI, then add the snippet below to your `AGENTS.md` or `CLAUDE.md`:

```bash
bun add -g @pleaseai/csp    # Install with bun (recommended)
npm install -g @pleaseai/csp # Or with npm
pnpm add -g @pleaseai/csp    # Or with pnpm
```

<details>
<summary>AGENTS.md / CLAUDE.md snippet</summary>

````markdown
## Code Search

Use `csp search` to find code by describing what it does or naming a symbol/identifier, instead of grep:

```bash
csp search "authentication flow" ./my-project
csp search "saveCheckpoint" ./my-project
csp search "save model to disk" ./my-project --top-k 10
```

If you anticipate doing more than one search, use `csp index` to create an index.

```bash
csp index ./my-project -o my_index
```

You can then reuse this index later on:

```bash
csp search "saveCheckpoint" --index my_index
```

An index is not automatically updated, so if the code changes significantly, reindex. If you notice stale results while resolving searches to files, reindex.

Use `--content docs` to search documentation and prose, `--content config` for config files (yaml, toml, etc.), or `--content all` to search code, docs, and config:

```bash
csp search "deployment guide" ./my-project --content docs
csp search "database host port" ./my-project --content config
csp search "authentication" ./my-project --content all
```

Use `csp find-related` to discover code similar to a known location (pass `file_path` and `line` from a prior search result):

```bash
csp find-related src/auth.ts 42 ./my-project
```

Like search, `find-related` also accepts an `--index` argument.

`path` defaults to the current directory when omitted; git URLs are accepted.

If `csp` is not on `$PATH`, use `bunx @pleaseai/csp` in its place.

### Workflow

1. Index the repo using `csp index -o cached_index`.
2. Start with `csp search` to find relevant chunks. Pass the index to achieve results faster.
3. Use `--content docs` for documentation, `--content config` for config files, or `--content all` for everything.
4. Inspect full files only when the returned chunk does not give enough context.
5. Optionally use `csp find-related` with a promising result's `file_path` and `line` to discover related implementations.
6. Use grep only when you need exhaustive literal matches or quick confirmation of an exact string.
````

</details>

### Sub-agent

For harnesses that support sub-agents, install a dedicated `csp-search` sub-agent so search runs in its own context (requires the CLI):

```bash
csp init   # Claude Code → .claude/agents/csp-search.md
```

See [Sub-agent setup](#sub-agent-setup) below for other harnesses (Cursor, Codex, OpenCode, etc.).

<details>
<summary>Updating csp</summary>

```bash
bun update -g @pleaseai/csp          # with bun
npm update -g @pleaseai/csp          # with npm
pnpm update -g @pleaseai/csp         # with pnpm
```

</details>

## Main Features

- **Fast**: indexes an average repo in well under a second and answers queries in milliseconds, all on CPU.
- **Accurate**: hybrid retrieval (dense embeddings + BM25) with code-aware reranking.
- **Token-efficient**: returns only the relevant chunks, using ~98% fewer tokens than grep+read.
- **Zero setup**: runs on CPU with no API keys, GPU, or external services required.
- **MCP server**: works with Claude Code, Cursor, Codex, OpenCode, VS Code, and any other MCP-compatible agent.
- **Local and remote**: pass a local path or a git URL.
- **Bun-native**: built and tested on [Bun](https://bun.com); runs on Node.js 22+ as well.

## MCP Server

`csp` can run as an MCP server so agents can search any codebase directly. Repos are cloned and indexed on demand, and indexes are cached for the lifetime of the session. Local paths are watched for file changes and re-indexed automatically.

### Setup

<details>
<summary>Claude Code</summary>

```bash
claude mcp add csp -s user -- bunx @pleaseai/csp mcp
```

</details>

<details>
<summary>Cursor</summary>

Add to `~/.cursor/mcp.json` (or `.cursor/mcp.json` in your project):

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Codex</summary>

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.csp]
command = "bunx"
args = ["@pleaseai/csp", "mcp"]
```

</details>

<details>
<summary>OpenCode</summary>

Add to `~/.opencode/config.json`:

```json
{
  "mcp": {
    "csp": {
      "type": "local",
      "command": ["bunx", "@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>VS Code</summary>

Add to `.vscode/mcp.json` in your project (or your user profile's `mcp.json`):

```json
{
  "servers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>GitHub Copilot CLI</summary>

Add to `~/.copilot/mcp-config.json`:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Windsurf</summary>

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Gemini CLI</summary>

Add to `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Zed</summary>

Add to `~/.config/zed/settings.json` (or `.zed/settings.json` in your project):

```json
{
  "context_servers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

### Tools

| Tool | Description |
|------|-------------|
| `search` | Search a codebase with a natural-language or code query. Pass `repo` as a local directory path or an https:// git URL. |
| `find_related` | Given a file path and line number, return chunks semantically similar to the code at that location. |

By default the MCP server indexes only code files. To also index documentation, config, or everything, append `--content docs`, `--content config`, or `--content all` to the server command, or a combination, e.g. `--content code docs`. For example, in Claude Code: `claude mcp add csp -s user -- bunx @pleaseai/csp mcp --content all`.

## Sub-agent setup

Claude Code, Gemini CLI, Cursor, OpenCode, GitHub Copilot CLI, and Kiro all support a dedicated `csp` search sub-agent. Run `csp init` once in your project root:

```bash
csp init                      # Claude Code  → .claude/agents/csp-search.md
csp init --agent gemini       # Gemini CLI   → .gemini/agents/csp-search.md
csp init --agent cursor       # Cursor       → .cursor/agents/csp-search.md
csp init --agent opencode     # OpenCode     → .opencode/agents/csp-search.md
csp init --agent copilot      # Copilot CLI  → .github/agents/csp-search.md
csp init --agent kiro         # Kiro         → .kiro/agents/csp-search.md
```

If `csp` is not on `$PATH`, prefix the command with `bunx @pleaseai/csp`.

## CLI

`csp` also ships as a standalone CLI. This is useful in scripts or anywhere you want search results without an MCP session.

```bash
# Search a local repo
csp search "authentication flow" ./my-project

# Index first for faster repeated searches (--index works with any command below)
csp index ./my-project -o my-index
csp search "authentication flow" --index my-index

# Search a remote repo (cloned on demand)
csp search "save model to disk" https://github.com/MinishLab/model2vec

# Limit results
csp search "save model to disk" ./my-project --top-k 10

# Search docs/config/everything instead of just code
csp search "deployment guide" ./my-project --content docs   # or: config, all

# Find code similar to a known location
csp find-related src/auth.ts 42 ./my-project
```

`--content` accepts `code` (default), `docs`, `config`, or `all`. `path` defaults to the current directory when omitted; git URLs are accepted. If `csp` is not on `$PATH`, use `bunx @pleaseai/csp` in its place.

<details>
<summary>Savings</summary>

`csp savings` shows how many tokens `csp` has saved across all your searches:

```bash
csp savings           # summary by period
csp savings --verbose # also show breakdown by call type
```

```
  csp Token Savings
  ════════════════════════════════════════════════════════════════
  Period        Calls   Savings
  ────────────────────────────────────────────────────────────────
  Today         42      [███████████████░]  ~58.4k tokens (95%)
  Last 7 days   287     [██████████████░░]  ~312.4k tokens (90%)
  All time      1.4k    [██████████████░░]  ~1.2M tokens (89%)
```

Savings are calculated as follows: for each call, `csp` records the total character count of the unique files containing returned chunks and the character count of the snippets returned. Estimated tokens saved is `(file chars − snippet chars) / 4` (4 chars per token). This is a conservative estimate: the baseline is reading matched files in full, which is how coding agents often explore unfamiliar code.

Stats are stored in `~/.csp/savings.jsonl`.

</details>

<details>
<summary>Library usage</summary>

`csp` can also be used as a TypeScript/JavaScript library for programmatic access, useful when building custom tooling or integrating search directly into your own code.

```ts
import { ContentType, CspIndex } from '@pleaseai/csp'

// Index a local directory (code only, the default)
const index = await CspIndex.fromPath('./my-project')

// Index docs and prose (markdown, rst, etc.)
const docsIndex = await CspIndex.fromPath('./my-project', {
  content: ContentType.DOCS,
})

// Index everything (code, docs, and config)
const allIndex = await CspIndex.fromPath('./my-project', {
  content: [ContentType.CODE, ContentType.DOCS, ContentType.CONFIG],
})

// Index code and docs together
const codeDocsIndex = await CspIndex.fromPath('./my-project', {
  content: [ContentType.CODE, ContentType.DOCS],
})

// Index a remote git repository
const remoteIndex = await CspIndex.fromGit(
  'https://github.com/MinishLab/model2vec',
)

// Search the index with a natural-language or code query
const results = await index.search('save model to disk', { topK: 3 })

// Find code similar to a specific result
const related = await index.findRelated(results[0], { topK: 3 })

// Each result exposes the matched chunk
const result = results[0]
result.chunk.filePath   // "src/model.ts"
result.chunk.startLine  // 127
result.chunk.endLine    // 150
result.chunk.content    // "export function saveCheckpoint(path: string, ..."
```

</details>

## How it works

`csp` splits each file into code-aware chunks using [tree-sitter](https://tree-sitter.github.io/), then scores every query against the chunks with two complementary retrievers: static [Model2Vec](https://github.com/MinishLab/model2vec) embeddings using the code-specialized `potion-code-16M` model for semantic similarity, and BM25 for lexical matches on identifiers and API names. The two score lists are fused with Reciprocal Rank Fusion (RRF).

After fusing, results are reranked with a set of code-aware signals:

<details>
<summary><b>Ranking signals</b></summary>

- **Adaptive weighting.** Symbol-like queries (`Foo::bar`, `_private`, `getUserById`) get more lexical weight, while natural-language queries stay balanced between semantic and lexical retrievers.
- **Definition boosts.** A chunk that defines the queried symbol (a `class`, `function`, `interface`, etc.) is ranked above chunks that merely reference it.
- **Identifier stems.** Query tokens are stemmed and matched against identifier stems in a chunk, giving an additional weight to chunks that contain them. For example, querying `parse config` boosts chunks containing `parseConfig`, `ConfigParser`, or `config_parser`.
- **File coherence.** When multiple chunks from the same file match the query, the file is boosted so the top result reflects broad file-level relevance rather than a single out-of-context chunk.
- **Noise penalties.** Test files, `compat/`/`legacy/` shims, example code, and `.d.ts` declaration stubs are down-ranked so canonical implementations surface first.

</details>

Because the embedding model is static with no transformer forward pass at query time, all of this runs in milliseconds on CPU.

## Development

```bash
bun install        # install dependencies
bun run build      # build with tsdown
bun run typecheck  # type check
bun run lint       # lint with @pleaseai/eslint-config
bun test           # run tests
```

## Credits

`csp` is a TypeScript port of [Semble](https://github.com/MinishLab/semble) by [Thomas van Dongen](https://github.com/Pringled) and [Stéphan Tulkens](https://github.com/stephantul) at [MinishLab](https://github.com/MinishLab). The algorithm, ranking signals, and overall architecture are theirs; this project simply brings them to the JavaScript/TypeScript ecosystem.

If you use the underlying ideas in your research, please cite the original Semble paper:

```bibtex
@software{minishlab2026semble,
  author       = {{van Dongen}, Thomas and Stephan Tulkens},
  title        = {Semble: Fast and Accurate Code Search for Agents},
  year         = {2026},
  publisher    = {Zenodo},
  doi          = {10.5281/zenodo.19785932},
  url          = {https://github.com/MinishLab/semble},
  license      = {MIT}
}
```

## License

[MIT](./LICENSE) © [Minsu Lee](https://github.com/amondnet)

`csp` is a derivative work of [MinishLab/semble](https://github.com/MinishLab/semble), which is also MIT-licensed.
