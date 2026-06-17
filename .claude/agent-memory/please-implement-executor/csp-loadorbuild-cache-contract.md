---
name: csp-loadorbuild-cache-contract
description: loadOrBuildIndex cache-validity contract — source-file hash (not chunks hash) gates local reuse; git keyed by URL+ref only
metadata:
  type: project
---

`loadOrBuildIndex(source, {content?, ref?, modelPath?, baseDir?})` in `src/indexing/cache.ts` is the auto-cache orchestrator wired into CLI search/find-related (T011) and MCP (T012).

**Cache-validity contract (load-bearing):**
- Local paths: validity = `computeContentHash(collectSourceFiles(source, content))` (a **source-file** hash, computable *before* a build) must equal the cached `manifest.contentHash`. `collectSourceFiles` mirrors `createIndexFromPath`'s scan: `getExtensions(content)` + `walkFiles` (ignore rules) + `MAX_FILE_BYTES` cutoff, paths relative to root.
- `CspIndex.save(dir, { contentHash? })` was extended so loadOrBuildIndex injects that source-file hash. **Without injection it defaults to `sha256(chunks JSON)`** (T006 behavior) — which is computable only *after* build, so it can't gate a pre-build cache check. The two hash definitions MUST agree or the cache misses forever.
- Git URLs (T009 STOP fallback): no live re-hash possible + temp-checkout metadata is non-deterministic → keyed by URL+ref alone via `resolveCacheDir`'s `ref` option. Manifest existence = reuse; build-time hash recorded for transparency only, not validation.

**Why:** plan flagged the contentHash-definition mismatch as the central risk; resolving it (optional `save` arg, backward compatible) is what makes the cache actually invalidate on source change.

**How to apply:** when wiring T011/T012, pass `baseDir` only in tests (real callers omit → `~/.csp`). Tests spy on rebuilds via static reassignment of `CspIndex.fromPath` (see [[csp-bun-mock-module-irreversible]]), not `mock.module`. exactOptionalPropertyTypes forbids passing explicit `undefined` — build option objects conditionally.
