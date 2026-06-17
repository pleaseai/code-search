# ADR 0002 — Index Storage & Caching Model: Global `~/.csp/index/` Content-Hash Cache

- **Status**: Accepted
- **Date**: 2026-06-18
- **Deciders**: csp maintainers
- **Context**: [Issue #18](https://github.com/pleaseai/code-search/issues/18) — "Wire up CspIndex orchestrator + decide index persistence/caching model"
- **Amends**: the `CLAUDE.md` note that default-ignored dirs "add `.csp/` (replacement for `.semble/`)", which implied a repo-local index cache (see [Divergence](#divergence-from-claudemd))

## Context

`@pleaseai/csp` ports MinishLab/semble. Wiring `CspIndex.fromPath`/`fromGit` and
`save`/`loadFromDisk` (this track) requires deciding **where a built index lives** and
**how it is reused/invalidated**. Two divergent references existed before this decision:

1. **Repo-local `.csp/`** — `CLAUDE.md` lists default-ignored dirs and says to "add `.csp/`
   (replacement for `.semble/`)". Old semble cached its index in a repo-local `.semble/`, so
   this implied csp's intended model was a repo-local `.csp/` auto-cache.
2. **Global cache** — Issue #18 notes that upstream semble "has since moved to auto-indexing
   in a global `~/.cache` folder with content-hash subdirs" (semble PRs #162, #177/#178, #182),
   which is what `clear index` targets upstream.

**Upstream source check (load-bearing for this ADR).** We inspected the cached upstream
checkout at the reviewed baseline (`eacbe43`, 2026-06-12). There is **no `cache.py`** and **no
on-disk content-hash cache** in that baseline. Upstream's only caching is:

- an **in-memory `_IndexCache` LRU** in `mcp.py`, keyed by source path / git URL (no disk persistence), and
- `functools.cache` memoization on a few helpers.

The global `~/.cache` content-hash auto-index referenced by #18 (semble #162 et al.) is **not
present in the ported baseline** — it is unported/aspirational. So there is no upstream disk
cache-key model to match; the design below is **csp-original**, justified on its own merits.

Additional constraints:

- `~/.csp/savings.jsonl` already exists (`src/stats.ts`) — csp already owns a `~/.csp/` home.
- `CspIndex.fromGit` indexes a **remote** repo via a transient checkout — there is no local
  working tree to host a repo-local `.csp/`.
- The CLI must still support **explicit** index paths: `csp index <path> -o <out>` (write) and
  `csp search --index <out>` (read).

## Decision

**Adopt a global `~/.csp/index/<key>/` content-hash auto-cache, with explicit `-o`/`--index`
paths honored verbatim.**

- **Cache home**: `~/.csp/index/` (sibling of the existing `~/.csp/savings.jsonl`).
- **Cache key** (`resolveCacheDir`, `src/indexing/cache.ts`): a hash of **source identity**
  (normalized absolute path for local sources, or git URL `+ ref` for remote) **plus** the
  selected `ContentType[]`. Same source + same content selection → same directory; deterministic.
- **Content-hash invalidation** (`computeContentHash` + `loadOrBuildIndex`): the cached
  `manifest.json` records a `contentHash` computed over the **sorted source file set**
  (path + bytes). On a cache hit, the live source hash is recomputed and compared; a mismatch
  means the source changed → rebuild and overwrite. Git URLs key on URL+ref alone (a remote
  cannot be cheaply re-hashed without re-cloning).
- **Layer split**:
  - `CspIndex.save(dir)` / `loadFromDisk(dir)` — explicit-path persistence roundtrip.
    Writes `manifest.json` + `chunks.json` + `bm25.json` + `vectors.bin` + `args.json`.
  - `cache.loadOrBuildIndex(source, opts)` — disk cache lookup → reuse-or-(build+save).
- **CLI routing**:
  - `csp index <path> -o <out>` → builds, then `save(out)`. `-o` stays **required**
    (explicit persistence only — `csp index` is not auto-cached).
  - `csp search` / `find-related` **with** `--index <path>` → `loadFromDisk(path)` (explicit
    path respected, never bypassed by the auto-cache).
  - `csp search` / `find-related` **without** `--index` → `loadOrBuildIndex(source, …)` (global
    auto-cache).
- **MCP**: the in-memory `IndexCache` (hot LRU + file watcher) routes its build through the
  same `loadOrBuildIndex`, so CLI and MCP share one `~/.csp/index/<key>` and never compute
  divergent views.
- **Invalidation ownership**: the MCP file watcher evicts only the **in-memory** hot entry; the
  on-disk **content-hash** owns disk reuse-vs-rebuild. The watcher never deletes a disk entry,
  so a file change triggers exactly one rebuild (no double rebuild). Disk-entry deletion is the
  job of `csp clear index`.
- **Permissions**: `~/.csp/`, `~/.csp/index/`, and each leaf are created/hardened to `0700`
  (`ensureCacheDir`), since indexed content may mirror private source.

## Alternatives Considered

1. **Repo-local `.csp/` (per the prior `CLAUDE.md` note).** Rejected: it cannot host a
   `fromGit` index (no local working tree for a remote source), it writes into every indexed
   repo (requiring a `.gitignore`/`.cspignore` entry per repo), and it splits the home from the
   already-global `~/.csp/savings.jsonl`. The repo-local model fit old semble's `.semble/` but
   not csp's `fromGit` + global-savings reality.
2. **Upstream-style in-memory cache only (no disk).** Rejected: it gives no reuse across
   separate CLI invocations — every `csp search` would rebuild from scratch. #18 explicitly
   wants a real, persistent index cache.
3. **Global content-hash disk cache (chosen).** Works for both local and remote sources,
   reuses the existing `~/.csp/` home, persists across runs, and invalidates precisely on
   source content change.

## Consequences

### Positive

- One cache model for CLI **and** MCP; no CLI↔MCP divergence.
- Works uniformly for `fromPath` (local) and `fromGit` (remote).
- Persistent across invocations; precise content-hash invalidation.
- `0700` hardening keeps indexed-content artifacts private.

### Negative / Follow-ups

- The global cache grows over time; `csp clear index` (deletes **only** `~/.csp/index/`, never
  the `~/.csp/` root or `savings.jsonl`) manages it.
- Git-URL sources key on URL+ref, not live content — a moving branch is only re-indexed when its
  in-memory entry is evicted or the cache entry is cleared (a remote re-hash would require a
  re-clone). Acceptable for the common pinned-ref / local-path cases.
- Diverges from the prior `CLAUDE.md` repo-local `.csp/` note (see below); docs are updated to
  match.

## Divergence from CLAUDE.md

`CLAUDE.md` previously implied a **repo-local** `.csp/` index cache ("replacement for
`.semble/`"). This ADR moves the **index cache** to the global `~/.csp/index/`. The
`.csp/`-as-ignored-dir guidance remains valid for any repo-local artifacts a user may create
(and `.csp/` stays in the default-ignore list), but the canonical index cache location is
`~/.csp/index/`. `CLAUDE.md` and both READMEs are updated accordingly (track task T015).
