# Tech Debt Tracker

> Tracked across all tracks. Updated during implementation and retrospectives.

## Active

| ID | Source Track | Description | Priority | Created |
|----|------------|-------------|----------|---------|
| TD-001 | rust-rewrite-20260618 | Real Model2Vec embeddings (model2vec-rs) + tree-sitter AST chunking not wired — Rust reproduces the TS stubs only | Medium | 2026-06-18 |
| TD-002 | rust-rewrite-20260618 | `ranking::{apply_query_boost, rerank_top_k}` ported but unwired; search pipeline uses inline stubs (mirrors TS) | Low | 2026-06-18 |
| TD-003 | rust-rewrite-20260618 | MCP server lacks model pre-warm + file watcher (TS `IndexCache` has both); no concurrent in-flight dedup (sync cache) | Low | 2026-06-18 |
| TD-004 | rust-rewrite-20260618 | Distribution cutover (flip live npm/Homebrew release from TS build to Rust binary) pending maintainer runtime-parity decision | Medium | 2026-06-18 |

## Resolved

| ID | Source Track | Description | Resolved In | Date |
|----|------------|-------------|-------------|------|
