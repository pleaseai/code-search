# Reference Analyses

> Deep-dive analyses of the external libraries `@pleaseai/csp` ports from or draws on.
> Each document maps an upstream codebase module-by-module to its csp counterpart,
> records the load-bearing algorithms and parameters, and tracks where the csp port
> intentionally (or accidentally) diverges from upstream.

These are **reference materials**, not contracts. The contract is `README.md` / `ARCHITECTURE.md`.
When upstream moves, update the relevant analysis here and reconcile any new drift against
[the sync baseline](#sync-baselines).

## Documents

| Library | Upstream | Role | Analysis |
|---|---|---|---|
| **semble** | [MinishLab/semble](https://github.com/MinishLab/semble) | Direct port source — analyzed against the **Rust port** (`crates/csp`, [ADR-0003](../decisions/0003-rewrite-in-rust.md)) | [semble.md](semble.md) |
| **cocoindex-code** | [cocoindex-io/cocoindex-code](https://github.com/cocoindex-io/cocoindex-code) | Prior-art / comparator — independent AST code-search MCP in the same niche (not a port source) | [cocoindex.md](cocoindex.md) |
| **model2vec** | [MinishLab/model2vec](https://github.com/MinishLab/model2vec) + [model2vec-rs](https://github.com/MinishLab/model2vec-rs) | Direct dependency — the dense-retrieval leg (`potion-code-16M`); Rust port wires `model2vec-rs` | [model2vec.md](model2vec.md) |

<!-- Add new reference analyses above this line as additional libraries are adopted. -->

## Sync baselines

Each analysis records the exact upstream commit it was written against. To check for upstream
drift, diff from that commit forward (`git log <baseline>..main` in the upstream checkout).

| Library | Analyzed at | Notes |
|---|---|---|
| semble | upstream `136b6f7` (2026-06-18); Rust port `2f2baa2` (PR #34) | Mapped to the Rust crates; beyond prior review baseline `eacbe43`; see semble.md §Divergences |
| cocoindex-code | web docs + GitHub README, 2026-06-19 (no commit pinned); embedding benchmarks from model HF cards | Comparison/prior-art, **not a port** — no parity oracle. Drift = re-check vs. cocoindex's docs/README; benchmark row reflects published `potion-code-16M` (CoIR) vs. `arctic-embed-xs` (MTEB) figures. See cocoindex.md §2 + `[^bench]` |
| model2vec | GitHub READMEs + HF cards, 2026-06-19; `model2vec-rs` `0.2.1` (no commit pinned) | **Direct dependency**, not a port — the dense leg. Drift = pin `model2vec-rs` crate version + `potion-code-16M` card revision when the stub is swapped for real weights. See model2vec.md §4–5 |

## How to add a new reference analysis

1. Create `.please/docs/references/<library>.md` following the structure of `semble.md`:
   overview → pipeline → module map → per-module deep dives → key algorithms → divergences.
2. Pin the exact upstream commit analyzed (record it in the **Sync baselines** table above).
3. Map every upstream module to its csp counterpart (or mark it *not ported* / *adapted*).
4. Add a row to the **Documents** table and link the new file.
