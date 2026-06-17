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

Deserialization trust-boundary status (as of PR #21 review):
- `chunkFromDict` (types.ts) is a **proper runtime guard** (throws TypeError on malformed input) — the model to follow.
- `IndexManifest` is **NOT validated** — `loadFromDisk` (index.ts) and `tryReuse` (cache.ts) both `JSON.parse(...) as IndexManifest`. Only `schemaVersion` is checked; `content`/`modelId`/`sourceId` flow in unvalidated. Recurring review flag: recommend a `parseManifest()` guard mirroring `chunkFromDict`. See [[serialization-trust-boundaries]].
