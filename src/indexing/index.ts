// Port of src/semble/index/index.py

import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Chunk, IndexStats, SearchResult } from '../types.ts'
import { CallType, ContentType, chunkFromDict, chunkToDict } from '../types.ts'
import { createIndexFromPath } from './create.ts'
import type { Model } from './dense.ts'
import { SelectableBasicBackend, loadModel } from './dense.ts'
import { Bm25Index } from './sparse.ts'
import { search, searchSemantic } from '../search.ts'
import { saveSearchStats } from '../stats.ts'
import { PersistencePath } from './types.ts'

/** Default content set: code only. */
export const DEFAULT_CONTENT: readonly ContentType[] = [ContentType.Code]
/** All content types — used by the `--content all` CLI flag. */
export const ALL_CONTENT: readonly ContentType[] = [ContentType.Code, ContentType.Docs, ContentType.Config]

/** Timeout (ms) applied to `git clone` invocations. */
export const GIT_CLONE_TIMEOUT_MS = Number.parseInt(process.env.CSP_CLONE_TIMEOUT ?? '60', 10) * 1000

export interface CspIndexConstructorArgs {
  model: Model
  bm25Index: Bm25Index
  semanticIndex: SelectableBasicBackend
  chunks: Chunk[]
  modelPath: string
  root?: string | null
  content?: ContentType | readonly ContentType[]
}

export interface FromPathOptions {
  extensions?: readonly string[]
  content?: ContentType | readonly ContentType[]
  modelPath?: string | null
}

export interface FromGitOptions extends FromPathOptions {
  ref?: string | null
}

export interface SearchInvocationOptions {
  topK?: number
  alpha?: number | null
  filterLanguages?: readonly string[]
  filterPaths?: readonly string[]
  rerank?: boolean | null
}

export interface FindRelatedOptions {
  topK?: number
}

/** Fast local code index with hybrid (semantic + BM25) search. */
export class CspIndex {
  readonly model: Model
  readonly chunks: Chunk[]

  private readonly _bm25Index: Bm25Index
  private readonly _semanticIndex: SelectableBasicBackend
  private readonly _modelPath: string
  private readonly _root: string | null
  private readonly _content: readonly ContentType[]
  private readonly _fileSizes: Record<string, number>
  private readonly _fileMapping: Record<string, number[]>
  private readonly _languageMapping: Record<string, number[]>

  constructor(args: CspIndexConstructorArgs) {
    this.model = args.model
    this.chunks = args.chunks
    this._bm25Index = args.bm25Index
    this._semanticIndex = args.semanticIndex
    this._modelPath = args.modelPath
    this._root = args.root ?? null
    this._content = normalizeContent(args.content ?? DEFAULT_CONTENT)
    this._fileSizes = this._root ? this._computeFileSizes(this._root) : {}
    const mappings = this._populateMapping()
    this._fileMapping = mappings.file
    this._languageMapping = mappings.language
  }

  /** Aggregate index statistics. */
  get stats(): IndexStats {
    const languageCounts: Record<string, number> = {}
    for (const chunk of this.chunks) {
      if (chunk.language) {
        languageCounts[chunk.language] = (languageCounts[chunk.language] ?? 0) + 1
      }
    }
    return {
      indexedFiles: Object.keys(this._fileMapping).length,
      totalChunks: this.chunks.length,
      languages: languageCounts,
    }
  }

  /** Create and index a CspIndex from a local directory. */
  static async fromPath(
    path: string | URL,
    options: FromPathOptions = {},
  ): Promise<CspIndex> {
    const resolved = await resolveDirectory(path)
    const { model, modelPath } = await loadModel(options.modelPath)
    const normalized = normalizeContent(options.content ?? DEFAULT_CONTENT)
    const created = await createIndexFromPath(resolved, {
      model,
      ...(options.extensions !== undefined ? { extensions: options.extensions } : {}),
      content: normalized,
      displayRoot: resolved,
    })
    return new CspIndex({
      model,
      bm25Index: created.bm25Index,
      semanticIndex: created.semanticIndex,
      chunks: created.chunks,
      modelPath,
      root: resolved,
      content: normalized,
    })
  }

  /** Clone a git repository to a tmp dir, index it, then clean up the clone. */
  static async fromGit(
    url: string,
    options: FromGitOptions = {},
  ): Promise<CspIndex> {
    const normalized = normalizeContent(options.content ?? DEFAULT_CONTENT)
    const tmpDir = await mkdtemp(join(tmpdir(), 'csp-'))
    try {
      await runGitClone(url, tmpDir, options.ref ?? null)

      const { model, modelPath } = await loadModel(options.modelPath)
      const resolved = resolve(tmpDir)
      const created = await createIndexFromPath(resolved, {
        model,
        ...(options.extensions !== undefined ? { extensions: options.extensions } : {}),
        content: normalized,
        displayRoot: resolved,
      })
      return new CspIndex({
        model,
        bm25Index: created.bm25Index,
        semanticIndex: created.semanticIndex,
        chunks: created.chunks,
        modelPath,
        root: resolved,
        content: normalized,
      })
    }
    finally {
      await rm(tmpDir, { recursive: true, force: true })
    }
  }

  /** Load a previously-saved index from disk. */
  static async loadFromDisk(path: string): Promise<CspIndex> {
    let exists = true
    try {
      await stat(path)
    }
    catch {
      exists = false
    }
    if (!exists) throw new Error(`Index not found at ${path}`)

    const persistencePaths = PersistencePath.fromPath(path)
    const missing = persistencePaths.nonExisting()
    if (missing.length > 0) {
      throw new Error(`Index not found at ${path}. Missing: ${missing.join(', ')}`)
    }

    const bm25Index = Bm25Index.load(persistencePaths.bm25Index)
    const semanticIndex = SelectableBasicBackend.load(persistencePaths.semanticIndex)
    const metadataRaw = await readFile(persistencePaths.metadata, 'utf8')
    const metadata = JSON.parse(metadataRaw) as {
      root_path?: string | null
      model_path?: string | null
    }
    const chunkRaw = await readFile(persistencePaths.chunks, 'utf8')
    const chunkData = JSON.parse(chunkRaw) as Array<Record<string, unknown>>
    const chunks = chunkData.map(chunkFromDict)

    const { model, modelPath } = await loadModel(metadata.model_path ?? null)
    return new CspIndex({
      model,
      bm25Index,
      semanticIndex,
      chunks,
      modelPath,
      root: metadata.root_path ?? null,
    })
  }

  /** Search the index and return the top-k most relevant chunks. */
  search(query: string, options: SearchInvocationOptions = {}): SearchResult[] {
    if (this.chunks.length === 0 || query.trim().length === 0) return []

    const topK = options.topK ?? 10
    const filterLanguages = options.filterLanguages
    const filterPaths = options.filterPaths
    const resolvedRerank = options.rerank ?? this._content.includes(ContentType.Code)
    const selector = this._getSelectorVector(filterLanguages, filterPaths)

    const results = search(
      query,
      this.model,
      this._semanticIndex,
      this._bm25Index,
      this.chunks,
      topK,
      {
        alpha: options.alpha ?? null,
        ...(selector !== null ? { selector } : {}),
        rerank: resolvedRerank,
      },
    )
    saveSearchStats(results, CallType.Search, this._fileSizes)
    return results
  }

  /** Return chunks semantically similar to the given chunk or search result. */
  findRelated(
    source: Chunk | SearchResult,
    options: FindRelatedOptions = {},
  ): SearchResult[] {
    const topK = options.topK ?? 5
    const target = isSearchResult(source) ? source.chunk : source
    const selector
      = target.language
        ? this._getSelectorVector([target.language], undefined)
        : null
    const results = searchSemantic(
      target.content,
      this.model,
      this._semanticIndex,
      this.chunks,
      topK + 1,
      selector,
    )
    const filtered = results
      .filter(r => !sameChunk(r.chunk, target))
      .slice(0, topK)
    saveSearchStats(filtered, CallType.FindRelated, this._fileSizes)
    return filtered
  }

  /** Persist the index to disk under `path` (created if missing). */
  async save(path: string): Promise<void> {
    await mkdir(path, { recursive: true })
    const persistencePaths = PersistencePath.fromPath(path)
    this._bm25Index.save(persistencePaths.bm25Index)
    this._semanticIndex.save(persistencePaths.semanticIndex)
    const chunksAsDict = this.chunks.map(chunkToDict)
    await writeFile(persistencePaths.chunks, JSON.stringify(chunksAsDict))
    const metadata = {
      root_path: this._root,
      time: Date.now() / 1000,
      model_path: this._modelPath,
    }
    await writeFile(persistencePaths.metadata, JSON.stringify(metadata))
  }

  private _populateMapping(): {
    file: Record<string, number[]>
    language: Record<string, number[]>
  } {
    const file: Record<string, number[]> = {}
    const language: Record<string, number[]> = {}
    for (let i = 0; i < this.chunks.length; i++) {
      const chunk = this.chunks[i]!
      if (chunk.language) {
        const arr = language[chunk.language]
        if (arr) arr.push(i)
        else language[chunk.language] = [i]
      }
      const arr = file[chunk.filePath]
      if (arr) arr.push(i)
      else file[chunk.filePath] = [i]
    }
    return { file, language }
  }

  private _computeFileSizes(root: string): Record<string, number> {
    const sizes: Record<string, number> = {}
    for (const chunk of this.chunks) {
      if (chunk.filePath in sizes) continue
      try {
        // Mirror Python's `root / chunk.file_path`: absolute paths win,
        // relative paths resolve against `root`.
        const abs = resolve(root, chunk.filePath)
        const buf = readFileSyncSafe(abs)
        if (buf !== null) sizes[chunk.filePath] = buf.length
      }
      catch {
        /* swallow */
      }
    }
    return sizes
  }

  private _getSelectorVector(
    filterLanguages?: readonly string[],
    filterPaths?: readonly string[],
  ): number[] | null {
    const out = new Set<number>()
    for (const language of filterLanguages ?? []) {
      const ids = this._languageMapping[language]
      if (ids) for (const i of ids) out.add(i)
    }
    for (const filename of filterPaths ?? []) {
      const ids = this._fileMapping[filename]
      if (ids) for (const i of ids) out.add(i)
    }
    if (out.size === 0) return null
    return [...out].sort((a, b) => a - b)
  }
}

function normalizeContent(
  content: ContentType | readonly ContentType[],
): readonly ContentType[] {
  if (Array.isArray(content)) return content
  return [content as ContentType]
}

function isSearchResult(value: Chunk | SearchResult): value is SearchResult {
  return (value as SearchResult).chunk !== undefined
    && typeof (value as SearchResult).score === 'number'
}

function sameChunk(a: Chunk, b: Chunk): boolean {
  return (
    a.filePath === b.filePath
    && a.startLine === b.startLine
    && a.endLine === b.endLine
    && a.content === b.content
  )
}

async function resolveDirectory(path: string | URL): Promise<string> {
  const raw = path instanceof URL ? fileURLToPath(path) : path
  let info
  try {
    info = await stat(raw)
  }
  catch {
    throw new Error(`Path does not exist: ${raw}`)
  }
  if (!info.isDirectory()) {
    throw new Error(`Path is not a directory: ${raw}`)
  }
  // Drop any trailing separator for consistency with semble's Path.resolve().
  let resolved = resolve(raw)
  if (resolved.length > 1 && resolved.endsWith(sep)) {
    resolved = resolved.slice(0, -1)
  }
  return resolved
}

function readFileSyncSafe(path: string): string | null {
  try {
    return readFileSync(path, { encoding: 'utf8' })
  }
  catch {
    return null
  }
}

/**
 * Shell-out to `git clone --depth 1` into `tmpDir`.
 *
 * Uses `spawn` (not `execFile`) so stdin can be redirected to `/dev/null` —
 * this mirrors semble's `subprocess.run(..., stdin=subprocess.DEVNULL)` and
 * prevents a hung remote from blocking on a tty prompt.
 */
async function runGitClone(url: string, tmpDir: string, ref: string | null): Promise<void> {
  // `--` prevents `url` from being interpreted as a git option (e.g. `--upload-pack=...`).
  const args = [
    'clone',
    '--depth',
    '1',
    ...(ref ? ['--branch', ref] : []),
    '--',
    url,
    tmpDir,
  ]
  await new Promise<void>((resolvePromise, rejectPromise) => {
    let child
    try {
      child = spawn('git', args, { stdio: ['ignore', 'pipe', 'pipe'] })
    }
    catch (err) {
      const e = err as NodeJS.ErrnoException
      if (e.code === 'ENOENT') {
        rejectPromise(new Error('git is not installed or not on PATH'))
        return
      }
      rejectPromise(err as Error)
      return
    }
    let stderr = ''
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      child.kill('SIGTERM')
    }, GIT_CLONE_TIMEOUT_MS)
    child.stderr?.setEncoding('utf8')
    child.stderr?.on('data', (chunk: string) => {
      stderr += chunk
    })
    child.on('error', (err: NodeJS.ErrnoException) => {
      clearTimeout(timer)
      if (err.code === 'ENOENT') {
        rejectPromise(new Error('git is not installed or not on PATH'))
        return
      }
      rejectPromise(err)
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      if (timedOut) {
        rejectPromise(new Error(
          `git clone timed out for ${JSON.stringify(url)} (limit: ${GIT_CLONE_TIMEOUT_MS / 1000} s)`,
        ))
        return
      }
      if (code !== 0) {
        rejectPromise(new Error(`git clone failed for ${JSON.stringify(url)}:\n${stderr.trim()}`))
        return
      }
      resolvePromise()
    })
  })
}
