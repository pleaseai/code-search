import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'

import { Bm25Index, type Chunk, enrichForBm25, selectorToMask } from './sparse.ts'

function makeChunk(overrides: Partial<Chunk> & { filePath: string, content?: string }): Chunk {
  return {
    content: overrides.content ?? '',
    filePath: overrides.filePath,
    startLine: overrides.startLine ?? 1,
    endLine: overrides.endLine ?? 1,
    language: overrides.language ?? null,
  }
}

describe('enrichForBm25', () => {
  test('appends repeated stem and last 3 dir parts (2-part dir)', () => {
    // Mirrors upstream Python: Path('src/utils/format.ts').parent.parts == ('src', 'utils'),
    // so last-3 is the full ['src', 'utils'].
    const out = enrichForBm25(makeChunk({ filePath: 'src/utils/format.ts', content: 'hello world' }))
    expect(out).toBe('hello world format format src utils')
  })

  test('trims to the last 3 dir parts (4-part dir)', () => {
    const out = enrichForBm25(makeChunk({ filePath: 'a/b/c/d/foo.py', content: 'x' }))
    expect(out).toBe('x foo foo b c d')
  })

  test('handles a top-level file with no directory components', () => {
    const out = enrichForBm25(makeChunk({ filePath: 'foo.py', content: 'x' }))
    expect(out).toBe('x foo foo ')
  })

  test('drops "." pseudo-segments from relative paths', () => {
    const out = enrichForBm25(makeChunk({ filePath: './a/b/foo.ts', content: 'x' }))
    expect(out).toBe('x foo foo a b')
  })
})

describe('selectorToMask', () => {
  test('builds a 0/1 mask the same length as `size`', () => {
    const mask = selectorToMask(new Uint32Array([0, 2, 5]), 6)
    expect(mask).not.toBeNull()
    expect(Array.from(mask!)).toEqual([1, 0, 1, 0, 0, 1])
  })

  test('returns null for a null selector', () => {
    expect(selectorToMask(null, 6)).toBeNull()
  })

  test('returns null for an undefined selector', () => {
    expect(selectorToMask(undefined, 6)).toBeNull()
  })

  test('ignores indices outside the mask bounds', () => {
    // Out-of-bounds indices are silently dropped rather than crashing —
    // upstream relies on the selector being well-formed but we want to be
    // defensive in the TS port.
    const mask = selectorToMask(new Uint32Array([0, 10]), 3)
    expect(Array.from(mask!)).toEqual([1, 0, 0])
  })
})

describe('Bm25Index.build / getScores', () => {
  test('ranks documents containing the query term higher', () => {
    const index = Bm25Index.build([
      ['hello', 'world'],
      ['hello'],
      ['world'],
    ])
    const scores = index.getScores(['hello'])
    expect(scores).toHaveLength(3)
    expect(scores[0]).toBeGreaterThan(0)
    expect(scores[1]).toBeGreaterThan(0)
    expect(scores[2]).toBe(0)
  })

  test('returns zero scores for unknown query tokens', () => {
    const index = Bm25Index.build([['hello'], ['world']])
    const scores = index.getScores(['unknown'])
    expect(Array.from(scores)).toEqual([0, 0])
  })

  test('returns an empty-array-equivalent for an empty corpus', () => {
    const index = Bm25Index.build([])
    const scores = index.getScores(['anything'])
    expect(scores).toHaveLength(0)
  })

  test('returns zero scores when query tokens are empty', () => {
    const index = Bm25Index.build([['hello'], ['world']])
    const scores = index.getScores([])
    expect(Array.from(scores)).toEqual([0, 0])
  })

  test('weightMask zeros out masked-out documents', () => {
    const index = Bm25Index.build([
      ['hello', 'world'],
      ['hello'],
      ['world'],
    ])
    // Mask in docs 0 and 2 only.
    const mask = new Uint8Array([1, 0, 1])
    const scores = index.getScores(['hello'], mask)
    expect(scores[0]).toBeGreaterThan(0)
    expect(scores[1]).toBe(0)
    expect(scores[2]).toBe(0) // doc 2 doesn't contain 'hello'
  })

  test('weightMask only suppresses scores; matched-in docs are unchanged', () => {
    const index = Bm25Index.build([
      ['hello', 'world'],
      ['hello'],
      ['world'],
    ])
    const baseline = index.getScores(['hello'])
    const masked = index.getScores(['hello'], new Uint8Array([1, 1, 1]))
    expect(Array.from(masked)).toEqual(Array.from(baseline))
  })

  test('repeated query tokens do not compound scores', () => {
    const index = Bm25Index.build([['hello']])
    const single = index.getScores(['hello'])
    const repeated = index.getScores(['hello', 'hello', 'hello'])
    expect(Array.from(repeated)).toEqual(Array.from(single))
  })
})

describe('Bm25Index.save / load', () => {
  let tmp: string

  beforeEach(async () => {
    tmp = await mkdtemp(path.join(tmpdir(), 'csp-bm25-'))
  })

  afterEach(async () => {
    await rm(tmp, { recursive: true, force: true })
  })

  test('round-trips an index and preserves scores', async () => {
    const index = Bm25Index.build([
      ['alpha', 'beta'],
      ['alpha'],
      ['beta', 'gamma'],
    ])
    await index.save(tmp)
    const loaded = await Bm25Index.load(tmp)
    const original = index.getScores(['alpha'])
    const restored = loaded.getScores(['alpha'])
    expect(Array.from(restored)).toEqual(Array.from(original))
  })
})
