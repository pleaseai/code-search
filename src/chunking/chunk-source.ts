// Port of src/semble/chunking/chunking.py
//
// Public entry point that takes raw source text and a language hint and
// returns concrete `Chunk` values with line numbers. Uses the AST chunker
// when the language is supported, line fallback otherwise.

import { chunk, chunkLines, isSupportedLanguage } from './core.ts'
import type { ChunkBoundary } from './core.ts'

// Inline Chunk type until Unit 1 (types) lands.
// Once `src/types.ts` exists, replace this with:
//   import type { Chunk } from '../types.ts'
export interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language: string | null
}

/** The desired length of chunks in chars. */
export const DESIRED_CHUNK_LENGTH_CHARS = 1500

/** Chunk pre-read source text. */
export async function chunkSource(
  source: string,
  filePath: string,
  language: string | null,
): Promise<Chunk[]> {
  if (source.trim().length === 0)
    return []

  let chunkBoundaries: ChunkBoundary[] | null = null
  if (language !== null && isSupportedLanguage(language))
    chunkBoundaries = await chunk(source, language, DESIRED_CHUNK_LENGTH_CHARS)

  // This is an `if` (not `else`) because the error state of the parser
  // above is `null` — fall through and use the line chunker.
  if (chunkBoundaries === null)
    chunkBoundaries = chunkLines(source, DESIRED_CHUNK_LENGTH_CHARS)

  // Resolve 1-indexed line numbers in a single pass. Boundaries are sorted by
  // their start offset, so we can advance a cursor through `source` once
  // instead of rescanning from index 0 per chunk (avoids O(N²) on large files).
  // Matches semble parity: only `\n` counts as a newline (see chunking.py).
  const chunks: Chunk[] = []
  let cursor = 0
  let line = 1
  const advanceTo = (target: number): number => {
    const limit = Math.min(target, source.length)
    while (cursor < limit) {
      if (source[cursor] === '\n')
        line += 1
      cursor += 1
    }
    return line
  }

  for (const boundary of chunkBoundaries) {
    // Clamp to start_index so zero-length chunks don't produce an off-by-one.
    const endIndex = Math.max(boundary.end - 1, boundary.start)
    const text = source.slice(boundary.start, endIndex + 1)
    const startLine = advanceTo(boundary.start)
    const endLine = advanceTo(endIndex)
    chunks.push({
      content: text,
      filePath,
      startLine,
      endLine,
      language,
    })
  }
  return chunks
}
