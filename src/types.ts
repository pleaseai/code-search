// Port of src/semble/types.py
// Minimal stub — full implementation lands in Unit 1.

/** Call type for token-savings tracking. */
export enum CallType {
  SEARCH = 'search',
  FIND_RELATED = 'find_related',
}

/** Content type for indexing and search pipeline selection. */
export enum ContentType {
  CODE = 'code',
  DOCS = 'docs',
  CONFIG = 'config',
}

/** A single indexable unit of code. */
export interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string | null
}

/** A single search result with score and source. */
export interface SearchResult {
  chunk: Chunk
  score: number
  toDict: () => Record<string, unknown>
}

/** Statistics about the current index state. */
export interface IndexStats {
  indexedFiles: number
  totalChunks: number
  languages: Record<string, number>
}

// ---------------------------------------------------------------------------
// Canonical camelCase round-trip serialization
// ---------------------------------------------------------------------------
//
// These helpers are the on-disk / round-trip representation of a `Chunk`:
// camelCase field names (matching the in-memory `Chunk`) plus a derived
// `location`. They are intentionally *separate* from `search.ts`'s wire-format
// `SearchResult.toDict` (snake_case, for CLI/MCP JSON output) — the two
// serializations have different audiences and must not be conflated.

/** A chunk serialized to a plain camelCase dict (e.g. for `chunks.json`). */
export interface ChunkDict {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language: string | null
  location: string
}

/**
 * Input accepted by {@link chunkFromDict}. Mirrors {@link ChunkDict} but the
 * derived `location` is optional (and ignored on reconstruction).
 */
export interface ChunkDictInput {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string | null
  location?: string
}

/** Format a chunk's source location as `filePath:startLine-endLine`. */
export function chunkLocation(chunk: Chunk): string {
  return `${chunk.filePath}:${chunk.startLine}-${chunk.endLine}`
}

/**
 * Serialize a {@link Chunk} to a camelCase {@link ChunkDict}. `language` is
 * normalized to `null` when absent (matching Python `asdict`'s `None`), and a
 * derived `location` is appended.
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

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

/**
 * Reconstruct a {@link Chunk} from a {@link ChunkDictInput}. The derived
 * `location` is stripped (never trusted — it is recomputed from the line
 * range), `null` language collapses to `undefined`, and malformed input throws
 * a `TypeError` so corrupt JSON can't pollute the index.
 */
export function chunkFromDict(dict: ChunkDictInput): Chunk {
  if (dict === null || typeof dict !== 'object') {
    throw new TypeError('chunkFromDict: expected an object')
  }

  const { content, filePath, startLine, endLine, language } = dict as unknown as Record<string, unknown>

  if (typeof content !== 'string') {
    throw new TypeError('chunkFromDict: `content` must be a string')
  }
  if (typeof filePath !== 'string') {
    throw new TypeError('chunkFromDict: `filePath` must be a string')
  }
  if (!isFiniteNumber(startLine)) {
    throw new TypeError('chunkFromDict: `startLine` must be a finite number')
  }
  if (!isFiniteNumber(endLine)) {
    throw new TypeError('chunkFromDict: `endLine` must be a finite number')
  }
  if (language !== undefined && language !== null && typeof language !== 'string') {
    throw new TypeError('chunkFromDict: `language` must be a string, null, or omitted')
  }

  const chunk: Chunk = { content, filePath, startLine, endLine }
  if (typeof language === 'string') {
    chunk.language = language
  }
  return chunk
}

/**
 * Serialize a search result to a camelCase dict, embedding the camelCase
 * {@link ChunkDict}. Counterpart to {@link chunkToDict} for results. Accepts
 * the structural `{ chunk, score }` subset so it does not require the
 * wire-format `toDict` closure carried by full {@link SearchResult} values.
 */
export function searchResultToDict(
  result: { chunk: Chunk, score: number },
): { chunk: ChunkDict, score: number } {
  return {
    chunk: chunkToDict(result.chunk),
    score: result.score,
  }
}
