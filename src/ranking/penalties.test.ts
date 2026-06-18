// Tests for src/ranking/penalties.ts — parity checked against the Python source.
import { describe, expect, it } from 'bun:test'

import {
  _filePathPenalty,
  FILE_SATURATION_DECAY,
  MILD_PENALTY,
  MODERATE_PENALTY,
  rerankTopK,
  STRONG_PENALTY,
} from './penalties.ts'

interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string
}

function makeChunk(filePath: string, idx = 0): Chunk {
  return {
    content: `chunk ${idx}`,
    filePath,
    startLine: idx,
    endLine: idx + 1,
  }
}

describe('_filePathPenalty', () => {
  it('penalises JS/TS test files with STRONG_PENALTY', () => {
    expect(_filePathPenalty('src/foo.test.ts')).toBe(STRONG_PENALTY)
  })

  it('penalises .spec.tsx files with STRONG_PENALTY', () => {
    expect(_filePathPenalty('src/foo.spec.tsx')).toBe(STRONG_PENALTY)
  })

  it('penalises __init__.py with MODERATE_PENALTY (re-export barrel)', () => {
    expect(_filePathPenalty('src/__init__.py')).toBe(MODERATE_PENALTY)
  })

  it('penalises .d.ts type stubs with MILD_PENALTY', () => {
    expect(_filePathPenalty('src/foo.d.ts')).toBe(MILD_PENALTY)
  })

  it('penalises files under tests/ — TEST_DIR + TEST_FILE share one STRONG branch', () => {
    // Python parity: only one STRONG_PENALTY multiplication regardless of how
    // many of {TEST_FILE_RE, TEST_DIR_RE} match (they are OR'd in one branch).
    expect(_filePathPenalty('tests/test_foo.py')).toBeCloseTo(STRONG_PENALTY, 10)
  })

  it('returns 1.0 for ordinary source files', () => {
    expect(_filePathPenalty('src/foo.ts')).toBe(1.0)
  })

  it('compounds STRONG (examples/) and STRONG (.test.ts) penalties', () => {
    // Python: examples/foo.test.ts -> 0.09
    expect(_filePathPenalty('examples/foo.test.ts')).toBeCloseTo(STRONG_PENALTY * STRONG_PENALTY, 10)
  })

  it('compounds MILD (.d.ts) and MODERATE (__init__) penalties', () => {
    // Python: src/__init__.d.ts -> 0.7 (only .d.ts matches; basename is __init__.d.ts)
    expect(_filePathPenalty('src/__init__.d.ts')).toBe(MILD_PENALTY)
  })

  it('penalises compat dirs with STRONG_PENALTY', () => {
    expect(_filePathPenalty('compat/foo.ts')).toBe(STRONG_PENALTY)
  })

  it('penalises examples dirs with STRONG_PENALTY', () => {
    expect(_filePathPenalty('examples/foo.ts')).toBe(STRONG_PENALTY)
  })

  it('normalises backslashes to forward slashes before matching', () => {
    expect(_filePathPenalty('src\\foo.test.ts')).toBe(STRONG_PENALTY)
  })

  it('handles bare __init__.py basename without path', () => {
    expect(_filePathPenalty('__init__.py')).toBe(MODERATE_PENALTY)
  })

  it('penalises Go _test.go files', () => {
    expect(_filePathPenalty('pkg/foo_test.go')).toBe(STRONG_PENALTY)
  })

  it('penalises Java FooTests.java files', () => {
    expect(_filePathPenalty('src/FooTests.java')).toBe(STRONG_PENALTY)
  })

  it('penalises legacy dirs with STRONG_PENALTY', () => {
    expect(_filePathPenalty('legacy/foo.ts')).toBe(STRONG_PENALTY)
  })
})

describe('rerankTopK', () => {
  it('returns an empty list for empty input', () => {
    expect(rerankTopK(new Map(), 5)).toEqual([])
  })

  it('returns an empty list for non-positive topK', () => {
    const a = makeChunk('a.ts', 0)
    const scores = new Map<Chunk, number>([[a, 1.0]])
    expect(rerankTopK(scores, 0)).toEqual([])
    expect(rerankTopK(scores, -1)).toEqual([])
    expect(rerankTopK(scores, -5)).toEqual([])
  })

  it('applies saturation decay to chunks from the same file', () => {
    // 4 chunks from the same file, all initial score 1.0, no path penalty.
    const a = makeChunk('src/foo.ts', 0)
    const b = makeChunk('src/foo.ts', 1)
    const c = makeChunk('src/foo.ts', 2)
    const d = makeChunk('src/foo.ts', 3)
    const scores = new Map<Chunk, number>([
      [a, 1.0],
      [b, 1.0],
      [c, 1.0],
      [d, 1.0],
    ])
    const result = rerankTopK(scores, 4, { penalisePaths: false })
    expect(result).toHaveLength(4)
    // Sorted descending after decay; ties preserved by sort stability of computation.
    const finalScores = result.map(([, s]) => s)
    // First chunk picked: 1.0 (no decay)
    // Second:  1.0 * 0.5   = 0.5
    // Third:   1.0 * 0.25  = 0.25
    // Fourth:  1.0 * 0.125 = 0.125
    expect(finalScores[0]).toBeCloseTo(1.0, 10)
    expect(finalScores[1]).toBeCloseTo(FILE_SATURATION_DECAY, 10)
    expect(finalScores[2]).toBeCloseTo(FILE_SATURATION_DECAY ** 2, 10)
    expect(finalScores[3]).toBeCloseTo(FILE_SATURATION_DECAY ** 3, 10)
  })

  it('truncates to topK after sorting', () => {
    const a = makeChunk('a.ts', 0)
    const b = makeChunk('b.ts', 1)
    const c = makeChunk('c.ts', 2)
    const scores = new Map<Chunk, number>([
      [a, 0.5],
      [b, 0.9],
      [c, 0.1],
    ])
    const result = rerankTopK(scores, 2, { penalisePaths: false })
    expect(result).toHaveLength(2)
    expect(result[0]![0]).toBe(b)
    expect(result[1]![0]).toBe(a)
  })

  it('applies path penalties before sorting when enabled', () => {
    // a is a test file (penalty 0.3), b is normal. a wins pre-penalty, b wins post.
    const a = makeChunk('src/foo.test.ts', 0)
    const b = makeChunk('src/foo.ts', 1)
    const scores = new Map<Chunk, number>([
      [a, 0.9],
      [b, 0.5],
    ])
    const result = rerankTopK(scores, 2)
    expect(result[0]![0]).toBe(b)
    expect(result[1]![0]).toBe(a)
    expect(result[0]![1]).toBeCloseTo(0.5, 10)
    expect(result[1]![1]).toBeCloseTo(0.9 * STRONG_PENALTY, 10)
  })

  it('does not apply path penalties when penalisePaths is false', () => {
    const a = makeChunk('src/foo.test.ts', 0)
    const b = makeChunk('src/foo.ts', 1)
    const scores = new Map<Chunk, number>([
      [a, 0.9],
      [b, 0.5],
    ])
    const result = rerankTopK(scores, 2, { penalisePaths: false })
    expect(result[0]![0]).toBe(a)
    expect(result[0]![1]).toBeCloseTo(0.9, 10)
    expect(result[1]![0]).toBe(b)
    expect(result[1]![1]).toBeCloseTo(0.5, 10)
  })

  it('mixes saturation decay across multiple files', () => {
    // Two files, two chunks each. All score 1.0. topK = 4.
    const a1 = makeChunk('a.ts', 0)
    const a2 = makeChunk('a.ts', 1)
    const b1 = makeChunk('b.ts', 2)
    const b2 = makeChunk('b.ts', 3)
    const scores = new Map<Chunk, number>([
      [a1, 1.0],
      [a2, 1.0],
      [b1, 1.0],
      [b2, 1.0],
    ])
    const result = rerankTopK(scores, 4, { penalisePaths: false })
    expect(result).toHaveLength(4)
    // First two picked at 1.0 (first of each file), next two at 0.5.
    const top = result.map(([, s]) => s)
    expect(top[0]).toBeCloseTo(1.0, 10)
    expect(top[1]).toBeCloseTo(1.0, 10)
    expect(top[2]).toBeCloseTo(FILE_SATURATION_DECAY, 10)
    expect(top[3]).toBeCloseTo(FILE_SATURATION_DECAY, 10)
  })
})
