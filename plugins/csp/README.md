# csp — Claude Code & Codex plugin

Fast, local, token-efficient hybrid code search for agents, packaged as both a [Claude Code plugin](https://docs.claude.com/en/docs/claude-code/plugins) and a [Codex plugin](https://developers.openai.com/codex/plugins/build) from a single directory. See the [project README](../../README.md) for the full library/CLI/MCP documentation.

## What this plugin installs

- **MCP server `csp`** — exposes the `search` and `find_related` tools, launched via `bunx @pleaseai/csp mcp`. Repos are cloned and indexed on demand; local paths are watched and re-indexed automatically.
- **`csp-search` helper** — a dedicated Claude Code sub-agent (`agents/csp-search.md`) / Codex skill (`skills/csp-search/SKILL.md`) that runs `csp` searches with the right flags.

## Install

**Claude Code:**

```text
/plugin marketplace add pleaseai/claude-code-plugins
/plugin install csp@pleaseai
```

Then restart Claude Code (or run `/mcp` to confirm the `csp` server is connected).

**Codex:**

```bash
codex plugin marketplace add pleaseai/claude-code-plugins
codex plugin add csp@pleaseai
```

## Requirements

[Bun](https://bun.sh) 1.3.10+ must be on your `PATH` so `bunx` can launch the MCP server. Node.js 22+ also works via `npx @pleaseai/csp mcp` (edit the MCP config if you prefer npm).

## Customization

By default the MCP server indexes only code files. To also index documentation or config, append `--content` to the server args, e.g. `["@pleaseai/csp", "mcp", "--content", "all"]`.

## Layout

This one directory carries both manifests; each harness reads its own and ignores the other:

| | Claude Code | Codex |
|---|---|---|
| Manifest | `.claude-plugin/plugin.json` | `.codex-plugin/plugin.json` |
| MCP config | `.mcp.json` (`mcpServers`) | `.mcp.codex.json` (`mcp_servers`) |
| Helper | `agents/csp-search.md` (sub-agent) | `skills/csp-search/SKILL.md` (skill) |

Marketplaces are defined at the repo root: `.claude-plugin/marketplace.json` (Claude Code) and `.agents/plugins/marketplace.json` (Codex).
