# Reference Analysis — CocoIndex Code (`cocoindex-code`)

> Prior-art / comparison analysis of [cocoindex-io/cocoindex-code](https://github.com/cocoindex-io/cocoindex-code)
> and the underlying [CocoIndex](https://cocoindex.io) data-pipeline framework. **This is not a
> port source** — csp ports [MinishLab/semble](semble.md). CocoIndex Code is an independent,
> competing project in the *same* niche (AST-aware semantic code search exposed to coding agents
> over MCP), so it is the closest direct comparator for csp's product surface. This document maps
> their design choices against csp's and flags what is worth borrowing vs. where the two
> deliberately diverge.
>
> **Analyzed at**: web docs + GitHub README as of 2026-06-19 (no upstream commit pinned — this is a
> comparison, not a parity oracle). Sources: <https://cocoindex.io/cocoindex-code/>,
> <https://github.com/cocoindex-io/cocoindex-code>, <https://cocoindex.io/docs-v0/examples/code_index/>.

---

## 1. What CocoIndex Code is

An MCP server + CLI that gives coding agents (Claude Code, Cursor, Codex) **AST-aware semantic
code search** over a whole repo, pitched on token savings (~70%) and sub-second freshness. It is
built on the general-purpose **CocoIndex** data-indexing framework (a Rust engine with a Python
API for declarative ETL/embedding pipelines); `cocoindex-code` is the packaged, code-search-specific
application of that framework.

Two layers, easy to conflate:

- **CocoIndex** (the framework) — Rust core + Python DSL for incremental, lineage-tracked indexing
  pipelines. Built-in `SplitRecursively` (tree-sitter chunker) and `SentenceTransformerEmbed`
  functions. This is the "build your own pipeline" toolkit.
- **CocoIndex Code** (`ccc`) — the opinionated end-user product built on it: a CLI/MCP server with
  an index, daemon, and agent integration baked in.

---

## 2. How it compares to csp (the load-bearing differences)

| Aspect | **CocoIndex Code** | **csp / semble** |
|---|---|---|
| Retrieval signal | **Dense-only** semantic vectors (no BM25) | **Hybrid** dense + BM25, fused via RRF (`k=60`) |
| Embeddings | **Real** models: `Snowflake/snowflake-arctic-embed-xs` local (SentenceTransformers), or 100+ cloud providers via LiteLLM | Model2Vec static embeddings (`potion-code-16M`); the Rust port loads `model2vec-rs::StaticModel` with a deterministic **stub** fallback on load failure, the TS port is still a stub |
| Chunking | tree-sitter AST via `SplitRecursively` (`chunk_size=1000`, `chunk_overlap=300` in the canonical example) | tree-sitter AST, **no overlap**, target 1500 chars, `_MIN_CHUNK_SIZE=50`, line-fallback |
| Indexing model | **Incremental delta** — diff vs. prior AST, re-embed only changed chunks, "80–90% cache hit"; optional **daemon** keeps index warm | Content-hash cache at `~/.csp/index/` (ADR-0002); rebuild on change, no long-running daemon |
| Storage | **SQLite** (`cocoindex.db` + `target_sqlite.db`) under `<project>/.cocoindex_code/` | JSON/serde index files under global `~/.csp/index/` |
| Ranking | Dense cosine kNN; no documented code-specific reranking | RRF → multi-chunk file boost → query-type boost → path penalties + file-saturation decay |
| Symbol / lexical queries | Weak by construction (dense-only — exact identifier match relies on the embedding) | **Adaptive alpha** (`0.3` symbol / `0.5` NL) + identifier-aware BM25 tokenization explicitly handle symbol lookup |
| Embedding benchmark[^bench] | `arctic-embed-xs`: 22M params, 384-dim — **50.15** MTEB *general-text* Retrieval (no public CoIR/code score) | `potion-code-16M`: 16M, 256-dim — **37.05** CoIR (code) avg NDCG@10; teacher `CodeRankEmbed` (137M) = 59.14 |
| Tech stack | Rust engine, Python wrapper (~98% Python repo); `pipx install`, binary `ccc` | TS/Bun (`@pleaseai/csp`, binary `csp`) + Rust port (`crates/csp`, ADR-0003) |
| Deployment tiers | local / shared **daemon** / VPC enterprise (branch overlays, cross-repo, SSO) | single local tool; no daemon/enterprise tier |
| License | Apache-2.0 | MIT |

**One-line takeaway**: CocoIndex Code bets on *real embeddings + incremental delta indexing + a
daemon*; csp/semble bet on *hybrid dense+sparse + code-specific reranking on a zero-dependency CPU
stack*. The dense-only choice makes CocoIndex weaker on exact-symbol queries but lets it lean on
stronger embedding models; csp's BM25 leg + adaptive alpha is precisely the hedge against that.

[^bench]: **Not a head-to-head — the two numbers are from different benchmarks.** `arctic-embed-xs`'s
    50.15 is **MTEB general-text Retrieval** (English prose), *not* code; arctic-embed is not a
    code-trained model and has no public CoIR score, so 50.15 must **not** be read as "code-search
    quality." `potion-code-16M`'s 37.05 is **CoIR** (code-specific, NDCG@10 avg over CosQA /
    CodeFeedback ST/MT). The genuinely load-bearing figure for csp: the model2vec card itself
    reports **potion-code-16M + BM25 hybrid = 40.41** (vs. 37.05 dense-only, **+3.36**) — the model's
    own authors measure that adding sparse retrieval beats static-dense alone, which directly
    validates csp/semble's hybrid + adaptive-alpha design over CocoIndex's dense-only path.
    Sources: [potion-code-16M card](https://huggingface.co/minishlab/potion-code-16M),
    [Snowflake-Labs/arctic-embed](https://github.com/Snowflake-Labs/arctic-embed),
    [CoIR benchmark (ACL 2025)](https://github.com/coir-team/coir).

---

## 3. CLI surface (`ccc`)

For comparison with csp's `csp` subcommands (`search`, `index`, `find-related`, `mcp`, `init`,
`savings`):

| `ccc` | Purpose | csp analog |
|---|---|---|
| `ccc init` | scaffold settings | `csp init` |
| `ccc index` | build/update index | `csp index` |
| `ccc search <query>` | semantic search | `csp search` |
| `ccc status` | index stats | (≈ `csp savings` / stats) |
| `ccc mcp` | MCP server, stdio | `csp mcp` |
| `ccc daemon [status\|restart\|stop]` | background index daemon | — (no equivalent) |
| `ccc reset` | delete index DBs | `csp clear index` |
| `ccc doctor` | diagnostics | — |

- **MCP tool**: a single `search()` tool with `languages` / `paths` / `limit` / `offset` filters.
  csp instead exposes **two** tools (`search`, `find_related`) — csp has no `find_related` analog
  on the CocoIndex side, and CocoIndex has structured filters csp does not.
- **Install/run**: `pipx install 'cocoindex-code[full]'` (local embeddings) or
  `pipx install cocoindex-code` (slim, cloud-only); binary `ccc`. csp: `bunx @pleaseai/csp`.
- **Config**: `~/.cocoindex_code/global_settings.yml` (`embedding.model`, `embedding.provider`,
  `embedding.device` = cpu/cuda/mps, `min_interval_ms`, asymmetric `indexing_params`/`query_params`);
  per-project `include_patterns` / `exclude_patterns`.

---

## 4. Ideas worth tracking for csp

Not endorsements — open questions surfaced by the comparison:

1. **Incremental delta indexing + daemon.** CocoIndex's headline feature is re-embedding only
   changed chunks against a warm index. csp currently caches by content hash and rebuilds; a
   chunk-level delta + optional daemon is the natural next perf step now that the Rust port loads
   real embeddings (under the stub fallback, re-embedding is cheap and the win is smaller — see
   `dense-embedding-is-a-stub`).
2. **Asymmetric query vs. index embedding params** (`indexing_params`/`query_params`). Relevant to
   csp's real Model2Vec path (Rust) — many code-retrieval models want a query prefix.
3. **Structured MCP filters** (`languages`, `paths`, `limit`, `offset`). csp's `search` tool could
   adopt these cheaply; they map onto existing chunk metadata.
4. **Chunk overlap** (`chunk_overlap=300`). semble/csp use **no** overlap; worth measuring whether
   overlap improves recall on boundary-spanning definitions, or just inflates the index.
5. **Branch overlays** (treat a PR/branch as a delta on a shared main index). Enterprise-tier idea,
   but the "index once, overlay per branch" model could inform csp's global `~/.csp/index/` layout.

Where csp should **not** follow: dropping BM25. The dense-only path is CocoIndex's biggest
weakness for symbol/identifier queries, and csp's hybrid + adaptive-alpha design is the explicit
counter-position (see [CLAUDE.md](../../../CLAUDE.md) "Conventions to preserve from semble").

---

## 5. Sources

- CocoIndex Code product page — <https://cocoindex.io/cocoindex-code/>
- `cocoindex-io/cocoindex-code` (CLI/MCP) — <https://github.com/cocoindex-io/cocoindex-code>
- CocoIndex framework code-index example — <https://cocoindex.io/docs-v0/examples/code_index/>
- `cocoindex-io/realtime-codebase-indexing` — <https://github.com/cocoindex-io/realtime-codebase-indexing>
