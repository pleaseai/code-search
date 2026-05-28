// Port of src/semble/index/index.py
// Minimal stub — full implementation lands in the indexing units.

import type { Chunk, ContentType, SearchResult } from '../types.ts'

export interface CspIndexLoadOptions {
  modelPath?: string
  content?: ContentType[]
}

export interface CspIndexFromGitOptions extends CspIndexLoadOptions {
  ref?: string
}

/**
 * Hybrid (dense + BM25) code search index.
 *
 * This is a stub for the MCP unit; the real implementation lands in the
 * indexing units. Only the surface area used by the MCP server is declared.
 */
export class CspIndex {
  readonly chunks: Chunk[]

  constructor(chunks: Chunk[] = []) {
    this.chunks = chunks
  }

  static async fromPath(
    _path: string,
    _options: CspIndexLoadOptions = {},
  ): Promise<CspIndex> {
    throw new Error('CspIndex.fromPath: not yet implemented (stub)')
  }

  static async fromGit(
    _url: string,
    _options: CspIndexFromGitOptions = {},
  ): Promise<CspIndex> {
    throw new Error('CspIndex.fromGit: not yet implemented (stub)')
  }

  search(_query: string, _options: { topK?: number } = {}): SearchResult[] {
    return []
  }

  findRelated(_chunk: Chunk, _options: { topK?: number } = {}): SearchResult[] {
    return []
  }
}

/** Lazy loader for the embedding model. Returns the cached on-disk path. */
export async function loadModel(): Promise<[unknown, string]> {
  throw new Error('loadModel: not yet implemented (stub)')
}
