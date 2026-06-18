---
name: csp-worktree-test-env-gotchas
description: In this csp worktree, "passes isolated/fails full-suite" was a leaked mock.module (now fixed), and ESLint can't run (missing jiti) — trust isolated runs, skip the lint gate
metadata:
  type: project
---

Two environment gotchas observed in the csp orchestrator worktree (2026-06-18, T004):

1. **Full-suite `bun test` showed extra fails — root cause was a leaked
   `mock.module`, NOT (primarily) tmpfs ENOSPC.** Earlier this was attributed to a
   sandbox disk-full flood, but on 2026-06-18 (T0A) the disk had 148Gi free and the
   real cause was identified: `src/mcp/server.test.ts`'s top-level
   `mock.module('../indexing/index.ts')` leaked a stub CspIndex into
   `src/indexing/index.test.ts`, which then failed in the full suite while passing
   in isolation. That leak is now FIXED (DI seam — see
   [[csp-bun-mock-module-irreversible]]). Full suite is now stable at
   **351 pass / 3 fail / 0 error** (the 3 fails are T006/T007 throwing stubs).

   **How to apply:** "passes isolated, fails in full suite" → first suspect a leaked
   `mock.module`, not disk. Still trust isolated runs (`bun test <file>`) for a
   per-file verdict, and use `git stash` + isolated run to prove a fail is
   pre-existing vs a regression. (If tmpfs ENOSPC ever does recur, `export
   TMPDIR=$PWD/.tmptest` to a roomy in-repo dir; never commit `.tmptest/`.)

   **Update (2026-06-18, T009): tmpfs ENOSPC DID recur** — `/private/tmp/claude-501/.../tasks`
   reported 0MB free and killed `bun`/`ask`/`find` with ENOSPC (real Data volume had 148Gi).
   Fix that worked: prefix every command with
   `CLAUDE_CODE_TMPDIR=/Users/lms/.cache/csp-tmp` (mkdir -p once). With that, the **full
   `bun test` ran completely clean (384 pass / 0 fail / 0 error)** — no flooding. So redirect
   the harness tmpdir rather than distrusting full-suite counts.

3. **`ask` CLI is not on the non-interactive shell PATH** (and MCP `ask_question`/
   `ask_public_library` tools are not directly invokable from this agent). To read upstream
   semble source, the `ask` cache on disk works: checkouts live under
   `~/.ask/github/github.com/<org>/<repo>/<ref>/` (e.g.
   `~/.ask/github/github.com/MinishLab/semble/main`). `find`/`grep`/`Read` that path directly.

2. **ESLint cannot run in this worktree — missing `jiti`.** `bunx eslint ...` fails
   with "The 'jiti' library is required for loading TypeScript configuration files."
   The flat config is `eslint.config.ts` (TS), which needs jiti. This is a
   pre-existing infra gap affecting every file equally, not something a task broke.

   **How to apply:** The lint gate is not runnable here — note it as skipped in the
   report and rely on matching project style manually (no semicolons, single quotes,
   2-space indent, per CLAUDE.md). Do not treat lint failure as a code defect.

See [[csp-typecheck-baseline-red]] for the typecheck gate and
[[csp-test-files-ahead-of-impl]] for why some tests are red at baseline.
