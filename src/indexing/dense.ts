// Port of src/semble/index/dense.py
//
// Loads a Model2Vec model, embeds chunks, and provides a vector
// backend with cosine distance + optional index-selector filtering.
//
// NOTE: This unit ships a STUB Model2Vec implementation. `loadModel` and
// `embedChunks` do not download or run a real Model2Vec model. Instead
// they produce deterministic, hash-seeded float vectors so that the API
// contract is exercised by tests without requiring network I/O.
// TODO(dense): integrate real Model2Vec model loading.

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

/**
 * Default Model2Vec model name (kept identical to semble for parity).
 */
export const DEFAULT_MODEL_NAME = 'minishlab/potion-code-16M'

/**
 * Default embedding dimension for the stub model. The real
 * `potion-code-16M` model emits 256-dim vectors, but the stub is
 * dimension-agnostic — pick something small enough for fast tests.
 */
const _DEFAULT_STUB_DIM = 256

/**
 * Minimal chunk shape this module consumes. We only need `content`,
 * so this is inlined rather than imported from a (not-yet-existing)
 * top-level `types.ts`. When `src/types.ts` lands, swap this for
 *   `import type { Chunk } from '../types.ts'`.
 */
export interface Chunk {
  content: string
  // Other fields (filePath, startLine, endLine, language) are unused
  // here but allowed via the index signature so callers can pass full
  // Chunk objects without type narrowing.
  [key: string]: unknown
}

/**
 * Loaded Model2Vec model. The real model exposes `.encode(texts)`;
 * the stub provides the same shape plus a `dim` accessor.
 */
export interface Model {
  readonly dim: number
  encode: (texts: string[]) => Float32Array[]
}

const _MODEL_CACHE = new Map<string, Model>()

/**
 * Deterministic 32-bit hash (FNV-1a) for stub seeding.
 */
function fnv1a(s: string): number {
  let h = 0x811C9DC5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193) >>> 0
  }
  return h >>> 0
}

/**
 * Mulberry32 PRNG — fast, deterministic, good enough for stub vectors.
 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6D2B79F5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/**
 * Build a deterministic unit-length vector from a string. Identical
 * input strings always produce identical vectors, satisfying the
 * "embedding is a pure function of content" contract.
 */
function stubEmbed(text: string, dim: number): Float32Array {
  const rng = mulberry32(fnv1a(text))
  const v = new Float32Array(dim)
  let norm = 0
  for (let i = 0; i < dim; i++) {
    // Box-Muller-ish: cheap normal-ish distribution out of two uniforms.
    const u1 = Math.max(rng(), 1e-12)
    const u2 = rng()
    const g = Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2)
    v[i] = g
    norm += g * g
  }
  norm = Math.sqrt(norm) || 1
  for (let i = 0; i < dim; i++) v[i] = v[i]! / norm
  return v
}

function makeStubModel(dim: number): Model {
  return {
    dim,
    encode(texts: string[]): Float32Array[] {
      return texts.map(t => stubEmbed(t, dim))
    },
  }
}

/**
 * Load (and cache) a Model2Vec model. Always async, mirroring the
 * eventual real implementation that performs an HF download.
 *
 * @param modelPath Optional model id; defaults to {@link DEFAULT_MODEL_NAME}.
 */
export async function loadModel(
  modelPath?: string,
): Promise<{ model: Model, modelPath: string }> {
  const resolved = modelPath ?? DEFAULT_MODEL_NAME
  let model = _MODEL_CACHE.get(resolved)
  if (!model) {
    // TODO(dense): replace with real Model2Vec download + inference.
    model = makeStubModel(_DEFAULT_STUB_DIM)
    _MODEL_CACHE.set(resolved, model)
  }
  return Promise.resolve({ model, modelPath: resolved })
}

/**
 * Embed chunks using the configured model. Returns one row per chunk;
 * the empty list maps to an empty result (matching semble).
 */
export function embedChunks(model: Model, chunks: Chunk[]): Float32Array[] {
  if (chunks.length === 0) return []
  return model.encode(chunks.map(c => c.content))
}

// ---------------------------------------------------------------------------
// SelectableBasicBackend
// ---------------------------------------------------------------------------

export interface BasicArgs {
  /** Distance metric — for parity we only support cosine. */
  metric?: 'cosine'
}

/**
 * Pre-normalise a vector in place (L2). Zero vectors stay zero.
 */
function normalizeInPlace(v: Float32Array): void {
  let n = 0
  for (let i = 0; i < v.length; i++) n += v[i]! * v[i]!
  n = Math.sqrt(n)
  if (n === 0) return
  for (let i = 0; i < v.length; i++) v[i] = v[i]! / n
}

function dot(a: Float32Array, b: Float32Array): number {
  let s = 0
  const n = a.length
  for (let i = 0; i < n; i++) s += a[i]! * b[i]!
  return s
}

/**
 * In-memory vector backend with cosine distance and optional
 * candidate-selector filtering — TS port of
 * `SelectableBasicBackend(CosineBasicBackend)` from semble.
 */
export class SelectableBasicBackend {
  /** Pre-normalised row vectors. */
  readonly vectors: Float32Array[]
  readonly arguments: BasicArgs
  readonly dim: number

  constructor(vectors: Float32Array[], options: BasicArgs = {}) {
    this.arguments = { metric: 'cosine', ...options }
    this.dim = vectors[0]?.length ?? 0
    // Defensive copy + normalise so cosine distance reduces to (1 - dot).
    this.vectors = vectors.map((v) => {
      if (v.length !== this.dim) {
        throw new Error(
          `Inconsistent vector dimensions: expected ${this.dim}, got ${v.length}`,
        )
      }
      const copy = new Float32Array(v)
      normalizeInPlace(copy)
      return copy
    })
  }

  /**
   * Batched k-NN query.
   *
   * @param queryVectors One row per query (raw — will be normalised here).
   * @param k Number of neighbours per query.
   * @param selector Optional pool of candidate indices; results are
   *   guaranteed to come from this set.
   * @returns For each query, an array of `[chunkIndex, cosineDistance]`
   *   sorted by ascending distance.
   * @throws Error if `k < 1`.
   */
  query(
    queryVectors: Float32Array[],
    k: number,
    selector?: Uint32Array,
  ): Array<Array<[number, number]>> {
    if (k < 1) throw new Error(`k should be >= 1, is now ${k}`)

    const numVectors = this.vectors.length
    let effectiveK = Math.min(k, numVectors)
    if (selector !== undefined) {
      // Bounds-check selector indices up front so we fail fast with a
      // descriptive error instead of crashing during the dot-product loop.
      for (let i = 0; i < selector.length; i++) {
        const idx = selector[i]!
        if (idx >= numVectors) {
          throw new Error(
            `Selector index out of bounds: ${idx} (total vectors: ${numVectors})`,
          )
        }
      }
      effectiveK = Math.min(effectiveK, selector.length)
    }

    const out: Array<Array<[number, number]>> = []
    if (effectiveK === 0) {
      for (let i = 0; i < queryVectors.length; i++) out.push([])
      return out
    }

    for (const raw of queryVectors) {
      if (raw.length !== this.dim) {
        throw new Error(
          `Query vector dimension mismatch: expected ${this.dim}, got ${raw.length}`,
        )
      }
      const q = new Float32Array(raw)
      normalizeInPlace(q)

      const candidatePool = selector ?? null
      const poolSize = candidatePool ? candidatePool.length : numVectors
      const distances = new Float64Array(poolSize)
      for (let i = 0; i < poolSize; i++) {
        const vecIdx = candidatePool ? candidatePool[i]! : i
        const target = this.vectors[vecIdx]!
        distances[i] = 1 - dot(q, target)
      }

      // Build [poolIdx, dist] pairs and partial-sort by distance.
      const pairs: Array<[number, number]> = Array.from(
        { length: poolSize },
        (_, i) => [i, distances[i]!],
      )
      pairs.sort((a, b) => a[1] - b[1])
      const top = pairs.slice(0, effectiveK)

      // Map pool-relative indices back to absolute chunk indices.
      const mapped: Array<[number, number]> = top.map(([poolIdx, dist]) => [
        candidatePool ? candidatePool[poolIdx]! : poolIdx,
        dist,
      ])
      out.push(mapped)
    }

    return out
  }

  /**
   * Persist vectors + args to `<dir>/vectors.bin` and `<dir>/args.json`.
   * Format is local to csp — vicinity's own format is not preserved.
   */
  async save(dir: string): Promise<void> {
    await mkdir(dir, { recursive: true })
    const rows = this.vectors.length
    const dim = this.dim
    const buf = new Float32Array(rows * dim)
    for (let r = 0; r < rows; r++) buf.set(this.vectors[r]!, r * dim)
    const meta = { rows, dim, arguments: this.arguments }
    await writeFile(join(dir, 'vectors.bin'), Buffer.from(buf.buffer, buf.byteOffset, buf.byteLength))
    await writeFile(join(dir, 'args.json'), JSON.stringify(meta))
  }

  /**
   * Inverse of {@link SelectableBasicBackend.save}.
   */
  static async load(dir: string): Promise<SelectableBasicBackend> {
    const metaRaw = await readFile(join(dir, 'args.json'), 'utf8')
    const meta = JSON.parse(metaRaw) as { rows: number, dim: number, arguments: BasicArgs }
    const bytes = await readFile(join(dir, 'vectors.bin'))
    const expectedBytes = meta.rows * meta.dim * 4
    if (bytes.byteLength !== expectedBytes) {
      throw new Error(
        `Vector file size mismatch: expected ${expectedBytes} bytes, got ${bytes.byteLength}`,
      )
    }
    // Copy into a fresh ArrayBuffer so alignment is guaranteed.
    const ab = new ArrayBuffer(bytes.byteLength)
    new Uint8Array(ab).set(bytes)
    const flat = new Float32Array(ab)
    const vectors: Float32Array[] = []
    for (let r = 0; r < meta.rows; r++) {
      vectors.push(flat.slice(r * meta.dim, (r + 1) * meta.dim))
    }
    return new SelectableBasicBackend(vectors, meta.arguments)
  }
}
