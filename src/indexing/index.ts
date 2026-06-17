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

import type { Chunk, ContentType, IndexStats, SearchResult } from '../types.ts'
import { ContentType as ContentTypeEnum } from '../types.ts'
import { createIndexFromPath } from './create.ts'
import { loadModel as loadDenseModel } from './dense.ts'
import type { Model, SelectableBasicBackend } from './dense.ts'
import type { Bm25Index } from './sparse.ts'

/** Default content selection when the caller does not specify one (code-only). */
export const DEFAULT_CONTENT: readonly ContentType[] = [ContentTypeEnum.CODE]

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

  search(_query: string, _options: SearchOptions = {}): SearchResult[] {
    // Real hybrid ranking is wired in T004.
    return []
  }

  findRelated(
    _seed: Chunk | SearchResult,
    _options: SearchOptions = {},
  ): SearchResult[] {
    // Real related-chunk ranking is wired in T004.
    return []
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
