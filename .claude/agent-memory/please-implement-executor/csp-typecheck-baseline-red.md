---
name: csp-typecheck-baseline-red
description: csp project's `bun run typecheck` is red at baseline — TS5097 fires on every relative .ts import; gate on "no new type errors", not "green tsc"
metadata:
  type: project
---

`bun run typecheck` (`tsc --noEmit`) in `@pleaseai/csp` is **red at baseline** and expected to be.

**Why:** the repo mandates `.ts` import extensions (CLAUDE.md: `verbatimModuleSyntax` + `.ts` imports, `moduleResolution: bundler`) but tsconfig does not set `allowImportingTsExtensions`. So `TS5097` ("An import path can only end with a '.ts' extension when 'allowImportingTsExtensions' is enabled") fires on **every** relative import — ~47 occurrences project-wide. There are also pre-existing errors like local-but-not-exported `Chunk`/`SearchResult` in some test files.

**How to apply:** when an implement task touches typecheck, the gate is **"do not add new *type* errors to the touched files, reduce if possible"** — NOT "whole-suite tsc green". Adding any new `.ts` import unavoidably adds one more `TS5097`; that is the established convention (e.g. `utils.ts` does it too), not a regression. Distinguish config-class errors (TS5097) from genuine type errors (TS2xxx mismatches). To check a file's delta, `git stash` and compare `tsc` output filtered to that file before/after.

Baseline test suite (as of 2026-06-18): **316 pass / 5 fail / 3 errors** across 20 files. The 5 fails include "public barrel > exposes ContentType as a runtime enum" and "csp search (stub-mocked) > formats non-empty results as JSON". Verify your change leaves the failing-test *set* unchanged (diff the `^(fail)` lines before/after) rather than chasing whole-suite green.

`bun run lint` is currently broken project-wide: eslint can't load the TS flat config (`jiti` library missing). Pre-existing tooling gap, unrelated to code changes.
