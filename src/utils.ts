// Port of src/semble/utils.py
// Minimal stub — full implementation lands in Unit 3.

import type { Chunk, SearchResult } from './types.ts'

const GIT_URL_SCHEMES = [
  'https://',
  'http://',
  'ssh://',
  'git://',
  'git+ssh://',
  'file://',
] as const

// scp-style git URL, e.g. `user@host:repo` (but not `user@host:/abs/path`).
const SCP_GIT_URL_RE = /^[\w.-]+@[\w.-]+:(?!\/)/

/** Return true if path looks like a remote git URL rather than a local path. */
export function isGitUrl(path: string): boolean {
  for (const scheme of GIT_URL_SCHEMES) {
    if (path.startsWith(scheme)) {
      return true
    }
  }
  return SCP_GIT_URL_RE.test(path)
}

/**
 * Return the chunk containing `line` in `filePath`, or null.
 *
 * Mirrors semble.utils.resolve_chunk: a strict inner match (`line < endLine`)
 * wins immediately; a boundary match (`line === endLine`) is kept only as a
 * fallback so end-of-file lines still resolve.
 */
export function resolveChunk(
  chunks: Chunk[],
  filePath: string,
  line: number,
): Chunk | null {
  let fallback: Chunk | null = null
  for (const chunk of chunks) {
    if (
      chunk.filePath === filePath
      && chunk.startLine <= line
      && line <= chunk.endLine
    ) {
      if (line < chunk.endLine) {
        return chunk
      }
      if (fallback === null) {
        fallback = chunk
      }
    }
  }
  return fallback
}

/** Render SearchResult objects as a JSONable object. */
export function formatResults(
  query: string,
  results: SearchResult[],
): { query: string, results: Record<string, unknown>[] } {
  return {
    query,
    results: results.map(r => r.toDict()),
  }
}
