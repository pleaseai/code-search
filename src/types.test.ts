// Tests for src/types.ts — port-parity with src/semble/tests/test_types.py.

import { describe, expect, test } from 'bun:test'
import {
  CallType,
  type Chunk,
  type ChunkDictInput,
  chunkFromDict,
  chunkLocation,
  chunkToDict,
  ContentType,
  type SearchResult,
  searchResultToDict,
} from './types'

describe('ContentType', () => {
  test('enum values match the Python str enum', () => {
    expect(ContentType.Code).toBe('code')
    expect(ContentType.Docs).toBe('docs')
    expect(ContentType.Config).toBe('config')
  })
})

describe('CallType', () => {
  test('enum values match the Python str enum', () => {
    expect(CallType.Search).toBe('search')
    // Python uses `find_related` (snake_case) — telemetry compatibility.
    expect(CallType.FindRelated).toBe('find_related')
  })
})

describe('chunkLocation', () => {
  test('formats as filePath:startLine-endLine', () => {
    const chunk: Chunk = {
      content: 'x = 1',
      filePath: 'file.ts',
      startLine: 10,
      endLine: 25,
    }
    expect(chunkLocation(chunk)).toBe('file.ts:10-25')
  })

  test('handles single-line chunks', () => {
    const chunk: Chunk = {
      content: 'x = 1',
      filePath: 'src/a.py',
      startLine: 5,
      endLine: 5,
    }
    expect(chunkLocation(chunk)).toBe('src/a.py:5-5')
  })
})

describe('chunkToDict / chunkFromDict roundtrip', () => {
  test('preserves all fields with language set', () => {
    const original: Chunk = {
      content: 'function foo() {}',
      filePath: 'src/foo.ts',
      startLine: 1,
      endLine: 3,
      language: 'typescript',
    }
    const dict = chunkToDict(original)
    expect(dict).toEqual({
      content: 'function foo() {}',
      filePath: 'src/foo.ts',
      startLine: 1,
      endLine: 3,
      language: 'typescript',
      location: 'src/foo.ts:1-3',
    })
    const reconstructed = chunkFromDict(dict)
    expect(reconstructed).toEqual(original)
  })

  test('preserves all fields with language omitted (undefined)', () => {
    const original: Chunk = {
      content: 'README content',
      filePath: 'README.md',
      startLine: 1,
      endLine: 10,
    }
    const dict = chunkToDict(original)
    // Python `asdict` emits `None`; we emit `null` to match wire format.
    expect(dict.language).toBeNull()
    expect(dict.location).toBe('README.md:1-10')

    const reconstructed = chunkFromDict(dict)
    expect(reconstructed).toEqual(original)
    expect(reconstructed.language).toBeUndefined()
  })

  test('chunkFromDict strips location before reconstruction', () => {
    // A malformed `location` must not desync the reconstructed Chunk.
    const reconstructed = chunkFromDict({
      content: 'x',
      filePath: 'a.ts',
      startLine: 1,
      endLine: 2,
      language: 'ts',
      location: 'WRONG:999-999',
    })
    // The derived location is recomputed from the line range — never trusted.
    expect(chunkLocation(reconstructed)).toBe('a.ts:1-2')
  })

  test('chunkFromDict accepts null language (wire format)', () => {
    const reconstructed = chunkFromDict({
      content: 'x',
      filePath: 'a.ts',
      startLine: 1,
      endLine: 2,
      language: null,
    })
    expect(reconstructed.language).toBeUndefined()
  })

  test('chunkFromDict throws on null or non-object input', () => {
    // The compile-time `ChunkDictInput` doesn't reach untrusted JSON callers,
    // so the runtime guard must catch these before they pollute the index.
    expect(() => chunkFromDict(null as unknown as ChunkDictInput)).toThrow(TypeError)
    expect(() => chunkFromDict(undefined as unknown as ChunkDictInput)).toThrow(TypeError)
    expect(() => chunkFromDict('oops' as unknown as ChunkDictInput)).toThrow(TypeError)
    expect(() => chunkFromDict(42 as unknown as ChunkDictInput)).toThrow(TypeError)
  })

  test('chunkFromDict throws on missing or wrong-typed required fields', () => {
    expect(() => chunkFromDict({} as unknown as ChunkDictInput)).toThrow(TypeError)
    expect(() =>
      chunkFromDict({ content: 'x', filePath: 'a.ts', startLine: 1 } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 'a.ts',
        startLine: '1',
        endLine: 2,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 42,
        startLine: 1,
        endLine: 2,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
  })

  test('chunkFromDict throws on NaN or non-finite startLine/endLine', () => {
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 'a.ts',
        startLine: Number.NaN,
        endLine: 2,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 'a.ts',
        startLine: 1,
        endLine: Number.POSITIVE_INFINITY,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 'a.ts',
        startLine: Number.NEGATIVE_INFINITY,
        endLine: 2,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
  })

  test('chunkFromDict throws when language has the wrong type', () => {
    expect(() =>
      chunkFromDict({
        content: 'x',
        filePath: 'a.ts',
        startLine: 1,
        endLine: 2,
        language: 42,
      } as unknown as ChunkDictInput),
    ).toThrow(TypeError)
  })
})

describe('searchResultToDict', () => {
  test('serialises chunk and score', () => {
    const chunk: Chunk = {
      content: 'def foo():\n    pass',
      filePath: 'foo.py',
      startLine: 1,
      endLine: 2,
      language: 'python',
    }
    const result: SearchResult = { chunk, score: 0.87 }
    expect(searchResultToDict(result)).toEqual({
      chunk: {
        content: 'def foo():\n    pass',
        filePath: 'foo.py',
        startLine: 1,
        endLine: 2,
        language: 'python',
        location: 'foo.py:1-2',
      },
      score: 0.87,
    })
  })
})
