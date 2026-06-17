// Port of src/semble/index/index.py
//
// CspIndex is the hybrid (dense + BM25) search orchestrator. It binds the
// indexing units (model loading + createIndexFromPath) into a single object
// that the CLI and MCP server drive.
//
// Wiring status:
//   - fromPath: implemented (this task, T003).
//   - fromGit:  stub (T005).
//   - search / findRelated: sync stubs returning [] (real ranking wired in T004).
//   - save / loadFromDisk: throwing stubs (real persistence in T006 / T007);
//     declared here so the Phase A branch type-checks (cli.ts references
//     CspIndex.loadFromDisk and index.save).

import { statSync } from 'node:fs'
import type { Chunk, ContentType, IndexStats, SearchResult } from '../types.ts'
import { ContentType as ContentTypeEnum } from '../types.ts'
import { search as runSearch } from '../search.ts'
import { createIndexFromPath } from './create.ts'
import { loadModel as loadDenseModel } from './dense.ts'
import type { Model, SelectableBasicBackend } from './dense.ts'
import type { Bm25Index } from './sparse.ts'

/** Default content selection when the caller does not specify one (code-only). */
export const DEFAULT_CONTENT: readonly ContentType[] = [ContentTypeEnum.CODE]

/** Default result count when the caller omits `topK` (matches the CLI `--top-k` default). */
const DEFAULT_TOP_K = 5

/**
 * Build a `SearchResult` for a related chunk, mirroring the `toDict` shape that
 * `search.ts` produces so downstream formatters treat both uniformly.
 */
function makeRelatedResult(chunk: Chunk, score: number): SearchResult {
  return {
    chunk,
    score,
    toDict: () => ({
      chunk: {
        content: chunk.content,
        file_path: chunk.filePath,
        start_line: chunk.startLine,
        end_line: chunk.endLine,
        language: chunk.language ?? null,
        location: `${chunk.filePath}:${chunk.startLine}-${chunk.endLine}`,
      },
      score,
    }),
  }
}

export interface CspIndexLoadOptions {
  modelPath?: string
  content?: ContentType | readonly ContentType[]
}

export interface CspIndexFromGitOptions extends CspIndexLoadOptions {
  ref?: string
}

/** Constructor payload — the fully built index state. */
export interface CspIndexState {
  model: Model
  bm25Index: Bm25Index
  semanticIndex: SelectableBasicBackend
  chunks: Chunk[]
  modelPath: string
  /** Source root the index was built from, or null (e.g. loaded from disk). */
  root: string | null
  content: readonly ContentType[]
}

/**
 * Hybrid (dense + BM25) code search index.
 *
 * Build with {@link CspIndex.fromPath} / {@link CspIndex.fromGit}, query with
 * {@link CspIndex.search} / {@link CspIndex.findRelated}, persist with
 * {@link CspIndex.save} / {@link CspIndex.loadFromDisk}.
 */
export class CspIndex {
  readonly model: Model
  readonly bm25Index: Bm25Index
  readonly semanticIndex: SelectableBasicBackend
  readonly chunks: Chunk[]
  readonly modelPath: string
  readonly root: string | null
  readonly content: readonly ContentType[]

  constructor(state: CspIndexState) {
    this.model = state.model
    this.bm25Index = state.bm25Index
    this.semanticIndex = state.semanticIndex
    this.chunks = state.chunks
    this.modelPath = state.modelPath
    this.root = state.root
    this.content = state.content
  }

  /**
   * Build an index from a local directory.
   *
   * Loads the embedding model, walks + chunks + embeds the directory via
   * {@link createIndexFromPath}, and returns a populated index.
   *
   * @throws if the path is missing, is not a directory, or has no supported files.
   */
  static async fromPath(
    path: string,
    options: CspIndexLoadOptions = {},
  ): Promise<CspIndex> {
    let stat: ReturnType<typeof statSync>
    try {
      stat = statSync(path)
    }
    catch {
      throw new Error(`Path does not exist: ${path}`)
    }
    if (!stat.isDirectory())
      throw new Error(`Path is not a directory: ${path}`)

    const { model, modelPath } = await loadDenseModel(options.modelPath)
    const content = normalizeContent(options.content)

    const { bm25Index, semanticIndex, chunks } = await createIndexFromPath(path, {
      model,
      content,
      displayRoot: path,
    })

    return new CspIndex({
      model,
      bm25Index,
      semanticIndex,
      chunks,
      modelPath,
      root: path,
      content,
    })
  }

  static async fromGit(
    _url: string,
    _options: CspIndexFromGitOptions = {},
  ): Promise<CspIndex> {
    throw new Error('CspIndex.fromGit: not yet implemented (T005)')
  }

  /** Aggregate index statistics: file count, chunk count, language histogram. */
  get stats(): IndexStats {
    const files = new Set<string>()
    const languages: Record<string, number> = {}
    for (const chunk of this.chunks) {
      files.add(chunk.filePath)
      const lang = chunk.language
      if (lang !== null && lang !== undefined)
        languages[lang] = (languages[lang] ?? 0) + 1
    }
    return {
      indexedFiles: files.size,
      totalChunks: this.chunks.length,
      languages,
    }
  }

  /**
   * Hybrid (dense + BM25) search over the indexed chunks.
   *
   * Returns `[]` for blank queries, non-positive `topK`, an empty index, or
   * when `filterLanguages`/`filterPaths` narrow the candidate pool to nothing
   * (no silent fallback to an unfiltered search). Otherwise delegates to the
   * shared ranking pipeline in {@link search.ts} — kept synchronous so the MCP
   * server can call it without `await`.
   */
  search(query: string, options: SearchOptions = {}): SearchResult[] {
    const topK = options.topK ?? DEFAULT_TOP_K
    if (query.trim().length === 0 || topK <= 0 || this.chunks.length === 0)
      return []

    const selector = this.buildSelector(options)
    if (selector !== undefined && selector.length === 0)
      return []

    return runSearch(
      query,
      this.model,
      this.semanticIndex,
      this.bm25Index,
      this.chunks,
      topK,
      selector === undefined ? {} : { selector },
    )
  }

  /**
   * Find chunks similar to a seed chunk, by re-embedding the seed's content
   * and querying the semantic backend. The seed itself is excluded from the
   * results (semble parity).
   */
  findRelated(
    // Seed needs only the chunk; accept a bare Chunk or anything carrying one
    // (e.g. a SearchResult) without forcing the caller to supply `toDict`.
    seed: Chunk | { chunk: Chunk, score?: number },
    options: SearchOptions = {},
  ): SearchResult[] {
    const seedChunk = 'chunk' in seed ? seed.chunk : seed
    const topK = options.topK ?? DEFAULT_TOP_K
    if (topK <= 0 || this.chunks.length === 0)
      return []

    // Over-fetch by one so we can drop the seed and still return up to topK.
    const queryEmbedding = this.model.encode([seedChunk.content])
    const batch = this.semanticIndex.query(queryEmbedding, topK + 1)
    const first = batch[0]
    if (first === undefined)
      return []

    const results: SearchResult[] = []
    for (const [index, distance] of first) {
      const chunk = this.chunks[index]
      if (chunk === undefined || chunk === seedChunk)
        continue
      results.push(makeRelatedResult(chunk, 1.0 - distance))
      if (results.length >= topK)
        break
    }
    return results
  }

  /**
   * Build a candidate-index selector from language/path filters, or `undefined`
   * when no filter is set. An empty `Uint32Array` (filters matched nothing) is
   * returned as-is so the caller can short-circuit to `[]`.
   */
  private buildSelector(options: SearchOptions): Uint32Array | undefined {
    const { filterLanguages, filterPaths } = options
    const hasLangFilter = filterLanguages !== undefined && filterLanguages.length > 0
    const hasPathFilter = filterPaths !== undefined && filterPaths.length > 0
    if (!hasLangFilter && !hasPathFilter)
      return undefined

    const indices: number[] = []
    for (let i = 0; i < this.chunks.length; i++) {
      const chunk = this.chunks[i]!
      if (hasLangFilter && !filterLanguages.includes(chunk.language ?? ''))
        continue
      if (hasPathFilter && !filterPaths.some(p => chunk.filePath.includes(p)))
        continue
      indices.push(i)
    }
    return Uint32Array.from(indices)
  }

  /**
   * Persist the index to `dir`. Real implementation lands in T006.
   * Declared as a throwing stub so cli.ts (`index.save(out)`) type-checks.
   */
  async save(_dir: string): Promise<void> {
    throw new Error('CspIndex.save: not yet implemented (T006)')
  }

  /**
   * Load an index previously persisted with {@link CspIndex.save}. Real
   * implementation lands in T007. Declared as a throwing stub so cli.ts
   * (`CspIndex.loadFromDisk`) type-checks in the Phase A branch.
   */
  static async loadFromDisk(_dir: string): Promise<CspIndex> {
    throw new Error('CspIndex.loadFromDisk: not yet implemented (T007)')
  }
}

export interface SearchOptions {
  topK?: number
  filterLanguages?: string[]
  filterPaths?: string[]
}

/**
 * Lazy loader for the embedding model. Returns `[model, modelPath]` so callers
 * that only need the cached path can destructure `[, modelPath]` (mcp server).
 */
export async function loadModel(modelPath?: string): Promise<[Model, string]> {
  const { model, modelPath: resolved } = await loadDenseModel(modelPath)
  return [model, resolved]
}

function normalizeContent(
  content: ContentType | readonly ContentType[] | undefined,
): readonly ContentType[] {
  if (content === undefined)
    return DEFAULT_CONTENT
  if (Array.isArray(content))
    return content
  return [content as ContentType]
}
