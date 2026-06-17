---
name: csp-cli-cache-di-seam
description: cli wires auto-cache via injectable loadOrBuild seam; cache only on build branch so --index (loadFromDisk) is untouched; mcp must mirror the same key
metadata:
  type: project
---

cli.ts (`runCli`) wires the `~/.csp/index/<key>` auto-cache for `search`/`find-related` through an injectable seam `RunOptions.loadOrBuild?(source,{content,ref?})` (default `_defaultLoadOrBuild` → `loadOrBuildIndex`). The cache is applied **only on the build (else) branch** — the `--index` branch keeps loading via `readIndex`/`loadFromDisk` directly.

**Why:**
- `loadOrBuildIndex` (cache.ts) does **not** accept build-fn injection, so the only place to inject a test seam that avoids touching the real `~/.csp` home is the cli layer. Tests pass `loadOrBuild` to assert routing and stay off disk.
- Auto-cache on build-only keeps T008's explicit-path guarantee intact: `--index <p>` must always load that exact path (mutually-exclusive if/else; build path never runs when `--index` is set).
- `_defaultLoadOrBuild` re-narrows `ref` (omit when undefined) because `LoadOrBuildOptions.ref` is `string` under `exactOptionalPropertyTypes` — spreading `ref: undefined` is a type error.

**How to apply:**
- T012 (mcp ↔ disk-cache alignment, same Phase C PR) must use the same `loadOrBuildIndex(source, {content, ref})` contract and omit `ref` when absent — otherwise cli and mcp compute different `~/.csp/index/<key>` for the same source/content/ref and present divergent cache views.
- When testing cache-backed cli paths, inject the seam; never let the default hit real `homedir()/.csp`.
- See [[csp-loadorbuild-cache-contract]] for how the key is derived (local=source-file hash via `save(dir,{contentHash})`; git=URL+ref only).
