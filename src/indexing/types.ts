// Port of src/semble/index/types.py

import { existsSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Resolved on-disk paths used by the index save/load roundtrip.
 *
 * Mirrors `semble.index.types.PersistencePath`.
 */
export class PersistencePath {
  readonly chunks: string
  readonly bm25Index: string
  readonly semanticIndex: string
  readonly metadata: string

  constructor(opts: {
    chunks: string
    bm25Index: string
    semanticIndex: string
    metadata: string
  }) {
    this.chunks = opts.chunks
    this.bm25Index = opts.bm25Index
    this.semanticIndex = opts.semanticIndex
    this.metadata = opts.metadata
  }

  /** Return absolute paths that don't currently exist on disk. */
  nonExisting(): string[] {
    return [this.chunks, this.bm25Index, this.semanticIndex, this.metadata]
      .filter(p => !existsSync(p))
  }

  /** Build a PersistencePath rooted at `base`. */
  static fromPath(base: string): PersistencePath {
    return new PersistencePath({
      chunks: join(base, 'chunks.json'),
      bm25Index: join(base, 'bm25_index'),
      semanticIndex: join(base, 'semantic_index'),
      metadata: join(base, 'metadata.json'),
    })
  }
}
