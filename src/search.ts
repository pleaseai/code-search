// Port of src/semble/search.py

import type { Chunk, SearchResult } from './types.ts'
import { tokenize } from './tokens.ts'

// Re-export the shared types so downstream importers (and tests) can keep
// pulling `Chunk`/`SearchResult` from this module's public surface.
export type { Chunk, SearchResult }

/**
 * Render a chunk as a JSONable object (snake_cased fields + `location`),
 * mirroring semble's `Chunk.to_dict`.
 */
function chunkToDict(chunk: Chunk): Record<string, unknown> {
  return {
    content: chunk.content,
    file_path: chunk.filePath,
    start_line: chunk.startLine,
    end_line: chunk.endLine,
    language: chunk.language ?? null,
    location: `${chunk.filePath}:${chunk.startLine}-${chunk.endLine}`,
  }
}

/**
 * Build a `SearchResult` with a `toDict` closure, so every result this module
 * produces satisfies the `../types.ts` `SearchResult` contract that
 * `utils.formatResults` consumes.
 */
function makeResult(chunk: Chunk, score: number): SearchResult {
  return {
    chunk,
    score,
    toDict: () => ({ chunk: chunkToDict(chunk), score }),
  }
}

// TODO(integration): replace with import from './ranking/weighting.ts'
const _ALPHA_SYMBOL = 0.3
const _ALPHA_NL = 0.5
const _SYMBOL_QUERY_RE = /^(?:[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Za-z_][A-Za-z0-9_]*)+|_[A-Za-z0-9_]*|[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*|[A-Z][A-Za-z0-9]*)$/
function isSymbolQuery(query: string): boolean {
  return _SYMBOL_QUERY_RE.test(query.trim())
}
function resolveAlpha(query: string, alpha: number | undefined): number {
  if (alpha !== undefined)
    return alpha
  return isSymbolQuery(query) ? _ALPHA_SYMBOL : _ALPHA_NL
}

// TODO(integration): replace with import from './ranking/boosting.ts'
function boostMultiChunkFiles(scores: Map<Chunk, number>): void {
  if (scores.size === 0)
    return
  let maxScore = -Infinity
  for (const v of scores.values()) {
    if (v > maxScore)
      maxScore = v
  }
  if (maxScore === 0)
    return
  const fileSum = new Map<string, number>()
  const bestChunk = new Map<string, Chunk>()
  for (const [chunk, score] of scores) {
    fileSum.set(chunk.filePath, (fileSum.get(chunk.filePath) ?? 0) + score)
    const existing = bestChunk.get(chunk.filePath)
    if (existing === undefined || score > (scores.get(existing) ?? -Infinity))
      bestChunk.set(chunk.filePath, chunk)
  }
  let maxFileSum = -Infinity
  for (const v of fileSum.values()) {
    if (v > maxFileSum)
      maxFileSum = v
  }
  const boostUnit = maxScore * 0.2
  for (const [filePath, chunk] of bestChunk) {
    const sum = fileSum.get(filePath) ?? 0
    scores.set(chunk, (scores.get(chunk) ?? 0) + (boostUnit * sum) / maxFileSum)
  }
}

// TODO(integration): replace with import from './ranking/boosting.ts'
function applyQueryBoost(
  combinedScores: Map<Chunk, number>,
  _query: string,
  _allChunks: Chunk[],
): Map<Chunk, number> {
  // Minimal stub — preserves identity. Full implementation arrives with ranking/boosting.ts.
  return new Map(combinedScores)
}

// TODO(integration): replace with import from './ranking/penalties.ts'
function rerankTopK(
  scores: Map<Chunk, number>,
  topK: number,
  options: { penalisePaths: boolean } = { penalisePaths: true },
): Array<[Chunk, number]> {
  // Minimal stub mirroring the Python file-saturation logic without path penalties.
  void options
  if (scores.size === 0)
    return []
  const ranked = [...scores.entries()].sort((a, b) => b[1] - a[1])
  const FILE_SATURATION_THRESHOLD = 1
  const FILE_SATURATION_DECAY = 0.5
  const fileSelected = new Map<string, number>()
  const selected: Array<[number, Chunk]> = []
  let minSelected = Number.POSITIVE_INFINITY

  for (const [chunk, penScore] of ranked) {
    if (selected.length >= topK && penScore <= minSelected)
      break
    const alreadySelected = fileSelected.get(chunk.filePath) ?? 0
    let effScore = penScore
    if (alreadySelected >= FILE_SATURATION_THRESHOLD) {
      const excess = alreadySelected - FILE_SATURATION_THRESHOLD + 1
      effScore *= FILE_SATURATION_DECAY ** excess
    }
    selected.push([effScore, chunk])
    fileSelected.set(chunk.filePath, alreadySelected + 1)
    if (selected.length >= topK) {
      minSelected = Number.POSITIVE_INFINITY
      for (const [s] of selected) {
        if (s < minSelected)
          minSelected = s
      }
    }
  }
  selected.sort((a, b) => b[0] - a[0])
  return selected.slice(0, topK).map(([score, chunk]) => [chunk, score])
}

// --- Public exports ---------------------------------------------------------

export const RRF_K = 60

/** Minimal embedding model interface (parallels `model2vec.StaticModel`). */
export interface Model {
  encode: (texts: string[]) => Float32Array[]
  dim: number
}

/**
 * Minimal vector backend interface (parallels `vicinity` CosineBasicBackend).
 *
 * `query` returns one result list per query vector — each list is a sequence
 * of `[chunkIndex, cosineDistance]` pairs sorted by ascending distance.
 */
export interface SelectableBasicBackend {
  query: (
    vectors: Float32Array[],
    k: number,
    selector?: Uint32Array,
  ) => Array<Array<[number, number]>>
}

/** Minimal BM25 backend interface (parallels `bm25s.BM25`). */
export interface Bm25Index {
  getScores: (queryTokens: string[], weightMask?: Uint8Array) => Float32Array
}

/** Build a boolean weight mask from a chunk-index selector, or `undefined` if no selector. */
function selectorToMask(selector: Uint32Array | undefined, size: number): Uint8Array | undefined {
  if (selector === undefined)
    return undefined
  const mask = new Uint8Array(size)
  for (const idx of selector) {
    if (idx < size)
      mask[idx] = 1
  }
  return mask
}

/**
 * Convert raw scores to RRF scores `1 / (RRF_K + rank)`; highest raw score → rank 1.
 *
 * Ties in the raw scores are broken by insertion order (the underlying sort is stable).
 */
export function _rrfScores(scores: Map<Chunk, number>): Map<Chunk, number> {
  if (scores.size === 0)
    return scores
  const ranked = [...scores.entries()].sort((a, b) => b[1] - a[1])
  const out = new Map<Chunk, number>()
  for (let i = 0; i < ranked.length; i++) {
    const entry = ranked[i]
    if (entry === undefined)
      continue
    const rank = i + 1
    out.set(entry[0], 1.0 / (RRF_K + rank))
  }
  return out
}

/** Partial sort: return indices of the top-k largest entries of `arr`, in descending-score order. */
export function _sortTopK(arr: Float32Array, topK: number): Uint32Array {
  const n = arr.length
  const indices = new Array<number>(n)
  for (let i = 0; i < n; i++)
    indices[i] = i
  indices.sort((a, b) => {
    const av = arr[a] as number
    const bv = arr[b] as number
    return bv - av
  })
  const k = Math.min(topK, n)
  const out = new Uint32Array(k)
  for (let i = 0; i < k; i++)
    out[i] = indices[i] as number
  return out
}

/** Run semantic search for a query. Converts cosine distance → similarity (`1 - distance`). */
export function _searchSemantic(
  query: string,
  model: Model,
  semanticIndex: SelectableBasicBackend,
  chunks: Chunk[],
  topK: number,
  selector: Uint32Array | undefined,
): SearchResult[] {
  const queryEmbedding = model.encode([query])
  const batch = semanticIndex.query(queryEmbedding, topK, selector)
  const first = batch[0]
  if (first === undefined)
    return []
  const results: SearchResult[] = []
  for (const [index, distance] of first) {
    const chunk = chunks[index]
    if (chunk === undefined)
      continue
    results.push(makeResult(chunk, 1.0 - distance))
  }
  return results
}

/** Return chunks ranked by BM25 score, excluding zero-score results. */
export function _searchBm25(
  query: string,
  bm25Index: Bm25Index,
  chunks: Chunk[],
  topK: number,
  selector: Uint32Array | undefined,
): SearchResult[] {
  const tokens = tokenize(query)
  if (tokens.length === 0)
    return []
  const mask = selectorToMask(selector, chunks.length)
  const scores = bm25Index.getScores(tokens, mask)
  const indices = _sortTopK(scores, topK)
  const results: SearchResult[] = []
  for (const i of indices) {
    const score = scores[i]
    if (score === undefined || score <= 0)
      continue
    const chunk = chunks[i]
    if (chunk === undefined)
      continue
    results.push(makeResult(chunk, score))
  }
  return results
}

export interface SearchOptions {
  /** Weight for semantic score (1-alpha for BM25). `undefined` → auto-detect by query type. */
  alpha?: number
  /** Optional chunk-index selector to filter candidates. */
  selector?: Uint32Array
  /** Whether to apply code-tuned reranking (path penalties, file saturation, boosts). */
  rerank?: boolean
}

/**
 * Hybrid search: alpha-weighted combination of semantic and BM25 scores.
 *
 * Both score sets are converted to RRF scores before combining, so `alpha` has a
 * consistent meaning regardless of raw-score magnitude.
 */
export function search(
  query: string,
  model: Model,
  semanticIndex: SelectableBasicBackend,
  bm25Index: Bm25Index,
  chunks: Chunk[],
  topK: number,
  options: SearchOptions = {},
): SearchResult[] {
  const { alpha, selector, rerank = true } = options
  const alphaWeight = resolveAlpha(query, alpha)

  // Over-fetch candidates so the merged pool is large enough after union & re-ranking.
  const candidateCount = topK * 5

  const semantic = _searchSemantic(query, model, semanticIndex, chunks, candidateCount, selector)
  const semanticScores = new Map<Chunk, number>()
  for (const r of semantic)
    semanticScores.set(r.chunk, r.score)

  const bm25Scores = new Map<Chunk, number>()
  for (const r of _searchBm25(query, bm25Index, chunks, candidateCount, selector)) {
    if (r.score)
      bm25Scores.set(r.chunk, r.score)
  }

  const normalizedSemantic = _rrfScores(semanticScores)
  const normalizedBm25 = _rrfScores(bm25Scores)

  // Sort the union by start_line to counteract hash-iteration nondeterminism.
  const union = new Set<Chunk>([...normalizedSemantic.keys(), ...normalizedBm25.keys()])
  const allCandidates = [...union].sort((a, b) => a.startLine - b.startLine)

  let combinedScores = new Map<Chunk, number>()
  for (const chunk of allCandidates) {
    const s = normalizedSemantic.get(chunk) ?? 0
    const b = normalizedBm25.get(chunk) ?? 0
    combinedScores.set(chunk, alphaWeight * s + (1.0 - alphaWeight) * b)
  }

  let ranked: Array<[Chunk, number]>
  if (rerank) {
    boostMultiChunkFiles(combinedScores)
    combinedScores = applyQueryBoost(combinedScores, query, chunks)
    ranked = rerankTopK(combinedScores, topK, { penalisePaths: alphaWeight < 1.0 })
  }
  else {
    ranked = [...combinedScores.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, topK)
  }

  return ranked.map(([chunk, score]) => makeResult(chunk, score))
}
