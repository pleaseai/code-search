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
