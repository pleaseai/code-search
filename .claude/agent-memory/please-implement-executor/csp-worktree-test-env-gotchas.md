---
name: csp-worktree-test-env-gotchas
description: In this csp worktree, full `bun test` is contaminated by a sandbox tmpfs ENOSPC flood and ESLint can't run (missing jiti) — trust isolated runs, skip the lint gate
metadata:
  type: project
---

Two environment gotchas observed in the csp orchestrator worktree (2026-06-18, T004):

1. **Full-suite `bun test` is unreliable due to tmpfs ENOSPC.** Running the whole
   suite floods the sandbox temp filesystem; commands intermittently die with
   "the temp filesystem ... is full (0MB free) ... ENOSPC". When this happens,
   tests that pass in isolation (e.g. `CspIndex.stats`, `public barrel`,
   `createIndexFromPath`, `csp search formats`) show up as extra `fail`s purely
   from disk-full, inflating the fail count (observed 12 fail/1 error full-suite
   vs the true 6 fail when running `bun test src/indexing/` in isolation).

   **Why:** the harness's task-recording mount and bun's temp output share a small
   tmpfs that fills mid-run.

   **How to apply:** Trust ISOLATED runs (`bun test <dir>` or `<file>`) for the
   pass/fail verdict, not the full-suite count. Set `TMPDIR` to a roomy in-repo dir
   (`mkdir .tmptest; export TMPDIR=$PWD/.tmptest`) and clear it between runs; never
   commit `.tmptest/`. To prove a failure is pre-existing vs your regression, use
   `git stash` + isolated run on the affected file. The track baseline is
   "320 pass / 5 fail / 3 errors" — compare against that, not a flooded full run.

2. **ESLint cannot run in this worktree — missing `jiti`.** `bunx eslint ...` fails
   with "The 'jiti' library is required for loading TypeScript configuration files."
   The flat config is `eslint.config.ts` (TS), which needs jiti. This is a
   pre-existing infra gap affecting every file equally, not something a task broke.

   **How to apply:** The lint gate is not runnable here — note it as skipped in the
   report and rely on matching project style manually (no semicolons, single quotes,
   2-space indent, per CLAUDE.md). Do not treat lint failure as a code defect.

See [[csp-typecheck-baseline-red]] for the typecheck gate and
[[csp-test-files-ahead-of-impl]] for why some tests are red at baseline.
