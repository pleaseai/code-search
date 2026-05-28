// Port of src/semble/utils.py tests
import { describe, expect, it } from 'bun:test'
import type { Chunk, SearchResult } from './utils.ts'
import { formatResults, isGitUrl, resolveChunk } from './utils.ts'

function makeChunk(overrides: Partial<Chunk> = {}): Chunk {
  return {
    content: 'x',
    filePath: 'a.ts',
    startLine: 1,
    endLine: 10,
    ...overrides,
  }
}

describe('isGitUrl', () => {
  it('returns true for https URLs', () => {
    expect(isGitUrl('https://github.com/foo/bar')).toBe(true)
  })

  it('returns true for http URLs', () => {
    expect(isGitUrl('http://example.com/foo/bar.git')).toBe(true)
  })

  it('returns true for ssh:// URLs', () => {
    expect(isGitUrl('ssh://git@github.com/foo/bar.git')).toBe(true)
  })

  it('returns true for git:// URLs', () => {
    expect(isGitUrl('git://github.com/foo/bar.git')).toBe(true)
  })

  it('returns true for git+ssh:// URLs', () => {
    expect(isGitUrl('git+ssh://git@github.com/foo/bar.git')).toBe(true)
  })

  it('returns true for file:// URLs', () => {
    expect(isGitUrl('file:///path/to/repo')).toBe(true)
  })

  it('returns true for scp-style git URLs', () => {
    expect(isGitUrl('git@github.com:foo/bar.git')).toBe(true)
  })

  it('returns true for scp-style git URLs with dots/dashes', () => {
    expect(isGitUrl('git-user.1@my-host.example.com:foo/bar')).toBe(true)
  })

  it('returns false for relative local paths', () => {
    expect(isGitUrl('./local/path')).toBe(false)
  })

  it('returns false for absolute local paths', () => {
    expect(isGitUrl('/abs/path')).toBe(false)
  })

  it('returns false for bare names', () => {
    expect(isGitUrl('some-repo')).toBe(false)
  })

  it('returns false for scp-like input with a slash after the colon (treated as path)', () => {
    // user@host:/abs/path is ambiguous; semble's regex excludes it via (?!/).
    expect(isGitUrl('user@host:/abs/path')).toBe(false)
  })

  it('returns false for empty string', () => {
    expect(isGitUrl('')).toBe(false)
  })
})

describe('resolveChunk', () => {
  it('returns the inner chunk when line is at the boundary between adjacent chunks', () => {
    // chunkA covers 1..10, chunkB covers 10..20. line=10 belongs strictly inside chunkB.
    const chunkA = makeChunk({ startLine: 1, endLine: 10, content: 'A' })
    const chunkB = makeChunk({ startLine: 10, endLine: 20, content: 'B' })
    const result = resolveChunk([chunkA, chunkB], 'a.ts', 10)
    expect(result).toBe(chunkB)
  })

  it('returns the chunk when line is on its endLine and no inner match exists (fallback)', () => {
    const chunkA = makeChunk({ startLine: 1, endLine: 10, content: 'A' })
    const result = resolveChunk([chunkA], 'a.ts', 10)
    expect(result).toBe(chunkA)
  })

  it('returns the chunk when line is strictly inside it', () => {
    const chunkA = makeChunk({ startLine: 1, endLine: 10, content: 'A' })
    expect(resolveChunk([chunkA], 'a.ts', 5)).toBe(chunkA)
  })

  it('returns the chunk when line equals startLine (strict inner match)', () => {
    const chunkA = makeChunk({ startLine: 1, endLine: 10, content: 'A' })
    expect(resolveChunk([chunkA], 'a.ts', 1)).toBe(chunkA)
  })

  it('returns null when line is outside any chunk', () => {
    const chunkA = makeChunk({ startLine: 1, endLine: 10, content: 'A' })
    expect(resolveChunk([chunkA], 'a.ts', 11)).toBeNull()
  })

  it('returns null when filePath does not match', () => {
    const chunkA = makeChunk({ startLine: 1, endLine: 10, filePath: 'a.ts' })
    expect(resolveChunk([chunkA], 'b.ts', 5)).toBeNull()
  })

  it('returns null for empty chunk list', () => {
    expect(resolveChunk([], 'a.ts', 1)).toBeNull()
  })

  it('ignores chunks from other files when matching', () => {
    const other = makeChunk({ startLine: 1, endLine: 10, filePath: 'b.ts', content: 'B' })
    const wanted = makeChunk({ startLine: 1, endLine: 10, filePath: 'a.ts', content: 'A' })
    expect(resolveChunk([other, wanted], 'a.ts', 5)).toBe(wanted)
  })

  it('keeps the first fallback when no strict inner match is found across multiple end-boundary candidates', () => {
    // Two contiguous end-only matches; the first one wins as the fallback.
    const c1 = makeChunk({ startLine: 1, endLine: 10, content: 'c1' })
    const c2 = makeChunk({ startLine: 10, endLine: 10, content: 'c2' })
    expect(resolveChunk([c1, c2], 'a.ts', 10)).toBe(c1)
  })
})

describe('formatResults', () => {
  it('returns the expected shape', () => {
    const chunkDict = {
      content: 'x',
      file_path: 'a.ts',
      start_line: 1,
      end_line: 5,
      language: null,
      location: 'a.ts:1-5',
    }
    const result: SearchResult = {
      chunk: makeChunk({ startLine: 1, endLine: 5 }),
      score: 0.42,
      toDict: () => ({ chunk: chunkDict, score: 0.42 }),
    }
    const out = formatResults('hello', [result])
    expect(out).toEqual({
      query: 'hello',
      results: [{ chunk: chunkDict, score: 0.42 }],
    })
  })

  it('handles empty results', () => {
    expect(formatResults('q', [])).toEqual({ query: 'q', results: [] })
  })

  it('preserves order of results', () => {
    const r1: SearchResult = {
      chunk: makeChunk(),
      score: 1,
      toDict: () => ({ tag: 'first' }),
    }
    const r2: SearchResult = {
      chunk: makeChunk(),
      score: 0.5,
      toDict: () => ({ tag: 'second' }),
    }
    const out = formatResults('q', [r1, r2])
    expect(out.results).toEqual([{ tag: 'first' }, { tag: 'second' }])
  })
})
