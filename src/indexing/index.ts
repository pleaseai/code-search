// Port of src/semble/index/index.py
//
// CspIndex is the hybrid (dense + BM25) search orchestrator. It binds the
// indexing units (model loading + createIndexFromPath) into a single object
// that the CLI and MCP server drive.
//
// Wiring status:
//   - fromPath: implemented (this task, T003).
//   - fromGit:  stub (T005).
//   - search / findRelated: sync delegation to search.ts (T004).
//   - save: implemented (this task, T006) — writes manifest + chunks + bm25 + dense.
//   - loadFromDisk: throwing stub (real persistence in T007); declared here so
//     the Phase A/B branch type-checks (cli.ts references CspIndex.loadFromDisk).

import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, existsSync, mkdtempSync, rmSync } from 'node:fs'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'
import type { Chunk, ContentType, IndexStats, SearchResult } from '../types.ts'
import { chunkFromDict, chunkToDict, ContentType as ContentTypeEnum } from '../types.ts'
import { search as runSearch } from '../search.ts'
import { createIndexFromPath } from './create.ts'
import { loadModel as loadDenseModel, makeStubModel } from './dense.ts'
import type { Model } from './dense.ts'
import { SelectableBasicBackend } from './dense.ts'
import { Bm25Index } from './sparse.ts'

/** Promisified `git` runner — keeps the network-bound clone off the event loop. */
const execFileAsync = promisify(execFile)

/**
 * On-disk index schema version. Bumped when the persisted artifact layout or
 * format changes; {@link CspIndex.loadFromDisk} (T007) rejects mismatches.
 */
export const INDEX_SCHEMA_VERSION = 1

/**
 * Persisted index manifest — the top-level metadata that ties the on-disk
 * artifacts (chunks.json / bm25.json / vectors.bin / args.json) together and
 * guards against loading an incompatible index.
 */
export interface IndexManifest {
  schemaVersion: number
  /** Hash of the chunk contents — deterministic identity of the indexed corpus. */
  contentHash: string
  /** Source root the index was built from (absolute path / git URL), or null. */
  sourceId: string | null
  /** Content types this index covers. */
  content: ContentType[]
  /** Embedding model identifier, so a load can reject a model mismatch. */
  modelId: string
}

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

/** Options for {@link CspIndex.save}. */
export interface CspIndexSaveOptions {
  /**
   * Override for the manifest `contentHash`. {@link loadOrBuildIndex} injects a
   * source-file hash here so cache validity can be checked before a rebuild.
   * Omitted → defaults to the serialized-chunks hash (T006 behavior).
   */
  contentHash?: string
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
    let pathStats: Awaited<ReturnType<typeof stat>>
    try {
      pathStats = await stat(path)
    }
    catch {
      throw new Error(`Path does not exist: ${path}`)
    }
    if (!pathStats.isDirectory())
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

  /**
   * Build an index from a remote git URL.
   *
   * Shallow-clones `url` into a fresh `0700` temp directory (non-interactive —
   * credential prompts are suppressed), then reuses the {@link CspIndex.fromPath}
   * pipeline against the clone root so `.cspignore` / `.gitignore` rules at the
   * checkout root are honored. The temp directory is always removed afterward,
   * on both the success and failure paths.
   *
   * @throws if the clone fails (bad URL, auth required, git missing).
   */
  static async fromGit(
    url: string,
    options: CspIndexFromGitOptions = {},
  ): Promise<CspIndex> {
    const dir = mkdtempSync(join(tmpdir(), 'csp-git-'))
    chmodSync(dir, 0o700)
    try {
      await cloneShallow(url, dir, options.ref)
      const { ref: _ref, ...fromPathOptions } = options
      const index = await CspIndex.fromPath(dir, fromPathOptions)
      // fromPath roots the index at the temp checkout, which we delete in the
      // `finally` below — re-root at the git URL so a persisted manifest records
      // a stable, meaningful sourceId (not a vanished temp path).
      return new CspIndex({
        model: index.model,
        bm25Index: index.bm25Index,
        semanticIndex: index.semanticIndex,
        chunks: index.chunks,
        modelPath: index.modelPath,
        root: url,
        content: index.content,
      })
    }
    finally {
      rmSync(dir, { recursive: true, force: true })
    }
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
   * Persist the index to `dir`, writing five artifacts:
   *   - `chunks.json`   — chunks in camelCase round-trip form ({@link chunkToDict}).
   *   - `bm25.json`     — sparse index ({@link Bm25Index.save}).
   *   - `vectors.bin` + `args.json` — dense index ({@link SelectableBasicBackend.save}).
   *   - `manifest.json` — schema version, content hash, source id, content, model id.
   *
   * The directory is created if absent. The five file names are mutually
   * distinct, so the backends do not clobber one another. The dense backend
   * writes already-normalized vectors and re-normalizes on load idempotently,
   * so the round-trip is bit-stable (verified — no float drift, NFR-002).
   *
   * `options.contentHash` overrides the manifest's `contentHash`. The auto-cache
   * orchestrator ({@link loadOrBuildIndex}) injects a *source-file* hash here so
   * the manifest records a value it can recompute and compare against the live
   * source before a build. When omitted, `contentHash` defaults to the hash of
   * the serialized chunks (T006 behavior — backward compatible).
   */
  async save(dir: string, options: CspIndexSaveOptions = {}): Promise<void> {
    await mkdir(dir, { recursive: true })

    const serializedChunks = this.chunks.map(chunkToDict)
    await writeFile(join(dir, 'chunks.json'), JSON.stringify(serializedChunks))

    await this.bm25Index.save(dir)
    await this.semanticIndex.save(dir)

    const manifest: IndexManifest = {
      schemaVersion: INDEX_SCHEMA_VERSION,
      contentHash: options.contentHash ?? hashChunks(serializedChunks),
      sourceId: this.root,
      content: [...this.content],
      modelId: this.modelPath,
    }
    await writeFile(join(dir, 'manifest.json'), JSON.stringify(manifest))
  }

  /**
   * Load an index previously persisted with {@link CspIndex.save}.
   *
   * Validates the directory and all five artifacts exist, checks the manifest
   * schema version matches {@link INDEX_SCHEMA_VERSION}, then restores chunks
   * ({@link chunkFromDict}), the BM25 index ({@link Bm25Index.load}), the dense
   * backend ({@link SelectableBasicBackend.load}), and reloads the embedding
   * model identified by the manifest. The chunk round-trip is lossless
   * (camelCase symmetry with {@link CspIndex.save}).
   *
   * @throws if the directory is missing, an artifact is missing, or the
   *   manifest schema version does not match.
   */
  static async loadFromDisk(dir: string): Promise<CspIndex> {
    if (!existsSync(dir))
      throw new Error(`Index not found: ${dir}`)

    const artifacts = ['manifest.json', 'chunks.json', 'bm25.json', 'vectors.bin', 'args.json']
    for (const name of artifacts) {
      if (!existsSync(join(dir, name)))
        throw new Error(`Missing: ${join(dir, name)}`)
    }

    const rawManifest = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8')) as unknown
    // Version check first so a stale index gets the precise mismatch error even
    // if its (older) shape would fail full validation below.
    const rawVersion = (rawManifest as { schemaVersion?: unknown } | null)?.schemaVersion
    if (rawVersion !== INDEX_SCHEMA_VERSION) {
      throw new Error(
        `Index schema version mismatch: expected ${INDEX_SCHEMA_VERSION}, got ${String(rawVersion)}`,
      )
    }
    const manifest = parseManifest(rawManifest)

    const serializedChunks = JSON.parse(await readFile(join(dir, 'chunks.json'), 'utf8')) as unknown[]
    const chunks = serializedChunks.map(c => chunkFromDict(c as Parameters<typeof chunkFromDict>[0]))

    const bm25Index = await Bm25Index.load(dir)
    const semanticIndex = await SelectableBasicBackend.load(dir)

    const { model, modelPath } = await loadDenseModel(manifest.modelId)
    // Keep the query model's dimension aligned with the persisted vectors so
    // re-embedded queries are comparable to the stored backend. (The stub model
    // is dimension-agnostic; the real model's dim is fixed by its weights.)
    const alignedModel = model.dim === semanticIndex.dim
      ? model
      : makeStubModel(semanticIndex.dim)

    return new CspIndex({
      model: alignedModel,
      bm25Index,
      semanticIndex,
      chunks,
      modelPath,
      root: manifest.sourceId,
      content: manifest.content,
    })
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

/**
 * Shallow-clone `url` into `dir` (already created, empty). Runs git
 * non-interactively so a missing-credential prompt fails fast instead of
 * hanging. Throws a clear error (including git's stderr) when the clone fails.
 */
async function cloneShallow(url: string, dir: string, ref?: string): Promise<void> {
  // A ref beginning with `-` would be parsed by git as a flag (e.g.
  // `--upload-pack=…`, `--config=…`) rather than a branch name — the `--`
  // separator below only shields the trailing url/dir, not `--branch <ref>`.
  // Reject it so a hostile ref can't inject git options (CWE-88).
  if (ref !== undefined && ref.startsWith('-'))
    throw new Error(`Invalid git ref (must not start with '-'): ${ref}`)

  const args = ['clone', '--depth', '1']
  if (ref !== undefined)
    args.push('--branch', ref)
  args.push('--', url, dir)

  // Async clone: a network-bound git clone must not block the event loop (an
  // MCP server may be serving other requests concurrently).
  try {
    await execFileAsync('git', args, {
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    })
  }
  catch (err) {
    const e = err as { stderr?: string, message?: string }
    const detail = (e.stderr ?? '').trim() || e.message || 'unknown error'
    throw new Error(`git clone failed for ${url}: ${detail}`)
  }
}

/**
 * Deterministic content hash of the serialized chunks. T006 only needs a stable
 * identity for the indexed corpus; the precise repo-content hash used for cache
 * invalidation lands in T009 (cache.ts). Uses sha256 over the chunks JSON so
 * identical chunk sets always produce the same digest.
 */
function hashChunks(serializedChunks: unknown[]): string {
  return createHash('sha256').update(JSON.stringify(serializedChunks)).digest('hex')
}

function isContentType(value: unknown): value is ContentType {
  return typeof value === 'string'
    && (Object.values(ContentTypeEnum) as string[]).includes(value)
}

/**
 * Parse and validate a persisted `manifest.json`. The manifest is an on-disk
 * trust boundary, so every field is checked at runtime (mirroring
 * {@link chunkFromDict}) — a corrupt or hand-edited manifest fails loudly here
 * instead of producing a `CspIndex` whose typed fields (`content`, `sourceId`,
 * `modelId`) silently lie about the persisted data.
 */
export function parseManifest(raw: unknown): IndexManifest {
  if (raw === null || typeof raw !== 'object')
    throw new Error('Invalid manifest: not an object')
  const m = raw as Record<string, unknown>
  if (typeof m.schemaVersion !== 'number')
    throw new Error('Invalid manifest: schemaVersion must be a number')
  if (typeof m.contentHash !== 'string')
    throw new Error('Invalid manifest: contentHash must be a string')
  if (!(m.sourceId === null || typeof m.sourceId === 'string'))
    throw new Error('Invalid manifest: sourceId must be a string or null')
  if (typeof m.modelId !== 'string')
    throw new Error('Invalid manifest: modelId must be a string')
  if (!Array.isArray(m.content) || !m.content.every(isContentType))
    throw new Error('Invalid manifest: content must be an array of ContentType')
  return {
    schemaVersion: m.schemaVersion,
    contentHash: m.contentHash,
    sourceId: m.sourceId,
    content: m.content,
    modelId: m.modelId,
  }
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
