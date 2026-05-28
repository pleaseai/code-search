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

/** Count newline characters in `s` up to (but not including) `endExclusive`. */
function _countNewlines(s: string, endExclusive: number): number {
  let n = 0
  const limit = Math.min(endExclusive, s.length)
  for (let i = 0; i < limit; i++) {
    if (s[i] === '\n')
      n += 1
  }
  return n
}

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

  const chunks: Chunk[] = []
  for (const boundary of chunkBoundaries) {
    // Clamp to start_index so zero-length chunks don't produce an off-by-one.
    const endIndex = Math.max(boundary.end - 1, boundary.start)
    const text = source.slice(boundary.start, endIndex + 1)
    chunks.push({
      content: text,
      filePath,
      startLine: _countNewlines(source, boundary.start) + 1,
      endLine: _countNewlines(source, endIndex) + 1,
      language,
    })
  }
  return chunks
}
