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
  Do NOT redo the Chunk unification — it is done. The `makeStubModel`/`documents`/
  `ContentType.Docs` test blockers above are still open and belong to T003.

**Why:** the plan front-loaded test files for the eventual API; impl lands incrementally
across tasks/phases, so a test can be red at baseline through no fault of the current task.

**How to apply:** when a task's plan scenario says "make X.test.ts green" but the test
won't even load, check whether the blockers are in your `Files:` scope. If they require
editing other modules or the test file itself, do NOT expand scope or weaken the test —
fix your scoped target, verify "no new errors / suite unchanged at baseline", and record
the cross-scope blocker in the plan's `## Surprises & Discoveries` for the owning task
(usually T003, which depends on T002). See [[csp-typecheck-baseline-red]] for the
typecheck gate ("no new errors", not "green tsc").
