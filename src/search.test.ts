import { describe, expect, it } from 'bun:test'
import type {
  Bm25Index,
  Chunk,
  Model,
  SelectableBasicBackend,
} from './search.ts'
import {
  _rrfScores,
  _searchBm25,
  _searchSemantic,
  _sortTopK,
  RRF_K,
  search,
} from './search.ts'

// ---------- Fixtures ---------------------------------------------------------

function makeChunk(overrides: Partial<Chunk>): Chunk {
  return {
    content: '',
    filePath: 'src/a.ts',
    startLine: 1,
    endLine: 10,
    language: 'ts',
    ...overrides,
  }
}

function makeChunks(): Chunk[] {
  return [
    makeChunk({ content: 'class Alpha {}', filePath: 'src/alpha.ts', startLine: 10, endLine: 20 }),
    makeChunk({ content: 'function beta() {}', filePath: 'src/alpha.ts', startLine: 30, endLine: 40 }),
    makeChunk({ content: 'export const gamma = 1', filePath: 'src/gamma.ts', startLine: 1, endLine: 5 }),
    makeChunk({ content: 'function delta() {}', filePath: 'src/delta.ts', startLine: 5, endLine: 15 }),
    makeChunk({ content: 'class Epsilon {}', filePath: 'src/epsilon.ts', startLine: 50, endLine: 60 }),
  ]
}

function mockModel(): Model {
  return {
    encode: (texts: string[]) => texts.map(() => new Float32Array([0.1, 0.2, 0.3])),
    dim: 3,
  }
}

interface QueryCall {
  vectors: Float32Array[]
  k: number
  selector: Uint32Array | undefined
}

function mockSemanticIndex(
  results: Array<[number, number]>,
  capture?: { calls: QueryCall[] },
): SelectableBasicBackend {
  return {
    query: (vectors, k, selector) => {
      capture?.calls.push({ vectors, k, selector })
      return [results]
    },
  }
}

interface Bm25Call {
  tokens: string[]
  mask: Uint8Array | undefined
}

function mockBm25(scores: number[], capture?: { calls: Bm25Call[] }): Bm25Index {
  return {
    getScores: (tokens, mask) => {
      capture?.calls.push({ tokens, mask })
      return new Float32Array(scores)
    },
  }
}

// ---------- _sortTopK -------------------------------------------------------

describe('_sortTopK', () => {
  it('returns indices in descending score order', () => {
    const scores = new Float32Array([0.1, 0.9, 0.5, 0.3, 0.7])
    const out = _sortTopK(scores, 3)
    expect([...out]).toEqual([1, 4, 2])
  })

  it('clamps to array length when topK is larger', () => {
    const scores = new Float32Array([1, 2, 3])
    const out = _sortTopK(scores, 10)
    expect([...out]).toEqual([2, 1, 0])
  })

  it('returns empty Uint32Array on empty input', () => {
    const out = _sortTopK(new Float32Array(), 5)
    expect(out.length).toBe(0)
  })
})

// ---------- _rrfScores ------------------------------------------------------

describe('_rrfScores', () => {
  it('assigns 1/(RRF_K+rank) to chunks in descending raw-score order', () => {
    const chunks = makeChunks()
    const raw = new Map<Chunk, number>([
      [chunks[0]!, 0.1],
      [chunks[1]!, 0.9],
      [chunks[2]!, 0.5],
    ])
    const rrf = _rrfScores(raw)
    expect(rrf.get(chunks[1]!)).toBeCloseTo(1 / (RRF_K + 1), 10)
    expect(rrf.get(chunks[2]!)).toBeCloseTo(1 / (RRF_K + 2), 10)
    expect(rrf.get(chunks[0]!)).toBeCloseTo(1 / (RRF_K + 3), 10)
  })

  it('returns an empty map for empty input', () => {
    const out = _rrfScores(new Map())
    expect(out.size).toBe(0)
  })

  it('first-rank chunk gets ~0.01639 (1/61)', () => {
    const chunks = makeChunks()
    const raw = new Map<Chunk, number>([[chunks[0]!, 5.0]])
    const rrf = _rrfScores(raw)
    expect(rrf.get(chunks[0]!)).toBeCloseTo(1 / 61, 10)
  })
})

// ---------- _searchSemantic / _searchBm25 -----------------------------------

describe('_searchSemantic', () => {
  it('converts cosine distance to similarity (1 - distance)', () => {
    const chunks = makeChunks()
    const idx = mockSemanticIndex([[0, 0.2], [2, 0.7]])
    const results = _searchSemantic('q', mockModel(), idx, chunks, 5, undefined)
    expect(results.length).toBe(2)
    expect(results[0]!.chunk).toBe(chunks[0]!)
    expect(results[0]!.score).toBeCloseTo(0.8, 10)
    expect(results[1]!.chunk).toBe(chunks[2]!)
    expect(results[1]!.score).toBeCloseTo(0.3, 10)
  })

  it('passes the selector through to semanticIndex.query', () => {
    const chunks = makeChunks()
    const capture = { calls: [] as QueryCall[] }
    const idx = mockSemanticIndex([[0, 0.5]], capture)
    const selector = new Uint32Array([0, 2])
    _searchSemantic('q', mockModel(), idx, chunks, 5, selector)
    expect(capture.calls.length).toBe(1)
    expect(capture.calls[0]!.selector).toBe(selector)
    expect(capture.calls[0]!.k).toBe(5)
  })
})

describe('_searchBm25', () => {
  it('excludes zero-score chunks and returns top-k sorted', () => {
    const chunks = makeChunks()
    const bm = mockBm25([0.5, 0, 0.9, 0.2, 0])
    const results = _searchBm25('alpha beta', bm, chunks, 5, undefined)
    expect(results.map(r => r.chunk)).toEqual([chunks[2]!, chunks[0]!, chunks[3]!])
    expect(results[0]!.score).toBeCloseTo(0.9, 5)
  })

  it('returns [] when tokenize yields no tokens', () => {
    const chunks = makeChunks()
    const bm = mockBm25([1, 1, 1, 1, 1])
    const results = _searchBm25('   ', bm, chunks, 5, undefined)
    expect(results).toEqual([])
  })

  it('builds a boolean weight mask from the selector', () => {
    const chunks = makeChunks()
    const capture = { calls: [] as Bm25Call[] }
    const bm = mockBm25([1, 1, 1, 1, 1], capture)
    _searchBm25('alpha', bm, chunks, 5, new Uint32Array([1, 3]))
    expect(capture.calls.length).toBe(1)
    expect(Array.from(capture.calls[0]!.mask!)).toEqual([0, 1, 0, 1, 0])
  })
})

// ---------- search() --------------------------------------------------------

describe('search() — alpha blending', () => {
  it('with alpha=1.0 yields purely semantic ordering (BM25 contribution = 0)', () => {
    const chunks = makeChunks()
    // chunks[2] is first in semantic, chunks[0] is second; BM25 strongly favors chunks[4]
    // but alpha=1.0 must zero its contribution so it never outranks a semantic hit.
    const idx = mockSemanticIndex([[2, 0.05], [0, 0.10]])
    const bm = mockBm25([0, 0, 0, 0, 9.0])
    const results = search('alpha', mockModel(), idx, bm, chunks, 3, { alpha: 1.0, rerank: false })
    // The two semantic hits must come first; chunks[4] (BM25-only) ranks last with score 0.
    expect(results[0]!.chunk).toBe(chunks[2]!)
    expect(results[1]!.chunk).toBe(chunks[0]!)
    expect(results[0]!.score).toBeGreaterThan(0)
    expect(results[1]!.score).toBeGreaterThan(0)
    // chunks[4] is in the union but scored 0 under alpha=1.0.
    const ch4Result = results.find(r => r.chunk === chunks[4])
    if (ch4Result !== undefined)
      expect(ch4Result.score).toBe(0)
  })

  it('with alpha=0.0 yields purely BM25 ordering', () => {
    const chunks = makeChunks()
    const idx = mockSemanticIndex([[0, 0.05]])
    const bm = mockBm25([0.5, 0, 0.9, 0.2, 0])
    const results = search('alpha', mockModel(), idx, bm, chunks, 3, { alpha: 0.0, rerank: false })
    // BM25 top: chunks[2] (0.9), chunks[0] (0.5), chunks[3] (0.2)
    expect(results.map(r => r.chunk)).toEqual([chunks[2]!, chunks[0]!, chunks[3]!])
  })
})

describe('search() — RRF normalisation', () => {
  it('produces score 1/61 for a chunk that is rank-1 in semantic with alpha=1.0', () => {
    const chunks = makeChunks()
    const idx = mockSemanticIndex([[0, 0.0]]) // distance 0 → similarity 1
    const bm = mockBm25([0, 0, 0, 0, 0])
    const results = search('q', mockModel(), idx, bm, chunks, 5, { alpha: 1.0, rerank: false })
    expect(results.length).toBe(1)
    expect(results[0]!.score).toBeCloseTo(1 / 61, 10)
  })
})

describe('search() — sort stability', () => {
  it('iterates candidates in startLine order before scoring (counteracts hash nondeterminism)', () => {
    // Build a scenario where two chunks tie on combined RRF score and we want
    // the lower-startLine chunk to be produced first.
    const chunks = [
      makeChunk({ content: 'foo', filePath: 'src/late.ts', startLine: 100 }),
      makeChunk({ content: 'bar', filePath: 'src/early.ts', startLine: 1 }),
    ]
    // Both rank 1 in their respective lists → both get the same RRF score
    // → combined ties → ordering must come from startLine.
    const idx = mockSemanticIndex([[0, 0.5]]) // chunks[0] only in semantic
    const bm = mockBm25([0, 1.0]) // chunks[1] only in bm25
    const results = search('q', mockModel(), idx, bm, chunks, 5, { alpha: 0.5, rerank: false })
    expect(results.length).toBe(2)
    expect(results[0]!.chunk.startLine).toBe(1)
    expect(results[1]!.chunk.startLine).toBe(100)
  })
})

describe('search() — empty inputs', () => {
  it('returns [] when both backends yield nothing', () => {
    const chunks = makeChunks()
    const idx = mockSemanticIndex([])
    const bm = mockBm25(chunks.map(() => 0))
    const results = search('q', mockModel(), idx, bm, chunks, 5)
    expect(results).toEqual([])
  })
})

describe('search() — rerank pipeline', () => {
  it('rerank=true applies multi-chunk file boost (chunks[0] & chunks[1] share src/alpha.ts)', () => {
    const chunks = makeChunks()
    // Semantic puts both alpha.ts chunks high; gamma.ts is also present.
    const idx = mockSemanticIndex([[0, 0.10], [1, 0.20], [2, 0.30]])
    const bm = mockBm25([0, 0, 0, 0, 0])
    const ranked = search('q', mockModel(), idx, bm, chunks, 3, { alpha: 1.0, rerank: true })
    // With multi-chunk boost, the best chunk in alpha.ts should outrank
    // gamma.ts even though gamma.ts has a respectable RRF rank.
    expect(ranked[0]!.chunk.filePath).toBe('src/alpha.ts')
  })

  it('rerank=false skips boosts and just sorts by combined score', () => {
    const chunks = makeChunks()
    const idx = mockSemanticIndex([[0, 0.10], [1, 0.20], [2, 0.30]])
    const bm = mockBm25([0, 0, 0, 0, 0])
    const ranked = search('q', mockModel(), idx, bm, chunks, 3, { alpha: 1.0, rerank: false })
    expect(ranked.map(r => r.chunk)).toEqual([chunks[0]!, chunks[1]!, chunks[2]!])
  })
})

describe('search() — file-saturation decay', () => {
  it('demotes extra chunks from the same file after the first match', () => {
    // Three chunks of src/alpha.ts at semantic ranks 1/2/3, plus one from src/beta.ts at rank 4.
    // Saturation should push the 2nd & 3rd alpha.ts chunks below beta.ts in the final ordering.
    const chunks = [
      makeChunk({ content: '', filePath: 'src/alpha.ts', startLine: 10 }),
      makeChunk({ content: '', filePath: 'src/alpha.ts', startLine: 30 }),
      makeChunk({ content: '', filePath: 'src/alpha.ts', startLine: 50 }),
      makeChunk({ content: '', filePath: 'src/beta.ts', startLine: 1 }),
    ]
    const idx = mockSemanticIndex([
      [0, 0.10],
      [1, 0.20],
      [2, 0.30],
      [3, 0.40],
    ])
    const bm = mockBm25([0, 0, 0, 0])
    // alpha=1.0 → multi-chunk boost only touches the single best chunk per file.
    // file-saturation decay (0.5^excess) demotes the 2nd & 3rd alpha.ts chunks.
    const ranked = search('q', mockModel(), idx, bm, chunks, 4, { alpha: 1.0, rerank: true })
    // First slot: best alpha.ts chunk (boosted).
    expect(ranked[0]!.chunk.filePath).toBe('src/alpha.ts')
    // Second slot: beta.ts (no saturation penalty applied to a different file).
    expect(ranked[1]!.chunk.filePath).toBe('src/beta.ts')
  })
})

describe('search() — auto-alpha for symbol queries', () => {
  it('passes penalisePaths=true (alpha<1.0) for symbol-shaped queries by default', () => {
    // Indirect assertion: a symbol query has alpha=0.3, so BM25 contributes.
    // With alpha=0.3 and only a BM25 hit, the result must contain that hit.
    const chunks = makeChunks()
    const idx = mockSemanticIndex([])
    const bm = mockBm25([0, 0, 0.9, 0, 0])
    const ranked = search('FooBar', mockModel(), idx, bm, chunks, 3, { rerank: false })
    expect(ranked.length).toBe(1)
    expect(ranked[0]!.chunk).toBe(chunks[2]!)
  })
})
