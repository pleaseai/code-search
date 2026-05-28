// Port of src/semble/index/sparse.py
//
// Implements the two helpers from the upstream module plus a minimal BM25
// index (Bm25Index) that stands in for Python's `bm25s` library.
//
// BM25 backend choice (see PR body for full discussion):
//   Option B (inline minimal BM25+ with k1=1.5, b=0.75) was chosen over a
//   third-party npm such as wink-bm25-text-search because:
//     - The dependency tree stays self-contained while the project is still
//       a scaffold (no other indexing deps are pinned yet).
//     - The required surface is tiny (build / getScores / save / load) and
//       getScores must respect a weight_mask that maps cleanly to BM25's
//       per-document scoring loop.
//     - Replacing this backend later is a localized change because all
//       callers go through the Bm25Index class.

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

// Stopgap structural type until ./types.ts lands from Unit 1.
// Mirrors semble.types.Chunk with camelCase field names per
// @pleaseai/csp public-API conventions.
export interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string | null
}

/**
 * Append file path components to BM25 content to boost path-based queries.
 *
 * Assumes `chunk.filePath` is already repo-relative (set during indexing) so
 * machine-specific directory components are never indexed. The stem is
 * repeated twice to up-weight file-path matches in BM25.
 */
export function enrichForBm25(chunk: Chunk): string {
  const parsed = path.parse(chunk.filePath)
  const stem = parsed.name
  const dirParts = parsed.dir
    .split(/[/\\]/)
    .filter(part => part !== '' && part !== '.' && part !== '/')
  const dirText = dirParts.slice(-3).join(' ')
  return `${chunk.content} ${stem} ${stem} ${dirText}`
}

/**
 * Convert a selector array of indices into a boolean mask of length `size`.
 *
 * Returns `null` when `selector` is null/undefined so callers can skip mask
 * application entirely (matching the upstream semantics).
 */
export function selectorToMask(
  selector: Uint32Array | null | undefined,
  size: number,
): Uint8Array | null {
  if (selector === null || selector === undefined)
    return null
  const mask = new Uint8Array(size)
  for (const idx of selector) {
    if (idx < size)
      mask[idx] = 1
  }
  return mask
}

// ---------------------------------------------------------------------------
// Minimal BM25 index
// ---------------------------------------------------------------------------

// Standard Okapi BM25 hyperparameters used by bm25s' default Lucene scorer.
const K1 = 1.5
const B = 0.75

interface Bm25State {
  // Number of documents indexed.
  numDocs: number
  // Document length (token count) per document, in doc order.
  docLengths: Float32Array
  // Average document length across the corpus.
  avgDocLength: number
  // Term -> array of [docId, termFreq] entries (postings list).
  postings: Map<string, Array<[number, number]>>
  // Term -> document frequency (count of docs containing the term).
  docFreq: Map<string, number>
}

/**
 * Minimal BM25 index supporting build / getScores / save / load.
 *
 * Documents are passed pre-tokenized (callers use `tokenize(enrichForBm25(...))`).
 * `getScores` returns a Float32Array of per-document scores in doc order,
 * matching the bm25s.BM25.get_scores contract used by upstream.
 */
export class Bm25Index {
  // Exposed only for save() — kept private to consumers.
  readonly #state: Bm25State

  private constructor(state: Bm25State) {
    this.#state = state
  }

  /** Build an index from an array of pre-tokenized documents. */
  static build(documents: string[][]): Bm25Index {
    const numDocs = documents.length
    const docLengths = new Float32Array(numDocs)
    const postings = new Map<string, Array<[number, number]>>()
    const docFreq = new Map<string, number>()

    let totalLen = 0
    for (let docId = 0; docId < numDocs; docId++) {
      const tokens = documents[docId] ?? []
      docLengths[docId] = tokens.length
      totalLen += tokens.length

      // Term frequencies for this document.
      const tf = new Map<string, number>()
      for (const token of tokens)
        tf.set(token, (tf.get(token) ?? 0) + 1)

      for (const [term, freq] of tf) {
        let list = postings.get(term)
        if (list === undefined) {
          list = []
          postings.set(term, list)
        }
        list.push([docId, freq])
        docFreq.set(term, (docFreq.get(term) ?? 0) + 1)
      }
    }

    const avgDocLength = numDocs > 0 ? totalLen / numDocs : 0

    return new Bm25Index({ numDocs, docLengths, avgDocLength, postings, docFreq })
  }

  /**
   * Compute BM25 scores for the given query tokens.
   *
   * Returns a Float32Array of length numDocs, in document order. When
   * `weightMask` is provided, documents with mask[i] === 0 receive a score
   * of 0 (matching bm25s.BM25.get_scores(..., weight_mask=mask) semantics).
   */
  getScores(queryTokens: string[], weightMask?: Uint8Array | null): Float32Array {
    const { numDocs, docLengths, avgDocLength, postings, docFreq } = this.#state
    const scores = new Float32Array(numDocs)
    if (queryTokens.length === 0 || numDocs === 0)
      return scores

    // De-duplicate query tokens — repeated terms shouldn't compound BM25 scores.
    const uniqueTerms = new Set(queryTokens)

    for (const term of uniqueTerms) {
      const list = postings.get(term)
      if (list === undefined)
        continue
      const df = docFreq.get(term) ?? 0
      // Lucene/Robertson IDF: log(1 + (N - df + 0.5) / (df + 0.5)).
      const idf = Math.log(1 + (numDocs - df + 0.5) / (df + 0.5))

      for (const [docId, freq] of list) {
        const dl = docLengths[docId] ?? 0
        const denom = freq + K1 * (1 - B + (B * dl) / (avgDocLength || 1))
        const contrib = (idf * (freq * (K1 + 1))) / (denom || 1)
        scores[docId] = (scores[docId] ?? 0) + contrib
      }
    }

    if (weightMask) {
      for (let i = 0; i < numDocs; i++) {
        if (!(weightMask[i] ?? 0))
          scores[i] = 0
      }
    }

    return scores
  }

  /** Persist the index to `dir`. Creates the directory if it doesn't exist. */
  async save(dir: string): Promise<void> {
    await mkdir(dir, { recursive: true })
    const { numDocs, docLengths, avgDocLength, postings, docFreq } = this.#state
    const serialized = {
      version: 1,
      numDocs,
      avgDocLength,
      docLengths: Array.from(docLengths),
      postings: Array.from(postings.entries()),
      docFreq: Array.from(docFreq.entries()),
    }
    await writeFile(path.join(dir, 'bm25.json'), JSON.stringify(serialized))
  }

  /** Load an index previously persisted with `save`. */
  static async load(dir: string): Promise<Bm25Index> {
    const raw = await readFile(path.join(dir, 'bm25.json'), 'utf8')
    const parsed = JSON.parse(raw) as {
      version: number
      numDocs: number
      avgDocLength: number
      docLengths: number[]
      postings: Array<[string, Array<[number, number]>]>
      docFreq: Array<[string, number]>
    }
    return new Bm25Index({
      numDocs: parsed.numDocs,
      docLengths: Float32Array.from(parsed.docLengths),
      avgDocLength: parsed.avgDocLength,
      postings: new Map(parsed.postings),
      docFreq: new Map(parsed.docFreq),
    })
  }
}
