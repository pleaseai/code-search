# csp-mcp — Claude Code plugin

Registers the `csp` MCP server for Claude Code. It is the MCP half of the [`csp`](../csp/README.md) plugin, split out so you can choose whether to run the server. See the [project README](../../README.md) for the full library/CLI/MCP documentation.

## What this plugin installs

- **MCP server `csp`** — exposes the `search` and `find_related` tools, launched via `bunx @pleaseai/csp mcp`. Repos are cloned and indexed on demand; local paths are watched and re-indexed automatically.

The `csp-search` sub-agent and skill live in the [`csp`](../csp/README.md) plugin. Install both for the full setup.

## Install

```text
/plugin marketplace add pleaseai/claude-code-plugins
/plugin install csp-mcp@pleaseai
```

Then restart Claude Code (or run `/mcp` to confirm the `csp` server is connected).

## Requirements

[Bun](https://bun.sh) 1.3.10+ must be on your `PATH` so `bunx` can launch the MCP server. Node.js 22+ also works via `npx @pleaseai/csp mcp` (edit the MCP config if you prefer npm).

## Customization

By default the MCP server indexes only code files. To also index documentation or config, append `--content` to the server args in `.mcp.json`, e.g. `["@pleaseai/csp", "mcp", "--content", "all"]`. A single tool call can also override this with its `content` argument (`code`, `docs`, `config`, or `all`).
