// Tests for src/indexing/index.ts (CspIndex)

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { Chunk } from '../types.ts'
import { ContentType } from '../types.ts'
import { CspIndex, DEFAULT_CONTENT } from './index.ts'
import { SelectableBasicBackend, makeStubModel } from './dense.ts'
import { Bm25Index } from './sparse.ts'

function makeChunk(
  filePath: string,
  startLine: number,
  endLine: number,
  language: string | null = 'typescript',
  content?: string,
): Chunk {
  return {
    content: content ?? `// chunk for ${filePath}:${startLine}-${endLine}`,
    filePath,
    startLine,
    endLine,
    language,
  }
}

function buildIndex(chunks: Chunk[]): CspIndex {
  const model = makeStubModel('test-model', 4)
  const vectors = chunks.map((_, i) => {
    const v = new Float32Array(4)
    v[0] = i + 1
    return v
  })
  return new CspIndex({
    model,
    bm25Index: new Bm25Index(chunks.map(() => ['x'])),
    semanticIndex: new SelectableBasicBackend(vectors, 4),
    chunks,
    modelPath: 'test-model',
    root: null,
    content: DEFAULT_CONTENT,
  })
}

describe('CspIndex.stats', () => {
  it('returns zeros for an empty index', () => {
    const idx = buildIndex([])
    expect(idx.stats).toEqual({
      indexedFiles: 0,
      totalChunks: 0,
      languages: {},
    })
  })

  it('reflects chunk count, file count, and language distribution', () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript'),
      makeChunk('a.ts', 11, 20, 'typescript'),
      makeChunk('b.py', 1, 5, 'python'),
      makeChunk('c.bin', 1, 1, null),
    ]
    const idx = buildIndex(chunks)
    expect(idx.stats).toEqual({
      indexedFiles: 3,
      totalChunks: 4,
      languages: { typescript: 2, python: 1 },
    })
  })
})

describe('CspIndex.search', () => {
  it('returns [] on an empty query', () => {
    const chunks = [makeChunk('a.ts', 1, 1)]
    const idx = buildIndex(chunks)
    expect(idx.search('')).toEqual([])
    expect(idx.search('   ')).toEqual([])
  })

  it('returns [] when the index has no chunks', () => {
    const idx = buildIndex([])
    expect(idx.search('anything')).toEqual([])
  })
})

describe('CspIndex.findRelated', () => {
  it('excludes the source chunk from results', () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'seed chunk'),
      makeChunk('a.ts', 11, 20, 'typescript', 'companion 1'),
      makeChunk('b.ts', 1, 5, 'typescript', 'companion 2'),
    ]
    const idx = buildIndex(chunks)
    const seed = chunks[0]!
    const results = idx.findRelated(seed, { topK: 5 })
    // Source chunk must not appear in the results.
    expect(results.find(r => r.chunk === seed)).toBeUndefined()
    expect(results.length).toBeLessThanOrEqual(5)
  })

  it('accepts a SearchResult as the seed', () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'seed'),
      makeChunk('b.ts', 1, 5, 'typescript', 'other'),
    ]
    const idx = buildIndex(chunks)
    const results = idx.findRelated({ chunk: chunks[0]!, score: 0.5 })
    expect(results.find(r => r.chunk === chunks[0]!)).toBeUndefined()
  })
})

describe('CspIndex save → loadFromDisk roundtrip', () => {
  let dir: string

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'csp-roundtrip-'))
  })
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  it('persists chunks, indexes, and metadata', async () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'A'),
      makeChunk('b.ts', 1, 5, 'python', 'B'),
    ]
    const idx = buildIndex(chunks)
    await idx.save(dir)
    const loaded = await CspIndex.loadFromDisk(dir)
    expect(loaded.chunks.length).toBe(2)
    expect(loaded.chunks.map(c => c.filePath)).toEqual(['a.ts', 'b.ts'])
    expect(loaded.stats.totalChunks).toBe(2)
    expect(loaded.stats.languages).toEqual({ typescript: 1, python: 1 })
  })

  it('loadFromDisk throws on a missing directory', async () => {
    await expect(CspIndex.loadFromDisk(join(dir, 'nope'))).rejects.toThrow(
      /Index not found/,
    )
  })

  it('loadFromDisk throws when a persisted artifact is missing', async () => {
    // Dir exists but is empty.
    await expect(CspIndex.loadFromDisk(dir)).rejects.toThrow(/Missing:/)
  })
})

describe('CspIndex.fromPath', () => {
  let dir: string

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'csp-from-path-'))
  })
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  it('throws when the path does not exist', async () => {
    await expect(CspIndex.fromPath(join(dir, 'nope'))).rejects.toThrow(
      /Path does not exist/,
    )
  })

  it('throws when the path exists but is a file', async () => {
    const filePath = join(dir, 'a.ts')
    writeFileSync(filePath, '// hello\n')
    await expect(CspIndex.fromPath(filePath)).rejects.toThrow(
      /not a directory/,
    )
  })

  it('builds a CspIndex from a real directory with a small TS file', async () => {
    writeFileSync(
      join(dir, 'sample.ts'),
      'export function greet(name: string) {\n  return `hi ${name}`\n}\n',
    )
    const idx = await CspIndex.fromPath(dir, { content: ContentType.Code })
    expect(idx.stats.totalChunks).toBeGreaterThan(0)
    expect(idx.stats.indexedFiles).toBe(1)
    expect(idx.chunks[0]!.filePath).toBe('sample.ts')
  })
})
