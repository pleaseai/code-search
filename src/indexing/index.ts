// TODO(unit-12): replace with the real CspIndex implementation.
//
// This file is a *placeholder stub* so the public barrel (`src/index.ts`)
// type-checks and `bun test src/index.test.ts` can import the package in
// isolation. Unit 12 lands the real port of `src/semble/index/index.py`;
// when it merges, this file is overwritten wholesale.
//
// The barrel only re-exports the *name* `CspIndex` — consumers don't
// instantiate it from this stub. Keeping the placeholder as a class (rather
// than a stand-in `const`) means the `typeof CspIndex === 'function'` check
// in `src/index.test.ts` is satisfied without a working implementation
// behind it.

import type { Chunk, IndexStats, SearchResult } from '../types.ts'

/**
 * Hybrid (dense + BM25) code-search index.
 *
 * Placeholder — Unit 12 ships the authoritative implementation porting
 * `semble.index.index.SembleIndex` (factories `fromPath`/`fromGit`, search /
 * findRelated, save/load, stats).
 */
export class CspIndex {
  // Throw eagerly so an accidental `new CspIndex()` against the stub fails
  // fast with a clear message, instead of looking like a working empty index.
  constructor() {
    throw new Error(
      'CspIndex is a placeholder stub — Unit 12 (`feat/unit-12-index`) ships the real implementation.',
    )
  }

  // Method signatures are intentionally omitted; the barrel only needs the
  // class to *exist* as a value export. Consumers reaching for `.fromPath()`
  // etc. against this stub would be using it before Unit 12 has merged,
  // which is a sequencing bug worth surfacing as a `TypeError` at call site.

  /** Placeholder — see Unit 12. */
  static fromPath(..._args: unknown[]): Promise<CspIndex> {
    return Promise.reject(new Error('CspIndex.fromPath: not implemented (Unit 12).'))
  }

  /** Placeholder — see Unit 12. */
  static fromGit(..._args: unknown[]): Promise<CspIndex> {
    return Promise.reject(new Error('CspIndex.fromGit: not implemented (Unit 12).'))
  }

  /** Placeholder — see Unit 12. */
  static load(..._args: unknown[]): Promise<CspIndex> {
    return Promise.reject(new Error('CspIndex.load: not implemented (Unit 12).'))
  }

  /** Placeholder — see Unit 12. */
  search(..._args: unknown[]): SearchResult[] {
    throw new Error('CspIndex.search: not implemented (Unit 12).')
  }

  /** Placeholder — see Unit 12. */
  findRelated(..._args: unknown[]): SearchResult[] {
    throw new Error('CspIndex.findRelated: not implemented (Unit 12).')
  }

  /** Placeholder — see Unit 12. */
  save(..._args: unknown[]): Promise<void> {
    return Promise.reject(new Error('CspIndex.save: not implemented (Unit 12).'))
  }

  /** Placeholder — see Unit 12. */
  get stats(): IndexStats {
    throw new Error('CspIndex.stats: not implemented (Unit 12).')
  }

  /** Placeholder — see Unit 12. */
  get chunks(): readonly Chunk[] {
    throw new Error('CspIndex.chunks: not implemented (Unit 12).')
  }
}
