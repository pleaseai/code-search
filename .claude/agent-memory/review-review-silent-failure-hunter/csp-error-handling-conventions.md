---
name: csp-error-handling-conventions
description: Intentional swallow/fallback patterns in @pleaseai/csp (indexing cache, MCP server) that are by-design and should NOT be flagged as silent failures
metadata:
  type: project
---

Established error-handling patterns in `@pleaseai/csp`. These are intentional and correct — do not re-flag in future silent-failure audits.

**Why:** PR #21 (cache orchestrator + persistence) audit found several swallow/fallback sites that all turned out to be by-design. Recording them avoids repeat false positives.

**How to apply:** Treat the following as sound unless the surrounding contract changes.

- **Cache-validity skip symmetry** (`cache.ts collectSourceFiles` ~226-252 vs `create.ts createIndexFromPath` ~52-67): both `continue` on `statSync`/`readFileSync` failure. The content hash mirrors the actually-indexed corpus, so dropped-file symmetry is NOT a stale-cache hazard.
- **tryReuse corrupt-cache catch** (`cache.ts` ~321-328): swallow `loadFromDisk` error → return null → caller REBUILDS loudly. Not a silent stale serve.
- **clearIndexCache** (`cache.ts` ~151-173): safety invariant (`basename === 'index'` && `!== home`) is checked and THROWS before any rmSync. The `readdirSync` catch only degrades the cosmetic `entries` count; deletion proceeds by design on a guard-validated target.
- **MCP watcher + prewarm swallows** (`server.ts` ~288-291, ~605-623): background watcher/pre-index failures are intentionally non-fatal; per-call `getIndex` (~344-350) wraps and surfaces errors as `Failed to index ...`.
- **Optional-dep import catches** (`server.ts` chokidar/MCP-SDK/zod/stdio ~261, ~482, ~524, ~639): swallow `import()` failure → documented placeholder/no-op. Legit optional-dependency pattern while deps are undeclared (scaffold). NOTE (sub-80 nit): once these become declared deps, distinguishing `ERR_MODULE_NOT_FOUND` from other load errors would prevent masking real in-module throws.
- **CLI top-level catch** (`cli.ts` ~513-517): prints message to stderr + returns exit 1 — surfaces, does not swallow. loadFromDisk errors propagate here.
- **git clone failure** (`index.ts cloneShallow` ~435-440): distinguishes spawn error vs non-zero status, includes git stderr. Surfaces loudly.
