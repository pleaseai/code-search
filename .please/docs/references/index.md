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

<!-- Add new reference analyses above this line as additional libraries are adopted. -->

## Sync baselines

Each analysis records the exact upstream commit it was written against. To check for upstream
drift, diff from that commit forward (`git log <baseline>..main` in the upstream checkout).

| Library | Analyzed at | Notes |
|---|---|---|
| semble | upstream `136b6f7` (2026-06-18); Rust port `2f2baa2` (PR #34) | Mapped to the Rust crates; beyond prior review baseline `eacbe43`; see semble.md §Divergences |

## How to add a new reference analysis

1. Create `.please/docs/references/<library>.md` following the structure of `semble.md`:
   overview → pipeline → module map → per-module deep dives → key algorithms → divergences.
2. Pin the exact upstream commit analyzed (record it in the **Sync baselines** table above).
3. Map every upstream module to its csp counterpart (or mark it *not ported* / *adapted*).
4. Add a row to the **Documents** table and link the new file.
