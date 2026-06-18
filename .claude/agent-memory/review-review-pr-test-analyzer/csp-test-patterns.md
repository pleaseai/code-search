---
name: csp-test-patterns
description: Testing conventions and recurring coverage-gap patterns in the @pleaseai/csp (bun:test) repo
metadata:
  type: project
---

Testing conventions in @pleaseai/csp (bun:test). Knowing these speeds up future test-coverage reviews of this repo.

**Why:** Recurring patterns across cache.test.ts / index.test.ts / cli.test.ts / mcp/server.test.ts that affect how to judge assertion strength.

**How to apply:**
- DI seam injection is the dominant test style: CLI commands take a `loadOrBuild`/`fromPath`/`readIndex`/`clearIndex`/`clearSavings` seam object; MCP `IndexCache` takes a `loadOrBuild` seam. Routing/forwarding is asserted by capturing seam args, not by side effects.
- `mock.module` is BANNED here — it mutates the process-wide registry irreversibly and leaks stubs into sibling test files. The convention is static-method reassignment with `afterAll` restore (see mcp/server.test.ts top comment, and cache.test.ts cache-hit/invalidation tests that spy on `CspIndex.fromPath`). The spy is valid because `loadOrBuildIndex`/`buildIndex` call `CspIndex.fromPath` via the static reference (cache.ts:355).
- `baseDir` override is the standard way to keep `~/.csp` cache tests off the real user home. `ensureCacheDir`/`clearIndexCache`/`resolveCacheDir` all accept `{ baseDir }`.
- Real-roundtrip tests (no seams) exist alongside seam tests: index.test.ts roundtrip + cli.test.ts "index -o → search --index" build a real CspIndex on a tiny temp dir.
- AC-015 (clear-index safety: home + savings.jsonl survive) is covered twice — unit (cache.test.ts:268) and CLI-level real-temp-home (cli.test.ts:402).
- fromGit temp-dir cleanup is asserted by counting `csp-git-*` dirs in tmpdir before/after, on BOTH success and clone-failure paths (index.test.ts:365,377).

Known thin spots (low criticality, not blockers): the `clearIndexCache` "Refusing to clear unsafe index path" throw guard (cache.ts:157) is only indirectly asserted (the test checks the invariant holds, never forces the throw); `tryReuse` corrupt-cache catch (cache.ts:325) has no direct test; dense bit-stability (NFR-002) is `toBeCloseTo(...,6)` not exact bit-equality.
