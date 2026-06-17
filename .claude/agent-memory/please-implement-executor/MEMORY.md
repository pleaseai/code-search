# Memory Index

- [csp typecheck baseline is red by design](csp-typecheck-baseline-red.md) — TS5097 .ts-import errors are project-wide; gate on "no new type errors", not "green tsc"
- [csp test files are ahead of impl](csp-test-files-ahead-of-impl.md) — some indexing *.test.ts depend on not-yet-existing APIs; a scoped source fix won't make them green
- [csp worktree test env gotchas](csp-worktree-test-env-gotchas.md) — full `bun test` is flooded by tmpfs ENOSPC and ESLint can't run (no jiti); trust isolated runs
- [csp bun mock.module is irreversible](csp-bun-mock-module-irreversible.md) — top-level mock.module leaks process-wide across files; use DI/static reassignment instead
- [csp dense roundtrip has no float drift](csp-dense-roundtrip-no-drift.md) — SelectableBasicBackend save→load is bit-stable; re-normalizing unit vectors is idempotent; NFR-002 safe, T007 can reuse .load
- [csp loadFromDisk model dim alignment](csp-loadfromdisk-model-dim-alignment.md) — reloaded stub model is fixed 256-dim; align to persisted backend dim or query() throws dim mismatch
- [csp upstream has no disk cache](csp-upstream-no-disk-cache.md) — semble has no cache.py; global ~/.csp/index cache is csp-original (#162, unported); cache-key STOP gates won't fire
- [csp loadOrBuildIndex cache contract](csp-loadorbuild-cache-contract.md) — local reuse gated by source-file hash via save(dir,{contentHash}); git keyed by URL+ref only
- [csp cli cache DI seam](csp-cli-cache-di-seam.md) — cli auto-cache via injectable loadOrBuild seam, build-branch only; mcp (T012) must mirror same key contract
