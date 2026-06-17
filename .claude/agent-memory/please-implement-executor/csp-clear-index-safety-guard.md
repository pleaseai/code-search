---
name: csp-clear-index-safety-guard
description: clear index deletes ONLY ~/.csp/index via clearIndexCache; AC-015 guard asserts target ends with `index` and != home before rmSync
metadata:
  type: project
---

`csp clear index` / `clear all` wiring (T014, src/cli.ts + src/indexing/cache.ts).

Fact: deletion of the global on-disk index cache is funneled through `clearIndexCache(options)` in `src/indexing/cache.ts`, which targets **only** `resolveIndexRoot(options)` = `<home>/index` (reuses cacheHome/baseDir rules). It returns `{ path, cleared, entries }`. Before any `rmSync(indexRoot, {recursive, force})` it asserts `basename(indexRoot) === 'index' && normalize(indexRoot) !== normalize(home)`, throwing otherwise.

`clear all` runs index removal **then** `clearSavings()` as two independent calls — savings.jsonl is never collateral of an index clear, and `~/.csp/` root is never rmtree'd.

**Why:** AC-015 / track safety constraint — a misconfigured baseDir must not escalate into a home-wide delete that destroys `~/.csp/savings.jsonl`. The T014 dispatch carried a STOP that fires if the computed delete path could include the `~/.csp` root or savings.jsonl.

**How to apply:** When touching clear/cache-deletion code on this track (e.g. T015 README docs, or any future eviction work), keep the `index`-segment guard and the two-independent-actions split intact. Tests inject a temp `baseDir` (never the real home) and assert savings + home survive an index clear. Related: [[csp-cli-cache-di-seam]], [[csp-upstream-no-disk-cache]].
