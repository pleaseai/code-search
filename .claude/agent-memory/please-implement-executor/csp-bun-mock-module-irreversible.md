---
name: csp-bun-mock-module-irreversible
description: Bun mock.module is process-global and irreversible; use DI/static-method reassignment for module stubs to avoid cross-file leaks
metadata:
  type: feedback
---

In this repo's Bun (1.3.10) test runner, `mock.module(path, factory)` mutates the
process-wide module registry **irreversibly**. There is no working restore:
- `afterAll(() => mock.module(path, () => realModule))` does NOT restore — the next
  test file still sees the stub.
- `mock.restore()` does NOT restore module mocks either.
- Bun evaluates every test file's top-level code before running any tests, so a
  top-level `mock.module` in one file poisons sibling files regardless of file order.

**Why it matters:** a top-level `mock.module('../indexing/index.ts')` in
`src/mcp/server.test.ts` leaked a stub CspIndex into `src/indexing/index.test.ts`,
making it fail only in the full suite (passed in isolation). Diagnosing "passes
isolated, fails in full suite" should immediately suspect a leaked `mock.module`.

**How to apply:** when a test needs to stub a module export, prefer a DI seam over
`mock.module`. For a class's static methods (e.g. `CspIndex.fromPath/fromGit`),
reassign them on the imported class object (same reference the SUT imports) and
restore in `afterAll` — that IS reversible (plain property mutation). Return real
instances from the stub when tests assert `instanceof`. See `src/mcp/server.test.ts`
for the pattern. Verify with `bun test <fileA> <fileB>` in BOTH orders.

Related: [[csp-worktree-test-env-gotchas]] (full-suite vs isolated trust).
