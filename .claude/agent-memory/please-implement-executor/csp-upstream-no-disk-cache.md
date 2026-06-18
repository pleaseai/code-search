---
name: csp-upstream-no-disk-cache
description: upstream semble has NO cache.py / disk content-hash cache — global ~/.csp/index cache is a csp-original (#162, unported); cache-key STOP gates won't trigger
metadata:
  type: project
---

Upstream `MinishLab/semble` (cached checkout `~/.ask/github/github.com/MinishLab/semble/main`,
~May-27 baseline, before `eacbe43`) has **no `cache.py`** and **no disk content-hash cache-dir
key model**.

The only "cache" upstream is:
- `mcp.py` `_IndexCache` — in-memory LRU (`_CACHE_MAX_SIZE=10`), keyed by **source path/URL**,
  session-scoped only.
- Python `functools.cache` memoization (`dense._load_cached`, `chunking._cached_get_parser`).

**Why this matters:** The csp global `~/.csp/index/<key>/` content-hash auto-cache (plan
Architecture Decision, T009/T010) is a **csp-original design** corresponding to upstream **#162**
(global cache auto-indexing), which is **not yet ported** (see project memory
`upstream-semble-sync-baseline`). So any STOP gate phrased as "upstream cache.py key model
fundamentally differs from the plan" **does not trigger** — there is no upstream disk-cache model
to conflict with. The in-memory cache keys on source identity, which is consistent with the plan's
source-identity component.

**How to apply:** For T010/T013, don't go looking for an upstream `cache.py` to port — it doesn't
exist in the synced baseline. T013's ADR should record this divergence (upstream in-memory-only ↔
csp disk content-hash). Verified via `find .../semble/main -iname cache.py` (no result) and
`grep -rE "content_hash|cache_dir|cache_key" src/semble` (only in-memory LRU + functools hits).

Related: [[csp-dense-roundtrip-no-drift]], [[csp-worktree-test-env-gotchas]].
