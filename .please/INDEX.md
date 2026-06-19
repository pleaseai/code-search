# .please/ Workspace Index

> Central navigation for all project artifacts managed by the please plugin.

## Project Documents

| Document | Purpose |
|---|---|
| [`../ARCHITECTURE.md`](../ARCHITECTURE.md) | Repository-level bird's-eye view *(not yet created)* |
| [`../CLAUDE.md`](../CLAUDE.md) | Project-level AI instructions |
| [`../README.md`](../README.md) | Public-facing spec (English) |
| [`../README.ko.md`](../README.ko.md) | Public-facing spec (한국어) |

## Directory Map

| Path | Purpose |
|---|---|
| `state/` | Runtime session state (progress) — not tracked in git |
| `docs/tracks/` | Implementation tracks (spec + plan) → [Tracks](docs/tracks.jsonl) |
| `docs/product-specs/` | Product-level specifications → [Product Specs Index](docs/product-specs/index.md) |
| `docs/decisions/` | Architecture Decision Records → [Decisions Index](docs/decisions/index.md) |
| `docs/investigations/` | Bug investigation reports |
| `docs/research/` | Research documents |
| `docs/references/` | External reference materials & upstream analyses → [References Index](docs/references/index.md) |
| `docs/knowledge/` | Stable project context (product, tech-stack, guidelines, workflow) |
| `templates/` | Workflow templates (plugin-provided) |
| `scripts/` | Utility scripts (plugin-provided) |

## Configuration

See [config.yml](config.yml) for workspace settings.

## Workflows

- `/please:new-track` — Create feature specification and architecture plan
- `/please:implement` — TDD implementation from plan file
- `/please:finalize` — Finalize PR, move track to completed
