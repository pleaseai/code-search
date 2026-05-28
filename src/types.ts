// TODO(unit-1): replace with the real port from `feat/unit-1-types`.
//
// This file is a *placeholder stub* so the public barrel (`src/index.ts`)
// type-checks and `bun test src/index.test.ts` can import the package in
// isolation. Unit 1 lands the real port of `src/semble/types.py`; when it
// merges, this file is overwritten wholesale (see PR `feat/unit-1-types`).
//
// Keep the exported names and value/type duality of `ContentType` in lockstep
// with Unit 1 — the barrel re-exports both forms.

/**
 * Content type for indexing and search pipeline selection.
 *
 * Placeholder mirroring Unit 1's `const`-object enum. Values are the same
 * lowercase strings as the upstream Python `str` enum so CLI flags and
 * persisted indices round-trip.
 */
export const ContentType = {
  Code: 'code',
  Docs: 'docs',
  Config: 'config',
} as const
export type ContentType = (typeof ContentType)[keyof typeof ContentType]

/** Placeholder shape — Unit 1 ships the authoritative definition. */
export interface Chunk {
  readonly content: string
  readonly filePath: string
  readonly startLine: number
  readonly endLine: number
  readonly language?: string | undefined
}

/** Placeholder shape — Unit 1 ships the authoritative definition. */
export interface SearchResult {
  readonly chunk: Chunk
  readonly score: number
}

/** Placeholder shape — Unit 1 ships the authoritative definition. */
export interface IndexStats {
  readonly indexedFiles: number
  readonly totalChunks: number
  readonly languages: Readonly<Record<string, number>>
}

/** Placeholder alias — Unit 1 ships the authoritative definition. */
export type EmbeddingMatrix = Float32Array
