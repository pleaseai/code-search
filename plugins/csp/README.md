# csp — Claude Code plugin

Fast, local, token-efficient hybrid code search for agents, packaged as a [Claude Code plugin](https://docs.claude.com/en/docs/claude-code/plugins). See the [project README](../../README.md) for the full library/CLI/MCP documentation.

## What this plugin installs

- **MCP server `csp`** — exposes the `search` and `find_related` tools, launched via `bunx @pleaseai/csp mcp`. Repos are cloned and indexed on demand; local paths are watched and re-indexed automatically.
- **Sub-agent `csp-search`** — a dedicated agent that runs `csp` searches in its own context (requires the `csp` CLI; falls back to `bunx @pleaseai/csp`).

## Install

```text
/plugin marketplace add pleaseai/code-search
/plugin install csp@pleaseai
```

Then restart Claude Code (or run `/mcp` to confirm the `csp` server is connected).

## Requirements

[Bun](https://bun.sh) 1.3.10+ must be on your `PATH` so `bunx` can launch the MCP server. Node.js 22+ also works via `npx @pleaseai/csp mcp` (edit `.mcp.json` if you prefer npm).

## Customization

By default the MCP server indexes only code files. To also index documentation or config, append `--content` to the server args in `.mcp.json`, e.g. `["@pleaseai/csp", "mcp", "--content", "all"]`.
