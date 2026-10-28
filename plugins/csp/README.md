# csp — Claude Code & Codex plugin

Fast, local, token-efficient hybrid code search for agents, packaged as both a [Claude Code plugin](https://docs.claude.com/en/docs/claude-code/plugins) and a [Codex plugin](https://developers.openai.com/codex/plugins/build) from a single directory. See the [project README](../../README.md) for the full library/CLI/MCP documentation.

## What this plugin installs

- **`csp-search` helper** — a dedicated sub-agent (`agents/csp-search.md`, Claude Code) and skill (`skills/csp-search/SKILL.md`, Claude Code and Codex) that run `csp search` / `csp find-related` through the CLI with the right flags.
- **MCP server `csp`** (Codex only) — exposes the `search` and `find_related` tools, launched via `bunx @pleaseai/csp mcp`.

On Claude Code the MCP server ships as a separate plugin, [`csp-mcp`](../csp-mcp/README.md), so you can choose whether to run it. Install both for the full setup.

## Install

**Claude Code:**

```text
/plugin marketplace add pleaseai/claude-code-plugins
/plugin install csp@pleaseai
/plugin install csp-mcp@pleaseai   # optional: the MCP server
```

**Codex:**

```bash
codex plugin marketplace add pleaseai/claude-code-plugins
codex plugin add csp@pleaseai
```

## Requirements

The helper calls the `csp` CLI. Install it with Homebrew (`brew install pleaseai/tap/csp`), or keep [Bun](https://bun.sh) 1.3.10+ on your `PATH` so the helper can fall back to `bunx @pleaseai/csp`.

## Layout

This one directory carries both manifests; each harness reads its own and ignores the other:

| | Claude Code | Codex |
|---|---|---|
| Manifest | `.claude-plugin/plugin.json` | `.codex-plugin/plugin.json` |
| MCP config | none (see [`csp-mcp`](../csp-mcp/README.md)) | `.mcp.codex.json` (`mcp_servers`) |
| Helper | `agents/csp-search.md` (sub-agent) and `skills/csp-search/SKILL.md` (skill) | `skills/csp-search/SKILL.md` (skill) |

Marketplaces are defined at the repo root: `.claude-plugin/marketplace.json` (Claude Code) and `.agents/plugins/marketplace.json` (Codex).
