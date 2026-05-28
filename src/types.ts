// Port of src/semble/types.py
//
// Public field names are camelCase (not snake_case) — see ARCHITECTURE.md:
// "Public field names are camelCase, not snake_case." The upstream Python
// exposes `chunk.file_path` / `start_line` / `end_line`; the TS port exposes
// `filePath` / `startLine` / `endLine`. This is load-bearing for the public
// surface documented in README.md.

/**
 * Call type for token-savings tracking.
 *
 * Port of `semble.types.CallType`. Values match the Python `str` enum so
 * serialised telemetry (`~/.csp/savings.jsonl`) stays compatible.
 */
export const CallType = {
  Search: 'search',
  FindRelated: 'find_related',
} as const
export type CallType = (typeof CallType)[keyof typeof CallType]

/**
 * Content type for indexing and search pipeline selection.
 *
 * Port of `semble.types.ContentType`. Values match the Python `str` enum
 * (`'code' | 'docs' | 'config'`) so CLI flags (`--content code`) and persisted
 * indices round-trip across the two implementations.
 */
export const ContentType = {
  Code: 'code',
  Docs: 'docs',
  Config: 'config',
} as const
export type ContentType = (typeof ContentType)[keyof typeof ContentType]

/**
 * A single indexable unit of code.
 *
 * Port of `semble.types.Chunk` (frozen dataclass). Fields are camelCase per
 * the public surface contract; use {@link chunkFromDict} to construct from
 * serialised data and {@link chunkToDict} to serialise.
 *
 * Treat instances as immutable — helpers do not mutate, and consumers should
 * not either. `readonly` makes the shape compile-time immutable; we don't
 * `Object.freeze` at construction time to avoid the runtime cost on hot paths
 * (large `Chunk[]` arrays during indexing).
 */
export interface Chunk {
  readonly content: string
  readonly filePath: string
  readonly startLine: number
  readonly endLine: number
  readonly language?: string | undefined
}

/**
 * A single search result with score and source.
 *
 * Port of `semble.types.SearchResult`.
 */
export interface SearchResult {
  readonly chunk: Chunk
  readonly score: number
}

/**
 * Statistics about the current index state.
 *
 * Port of `semble.types.IndexStats`.
 */
export interface IndexStats {
  readonly indexedFiles: number
  readonly totalChunks: number
  readonly languages: Readonly<Record<string, number>>
}

/**
 * Flat row-major Float32 embedding matrix.
 *
 * Port of `semble.types.EmbeddingMatrix` (`npt.NDArray[np.float32]`).
 *
 * We use a single `Float32Array` (row-major) instead of `Float32Array[]`
 * because:
 *   1. Dense retrieval computes `embeddings @ query` as one contiguous BLAS-
 *      style sweep — a flat buffer keeps that hot loop cache-friendly and
 *      avoids per-row indirection.
 *   2. Persistence (semble pickles the numpy matrix) maps cleanly onto a
 *      single binary blob without per-row length headers.
 * The companion {@link EmbeddingShape} carries `(rows, dim)` since a flat
 * `Float32Array` has lost that information.
 */
export type EmbeddingMatrix = Float32Array

/** Shape companion for a flat row-major {@link EmbeddingMatrix}. */
export interface EmbeddingShape {
  readonly rows: number
  readonly dim: number
}

/**
 * Format a chunk's source location as `filePath:startLine-endLine`.
 *
 * Port of the `Chunk.location` `@property` in Python. Kept as a free function
 * because `Chunk` is a plain interface (no methods) in the TS port.
 */
export function chunkLocation(chunk: Chunk): string {
  return `${chunk.filePath}:${chunk.startLine}-${chunk.endLine}`
}

/**
 * Serialised form of a {@link Chunk}.
 *
 * `location` is included for consumer convenience (matches Python
 * `Chunk.to_dict`) and is reconstructed from the other fields, never trusted
 * on the way back in — see {@link chunkFromDict}.
 */
export interface ChunkDict {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language: string | null
  location: string
}

/**
 * Convert a {@link Chunk} to a plain serialisable object.
 *
 * Port of `Chunk.to_dict`. Includes the derived `location` field. Mirrors
 * Python's `dataclasses.asdict`, which represents `Optional[str] = None` as
 * literal `null` rather than omitting the key — keeping that shape preserves
 * JSON parity across the two implementations.
 */
export function chunkToDict(chunk: Chunk): ChunkDict {
  return {
    content: chunk.content,
    filePath: chunk.filePath,
    startLine: chunk.startLine,
    endLine: chunk.endLine,
    language: chunk.language ?? null,
    location: chunkLocation(chunk),
  }
}

/** Input shape accepted by {@link chunkFromDict} — `location` is ignored. */
export interface ChunkDictInput {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string | null | undefined
  location?: string | undefined
}

/**
 * Reconstruct a {@link Chunk} from a {@link ChunkDict}.
 *
 * Port of `Chunk.from_dict`. The `location` field, if present, is stripped
 * before construction (it's a derived value; trusting it on the way in would
 * let a malformed payload desynchronise it from the line range).
 *
 * This is a trust boundary: TypeScript's compile-time `ChunkDictInput` is
 * bypassed when parsing untrusted JSON (persisted indices, MCP payloads,
 * external callers). Validate at runtime so malformed input fails loudly
 * with a `TypeError` instead of producing a `Chunk` with `NaN` line numbers
 * or `undefined` fields that surface as confusing errors deeper in the
 * pipeline.
 */
export function chunkFromDict(data: ChunkDictInput): Chunk {
  if (data === null || typeof data !== 'object') {
    throw new TypeError('chunkFromDict: data must be a non-null object')
  }
  const d = data as Record<string, unknown>
  if (typeof d.content !== 'string'
    || typeof d.filePath !== 'string'
    || typeof d.startLine !== 'number'
    || typeof d.endLine !== 'number') {
    throw new TypeError(
      'chunkFromDict: missing or invalid required fields '
      + '(content: string, filePath: string, startLine: number, endLine: number)',
    )
  }
  if (d.language !== undefined && d.language !== null && typeof d.language !== 'string') {
    throw new TypeError('chunkFromDict: language must be a string, null, or omitted')
  }
  // `exactOptionalPropertyTypes` distinguishes "language: undefined" from
  // omitted; build the object conditionally so the resulting Chunk matches
  // the `language?: string | undefined` signature exactly.
  const language = d.language ?? undefined
  return language === undefined
    ? {
        content: d.content,
        filePath: d.filePath,
        startLine: d.startLine,
        endLine: d.endLine,
      }
    : {
        content: d.content,
        filePath: d.filePath,
        startLine: d.startLine,
        endLine: d.endLine,
        language: language as string,
      }
}

/** Serialised form of a {@link SearchResult}. */
export interface SearchResultDict {
  chunk: ChunkDict
  score: number
}

/**
 * Convert a {@link SearchResult} to a plain serialisable object.
 *
 * Port of `SearchResult.to_dict`.
 */
export function searchResultToDict(result: SearchResult): SearchResultDict {
  return {
    chunk: chunkToDict(result.chunk),
    score: result.score,
  }
}
