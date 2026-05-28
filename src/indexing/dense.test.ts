// Port of src/semble/index/dense.py — unit tests

import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  DEFAULT_MODEL_NAME,
  embedChunks,
  loadModel,
  SelectableBasicBackend,
  type Chunk,
} from './dense'

function chunk(content: string): Chunk {
  return {
    content,
    filePath: 'a.ts',
    startLine: 1,
    endLine: 1,
    language: 'typescript',
  }
}

describe('loadModel', () => {
  it('resolves with a Model exposing a positive dim', async () => {
    const { model, modelPath } = await loadModel()
    expect(modelPath).toBe(DEFAULT_MODEL_NAME)
    expect(model.dim).toBeGreaterThan(0)
  })

  it('caches models by path', async () => {
    const a = await loadModel('test/path-A')
    const b = await loadModel('test/path-A')
    expect(a.model).toBe(b.model)
  })

  it('returns distinct entries for different paths', async () => {
    const a = await loadModel('test/path-X')
    const b = await loadModel('test/path-Y')
    expect(a.modelPath).toBe('test/path-X')
    expect(b.modelPath).toBe('test/path-Y')
  })
})

describe('embedChunks', () => {
  it('returns [] for an empty input', async () => {
    const { model } = await loadModel()
    expect(embedChunks(model, [])).toEqual([])
  })

  it('returns one vector per chunk with model.dim length', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('hello'), chunk('world')])
    expect(vectors).toHaveLength(2)
    for (const v of vectors) {
      expect(v).toBeInstanceOf(Float32Array)
      expect(v.length).toBe(model.dim)
    }
  })

  it('is deterministic: same content → same vector', async () => {
    const { model } = await loadModel()
    const [v1] = embedChunks(model, [chunk('def search()')])
    const [v2] = embedChunks(model, [chunk('def search()')])
    expect(v1).toBeDefined()
    expect(v2).toBeDefined()
    expect(Array.from(v1!)).toEqual(Array.from(v2!))
  })

  it('produces different vectors for different content', async () => {
    const { model } = await loadModel()
    const [v1, v2] = embedChunks(model, [chunk('foo'), chunk('bar')])
    expect(v1).toBeDefined()
    expect(v2).toBeDefined()
    expect(Array.from(v1!)).not.toEqual(Array.from(v2!))
  })
})

describe('SelectableBasicBackend.query', () => {
  it('throws when k < 1', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b')])
    const backend = new SelectableBasicBackend(vectors)
    expect(() => backend.query([vectors[0]!], 0)).toThrow()
  })

  it('throws when constructed with inconsistent vector dimensions', async () => {
    const { model } = await loadModel()
    const [v0] = embedChunks(model, [chunk('a')])
    const truncated = new Float32Array(v0!.length - 1)
    expect(() => new SelectableBasicBackend([v0!, truncated])).toThrow(
      /Inconsistent vector dimensions/,
    )
  })

  it('throws when a query vector dimension differs from the index dim', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b')])
    const backend = new SelectableBasicBackend(vectors)
    const bad = new Float32Array(backend.dim - 1)
    expect(() => backend.query([bad], 1)).toThrow(/Query vector dimension mismatch/)
  })

  it('throws when a selector index is out of bounds', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b')])
    const backend = new SelectableBasicBackend(vectors)
    const selector = new Uint32Array([0, 5])
    expect(() => backend.query([vectors[0]!], 1, selector)).toThrow(
      /Selector index out of bounds/,
    )
  })

  it('returns top-k (index, distance) pairs sorted by distance', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b'), chunk('c'), chunk('d')])
    const backend = new SelectableBasicBackend(vectors)

    const results = backend.query([vectors[0]!], 3)
    expect(results).toHaveLength(1)
    const hits = results[0]!
    expect(hits).toHaveLength(3)
    // Self should be the nearest with ~0 distance.
    expect(hits[0]![0]).toBe(0)
    expect(hits[0]![1]).toBeCloseTo(0, 5)
    // Distances must be monotonically non-decreasing.
    for (let i = 1; i < hits.length; i++) {
      expect(hits[i]![1]).toBeGreaterThanOrEqual(hits[i - 1]![1])
    }
  })

  it('only returns indices from the selector pool', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b'), chunk('c'), chunk('d')])
    const backend = new SelectableBasicBackend(vectors)

    const selector = new Uint32Array([1, 2])
    const results = backend.query([vectors[0]!], 5, selector)
    expect(results).toHaveLength(1)
    const hits = results[0]!
    // effective_k = min(5, 4, 2) = 2.
    expect(hits).toHaveLength(2)
    const indices = hits.map(h => h[0])
    for (const i of indices) {
      expect([1, 2]).toContain(i)
    }
  })

  it('handles multiple query vectors', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b'), chunk('c')])
    const backend = new SelectableBasicBackend(vectors)

    const results = backend.query([vectors[0]!, vectors[1]!], 2)
    expect(results).toHaveLength(2)
    expect(results[0]![0]![0]).toBe(0)
    expect(results[1]![0]![0]).toBe(1)
  })

  it('caps effective_k at the number of stored vectors', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('a'), chunk('b')])
    const backend = new SelectableBasicBackend(vectors)
    const results = backend.query([vectors[0]!], 10)
    expect(results[0]!).toHaveLength(2)
  })
})

describe('SelectableBasicBackend save/load', () => {
  let dir: string
  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'csp-dense-'))
  })
  afterEach(async () => {
    await rm(dir, { recursive: true, force: true })
  })

  it('roundtrip preserves vectors and query results', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('alpha'), chunk('beta'), chunk('gamma')])
    const original = new SelectableBasicBackend(vectors)
    await original.save(dir)

    const loaded = await SelectableBasicBackend.load(dir)
    expect(loaded.vectors).toHaveLength(original.vectors.length)
    expect(loaded.dim).toBe(original.dim)

    for (let i = 0; i < original.vectors.length; i++) {
      const a = original.vectors[i]!
      const b = loaded.vectors[i]!
      expect(b.length).toBe(a.length)
      for (let j = 0; j < a.length; j++) {
        expect(b[j]!).toBeCloseTo(a[j]!, 6)
      }
    }

    const origResults = original.query([vectors[0]!], 2)
    const loadedResults = loaded.query([vectors[0]!], 2)
    expect(loadedResults[0]!.map(h => h[0])).toEqual(origResults[0]!.map(h => h[0]))
  })

  it('rejects a truncated vectors.bin during load', async () => {
    const { model } = await loadModel()
    const vectors = embedChunks(model, [chunk('alpha'), chunk('beta')])
    const original = new SelectableBasicBackend(vectors)
    await original.save(dir)

    // Truncate vectors.bin to half its expected size.
    const truncated = new Float32Array(original.dim) // one row instead of two
    await writeFile(
      join(dir, 'vectors.bin'),
      Buffer.from(truncated.buffer, truncated.byteOffset, truncated.byteLength),
    )

    await expect(SelectableBasicBackend.load(dir)).rejects.toThrow(
      /Vector file size mismatch/,
    )
  })
})
