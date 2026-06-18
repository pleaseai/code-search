import type { ChunkBoundary } from './core.ts'

import { describe, expect, it } from 'bun:test'
import {
  _mergeAdjacentChunks,
  _mergeNode,
  _mergeNodeInner,
  chunk,
  chunkLines,
  isSupportedLanguage,
  MIN_CHUNK_SIZE,
  RECURSION_DEPTH,
} from './core.ts'

describe('constants', () => {
  it('matches semble defaults', () => {
    expect(RECURSION_DEPTH).toBe(500)
    expect(MIN_CHUNK_SIZE).toBe(50)
  })
})

describe('isSupportedLanguage', () => {
  it('returns true for known languages and false for unknown ones', () => {
    expect(isSupportedLanguage('typescript')).toBe(true)
    expect(isSupportedLanguage('python')).toBe(true)
    expect(isSupportedLanguage('not-a-real-language')).toBe(false)
  })
})

describe('_mergeAdjacentChunks', () => {
  it('returns [] for empty input', () => {
    expect(_mergeAdjacentChunks([], 100)).toEqual([])
  })

  it('passes through a single chunk', () => {
    expect(_mergeAdjacentChunks([{ start: 0, end: 50 }], 100)).toEqual([
      { start: 0, end: 50 },
    ])
  })

  it('merges adjacent chunks under the desired length', () => {
    const input: ChunkBoundary[] = [
      { start: 0, end: 30 },
      { start: 30, end: 60 },
      { start: 60, end: 80 },
    ]
    expect(_mergeAdjacentChunks(input, 100)).toEqual([{ start: 0, end: 80 }])
  })

  it('keeps chunks separate when the merged length exceeds desired', () => {
    const input: ChunkBoundary[] = [
      { start: 0, end: 60 },
      { start: 60, end: 130 },
    ]
    expect(_mergeAdjacentChunks(input, 100)).toEqual([
      { start: 0, end: 60 },
      { start: 60, end: 130 },
    ])
  })

  it('greedily packs groups up to desired length', () => {
    const input: ChunkBoundary[] = [
      { start: 0, end: 40 },
      { start: 40, end: 80 },
      { start: 80, end: 130 },
      { start: 130, end: 160 },
    ]
    // 40 + 40 = 80 (fits), +50 = 130 (exceeds 100) → split, 50+30=80 (fits).
    expect(_mergeAdjacentChunks(input, 100)).toEqual([
      { start: 0, end: 80 },
      { start: 80, end: 160 },
    ])
  })
})

describe('chunkLines', () => {
  it('returns [] for empty source', () => {
    expect(chunkLines('', 100)).toEqual([])
  })

  it('returns [] for whitespace-only source', () => {
    expect(chunkLines('   \n\n\t  \n', 100)).toEqual([])
  })

  it('emits one chunk for short input', () => {
    const src = 'hello\nworld\n'
    const chunks = chunkLines(src, 1500)
    expect(chunks).toHaveLength(1)
    expect(chunks[0]).toEqual({ start: 0, end: src.length })
  })

  it('splits a long source into multiple chunks (each ≤ desired length)', () => {
    // 100 lines × ~40 chars = ~4000 chars total.
    const line = `${'x'.repeat(39)}\n`
    const src = line.repeat(100)
    expect(src.length).toBe(4000)

    const desired = 1500
    const chunks = chunkLines(src, desired)
    expect(chunks.length).toBeGreaterThanOrEqual(2)

    // Each merged chunk should be ≤ desired length, except possibly the
    // tail when a single line exceeds desired (not the case here).
    for (const c of chunks) {
      const len = c.end - c.start
      expect(len).toBeLessThanOrEqual(desired)
    }
  })

  it('chunks contiguously cover the input', () => {
    const src = Array.from({ length: 200 }, (_, i) => `line ${i}\n`).join('')
    const chunks = chunkLines(src, 500)
    expect(chunks[0]!.start).toBe(0)
    expect(chunks[chunks.length - 1]!.end).toBe(src.length)
    for (let i = 1; i < chunks.length; i++) {
      expect(chunks[i]!.start).toBe(chunks[i - 1]!.end)
    }
  })

  it('preserves CRLF line endings in offsets', () => {
    const src = 'a\r\nb\r\nc\r\n'
    const chunks = chunkLines(src, 1500)
    expect(chunks).toHaveLength(1)
    expect(chunks[0]).toEqual({ start: 0, end: src.length })
  })
})

describe('_mergeNode + _mergeNodeInner', () => {
  // Build a fake tree-sitter node tree for unit testing the algorithm.
  interface FakeNode {
    startByte: () => number
    endByte: () => number
    childCount: () => number
    child: (i: number) => FakeNode | null
  }

  function leaf(start: number, end: number): FakeNode {
    return {
      startByte: () => start,
      endByte: () => end,
      childCount: () => 0,
      child: () => null,
    }
  }

  function branch(start: number, end: number, children: FakeNode[]): FakeNode {
    return {
      startByte: () => start,
      endByte: () => end,
      childCount: () => children.length,
      child: (i: number) => children[i] ?? null,
    }
  }

  it('returns a single boundary for a leaf', () => {
    const out = _mergeNodeInner(leaf(10, 60), 100, 0)
    expect(out).toEqual([{ start: 10, end: 60 }])
  })

  it('does not recurse into nodes shorter than MIN_CHUNK_SIZE', () => {
    // length = 40, MIN_CHUNK_SIZE = 50 — must be treated as a leaf.
    const root = branch(0, 40, [leaf(0, 20), leaf(20, 40)])
    expect(_mergeNodeInner(root, 100, 0)).toEqual([{ start: 0, end: 40 }])
  })

  it('caps recursion depth at RECURSION_DEPTH', () => {
    const root = branch(0, 200, [leaf(0, 100), leaf(100, 200)])
    const out = _mergeNodeInner(root, 50, RECURSION_DEPTH + 1)
    expect(out).toEqual([{ start: 0, end: 200 }])
  })

  it('groups children up to the desired length', () => {
    const root = branch(0, 300, [
      leaf(0, 40),
      leaf(40, 80),
      leaf(80, 200),
      leaf(200, 300),
    ])
    // 40+40=80 (fits in 100), +120 would exceed → close group.
    // Then 120 > 100 → recurse (but 120-child has no children) → leaf.
    // Then 100 (fits alone).
    const inner = _mergeNodeInner(root, 100, 0)
    expect(inner).toEqual([
      { start: 0, end: 80 },
      { start: 80, end: 200 },
      { start: 200, end: 300 },
    ])
  })

  it('_mergeNode merges adjacent groups returned by inner', () => {
    // Three small children that each end up alone in inner because they're leaves.
    const root = branch(0, 150, [leaf(0, 30), leaf(30, 60), leaf(60, 150)])
    // inner returns [(0,30), (30,60), (60,150)] when desired=100:
    //   - 30+30=60 fits, +90=150 exceeds → group (0,60), then (60,150).
    // Wait — inner has different logic. Let's verify the actual semble behavior:
    // inner: index=0, child=(0,30) start=0 end=30 len=30, len<=100 not >desired
    //        inner loop: child[1]=(30,60) childLen=30, 30+30=60<=100, end=60 len=60 idx=2
    //                    child[2]=(60,150) childLen=90, 60+90=150>100 → break
    //        push (0,60)
    //        index=2, child=(60,150) start=60 end=150 len=90, len<=100 → push (60,150)
    // → [(0,60),(60,150)]
    // Then _mergeAdjacentChunks with desired=100:
    //   (0,60) curLen=60, (60,150) len=90, 60+90=150>100 → keep separate.
    // → same.
    expect(_mergeNode(root, 100)).toEqual([
      { start: 0, end: 60 },
      { start: 60, end: 150 },
    ])
  })
})

describe('chunk (tree-sitter)', () => {
  it('returns [] for whitespace-only input regardless of language', async () => {
    expect(await chunk('   \n\t\n', 'typescript', 1500)).toEqual([])
    expect(await chunk('', 'python', 1500)).toEqual([])
  })

  // Real tree-sitter parsing is best-effort — depends on Worker 0 installing
  // @kreuzberg/tree-sitter-language-pack. When the parser is unavailable the
  // function returns null and callers fall back to chunkLines.
  it('returns null when no parser is available (line-fallback contract)', async () => {
    // A bogus language guarantees the parser load fails.
    const result = await chunk('let x = 1\n', '__definitely_not_a_real_language__', 1500)
    expect(result).toBeNull()
  })
})
