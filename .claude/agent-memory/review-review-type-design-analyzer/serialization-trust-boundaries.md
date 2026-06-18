---
name: serialization-trust-boundaries
description: csp on-disk (camelCase) vs wire (snake_case) serialization split, and which deserialization boundaries are runtime-validated
metadata:
  type: project
---

csp has two distinct chunk serializations that must not be conflated:
- **On-disk / round-trip = camelCase** — owned by `src/types.ts` (`chunkToDict`/`chunkFromDict`/`ChunkDict`/`ChunkDictInput`), used for `chunks.json`.
- **Wire format (CLI/MCP JSON) = snake_case** — `SearchResult.toDict` closures in `src/search.ts` (`file_path`, `start_line`) and duplicated inline in `index.ts` `makeRelatedResult`.

**Why:** different audiences (disk persistence vs external JSON consumers); the type comment at `types.ts:43-48` states they "must not be conflated."

**How to apply:** When reviewing serialization in this repo, check which audience a `toDict` targets before flagging a casing mismatch — both casings are intentional.

Deserialization trust-boundary status (resolved in PR #21):
- `chunkFromDict` (types.ts) is a **proper runtime guard** (throws TypeError on malformed input) — the model to follow.
- `IndexManifest` is now **runtime-validated** by `parseManifest()` (`index.ts`), which checks `schemaVersion`/`contentHash`/`sourceId`/`modelId`/`content` (every field) and throws `Invalid manifest: …` on a bad value — mirroring `chunkFromDict`. `loadFromDisk` (version-check-first, then `parseManifest`) and `tryReuse` (cache.ts) both route through it; no remaining `JSON.parse(...) as IndexManifest` on the load path.
