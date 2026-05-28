import { describe, expect, it } from 'bun:test'

import { chunkSource, DESIRED_CHUNK_LENGTH_CHARS } from './chunk-source.ts'

describe('chunkSource', () => {
  it('returns [] for empty source', async () => {
    expect(await chunkSource('', 'foo.txt', null)).toEqual([])
  })

  it('returns [] for whitespace-only source', async () => {
    expect(await chunkSource('   \n\t\n  ', 'foo.txt', null)).toEqual([])
  })

  it('produces a single chunk for short plain text (no language)', async () => {
    const src = 'hello\nworld\n'
    const chunks = await chunkSource(src, 'foo.txt', null)
    expect(chunks).toHaveLength(1)
    expect(chunks[0]).toMatchObject({
      filePath: 'foo.txt',
      language: null,
      startLine: 1,
      endLine: 2,
    })
    // Content should reproduce the source (minus possibly the very last byte
    // depending on the end-clamp logic).
    expect(chunks[0]!.content.startsWith('hello\nworld')).toBe(true)
  })

  it('chunks ≤ DESIRED_CHUNK_LENGTH_CHARS for long source (line fallback)', async () => {
    // ~3000 chars, well above the 1500-char target.
    const line = `${'x'.repeat(49)}\n` // 50 chars per line
    const src = line.repeat(60) // 3000 chars
    expect(src.length).toBe(3000)

    const chunks = await chunkSource(src, 'big.txt', null)
    expect(chunks.length).toBeGreaterThanOrEqual(2)

    for (const c of chunks)
      expect(c.content.length).toBeLessThanOrEqual(DESIRED_CHUNK_LENGTH_CHARS)
  })

  it('emits 1-indexed start/end line numbers', async () => {
    const src = 'line1\nline2\nline3\nline4\n'
    const chunks = await chunkSource(src, 'foo.txt', null)
    expect(chunks).toHaveLength(1)
    expect(chunks[0]!.startLine).toBe(1)
    // Last line of content is "line4\n" — start of line 4, end is also line 4.
    expect(chunks[0]!.endLine).toBe(4)
  })

  it('falls back to line chunker for an unsupported language', async () => {
    const src = 'a\nb\nc\n'
    const chunks = await chunkSource(src, 'foo.xyz', 'not-a-real-language')
    expect(chunks).toHaveLength(1)
    expect(chunks[0]!.startLine).toBe(1)
    expect(chunks[0]!.language).toBe('not-a-real-language')
  })

  it('preserves filePath on every chunk', async () => {
    const src = `${'a'.repeat(100)}\n`.repeat(50)
    const chunks = await chunkSource(src, 'path/to/file.txt', null)
    expect(chunks.length).toBeGreaterThan(0)
    for (const c of chunks)
      expect(c.filePath).toBe('path/to/file.txt')
  })

  it('start/end lines align with source content across multi-chunk output', async () => {
    // 100 lines × 40 chars = 4000 chars — comfortably above 1500.
    const lines = Array.from({ length: 100 }, (_, i) => `${i.toString().padStart(3, '0')} ${'x'.repeat(35)}`)
    const src = `${lines.join('\n')}\n`
    const chunks = await chunkSource(src, 'foo.txt', null)
    expect(chunks.length).toBeGreaterThanOrEqual(2)

    // First chunk must start at line 1; chunks are sorted; line ranges should
    // be contiguous (next chunk starts on or right after the previous end).
    expect(chunks[0]!.startLine).toBe(1)
    for (let i = 1; i < chunks.length; i++) {
      const prev = chunks[i - 1]!
      const cur = chunks[i]!
      expect(cur.startLine).toBeGreaterThanOrEqual(prev.endLine)
    }
  })
})
