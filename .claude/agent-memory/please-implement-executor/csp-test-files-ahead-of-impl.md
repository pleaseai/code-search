---
name: csp-test-files-ahead-of-impl
description: csp indexing test files were authored against APIs that don't exist yet — fixing a single in-scope source file won't make them green
metadata:
  type: project
---

In the csp (`@pleaseai/csp`) indexing track, some `*.test.ts` files were written
against a target API surface that the implementation does not yet expose. Fixing one
in-scope source file (e.g. T002 = the 3 compile errors in `src/indexing/create.ts`)
does **not** make the matching test green, because the test depends on cross-module
API gaps that are out of that task's `Files:` scope.

Concrete example (`src/indexing/create.test.ts`, observed 2026-06-18, baseline 3 errors):
- calls `makeStubModel('name', dim)` / `makeStubModel()` but `makeStubModel` is NOT
  exported from `dense.ts` and its real signature is `makeStubModel(dim: number)`.
- accesses `bm25Index.documents` but `Bm25Index` exposes no `documents` property
  (state is a private `#state` field; only `static build`/`getScores`/`save`/`load`).
- uses `ContentType.Docs` but the enum is `CODE | DOCS | CONFIG` (uppercase).
- `create.ts` previously had 4 async/Chunk-type errors (lines 49/67/74) — these were
  RESOLVED in T002 round 2 (2026-06-18, commit a328727): `for await` over `walkFiles`
  (AsyncIterable), `await chunkSource(...)` (Promise), `language ?? null`, and unifying
  `dense.ts`/`sparse.ts` local `Chunk` into `../types.ts` (with `export type { Chunk }`
  re-export so `dense.test.ts`/`sparse.test.ts` keep importing `Chunk` from those modules).
  Do NOT redo the Chunk unification — it is done.

T003 (2026-06-18, commit 7aa721a) wired `src/indexing/index.ts` (fromPath + ctor options
object + stats + DEFAULT_CONTENT export + save/loadFromDisk throwing stubs + loadModel tuple).
That fixed the `DEFAULT_CONTENT` import blocker in `index.test.ts`, but the test file STILL
won't load — the blockers that remain are ALL outside `src/indexing/index.ts` (T003's only
Files-scope), so they were NOT fixed and must be handled by whoever owns dense.ts/sparse.ts:
- `makeStubModel` not exported from `dense.ts`; test calls `makeStubModel('name', dim)` but
  real sig is private `makeStubModel(dim: number)` → needs dense.ts export + 2-arg signature.
- test uses `new SelectableBasicBackend(vectors, dim)` but current ctor is `(vectors, BasicArgs)`
  → needs dense.ts ctor change.
- test uses `new Bm25Index([['x']])` (public ctor) but ctor is private (`static build` only)
  → needs sparse.ts change.
- test line 197 uses `ContentType.Code` (should be `CODE`) → needs the test file itself fixed.
The `findRelated`/`search` filter-behavior asserts in `index.test.ts` are T004 (behavioral),
not T003. So `index.test.ts` goes green only after dense.ts + sparse.ts + the test file are
fixed AND T004 ranking is wired — not within any single one of those scopes.

**Why:** the plan front-loaded test files for the eventual API; impl lands incrementally
across tasks/phases, so a test can be red at baseline through no fault of the current task.

**How to apply:** when a task's plan scenario says "make X.test.ts green" but the test
won't even load, check whether the blockers are in your `Files:` scope. If they require
editing other modules or the test file itself, do NOT expand scope or weaken the test —
fix your scoped target, verify "no new errors / suite unchanged at baseline", and record
the cross-scope blocker in the plan's `## Surprises & Discoveries` for the owning task
(usually T003, which depends on T002). See [[csp-typecheck-baseline-red]] for the
typecheck gate ("no new errors", not "green tsc").

**RESOLVED in T004 (2026-06-18, commit ba30228):** `index.test.ts` went green for its
search/findRelated/stats cases. The planner correctly put the cross-file fixes in T004's
`Files:` (index.ts, index.test.ts, dense.ts). Key correction to the T003 prediction above:
- Only `makeStubModel` export (dense.ts) was actually needed. The scaffold test's GUESSES
  `new Bm25Index([['x']])` and `new SelectableBasicBackend(vecs, 4)` were simply WRONG —
  the real APIs `Bm25Index.build(docs)` and `new SelectableBasicBackend(vecs)` (dim derived
  from vectors) already existed. So **sparse.ts needed NO change**; the fix was correcting
  the TEST setup to the real API, not changing the impl. That is why T004's Files had
  index.test.ts + dense.ts but not sparse.ts.
- Wiring pattern that avoided the STOP("search.ts API structurally incompatible"): put the
  blank-query / `topK<=0` / empty-index / empty-selector guards in the CspIndex.search
  LAYER, then delegate to `search.ts search(query, model, semanticIndex, bm25Index, chunks,
  topK, {selector?})`. search.ts already returns `[]` for an empty selector (effectiveK→0),
  so passing the empty `Uint32Array` through (no unfiltered fallback) satisfies the
  "filters match nothing → []" regression test. `findRelated` re-embeds the seed content,
  calls `semanticIndex.query(emb, topK+1)`, drops the seed chunk. Both kept SYNC (mcp/
  server.ts:370 and cli.ts call without await).
