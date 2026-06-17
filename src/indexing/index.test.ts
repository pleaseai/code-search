// Tests for src/indexing/index.ts (CspIndex)

import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
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
  const model = makeStubModel(4)
  const vectors = chunks.map((_, i) => {
    const v = new Float32Array(4)
    v[0] = i + 1
    return v
  })
  return new CspIndex({
    model,
    bm25Index: Bm25Index.build(chunks.map(() => ['x'])),
    semanticIndex: new SelectableBasicBackend(vectors),
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

  it('returns [] when topK <= 0', () => {
    const chunks = [makeChunk('a.ts', 1, 1)]
    const idx = buildIndex(chunks)
    expect(idx.search('anything', { topK: 0 })).toEqual([])
    expect(idx.search('anything', { topK: -1 })).toEqual([])
  })

  it('returns [] when filters are set but match nothing (no fallback to unfiltered)', () => {
    // Regression: previously an empty selector was treated as "no filter"
    // which fell back to an unfiltered search — silently ignoring user intent.
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'alpha'),
      makeChunk('b.py', 1, 10, 'python', 'beta'),
    ]
    const idx = buildIndex(chunks)
    expect(idx.search('anything', { filterLanguages: ['nonexistent'] })).toEqual([])
    expect(idx.search('anything', { filterPaths: ['nope.ts'] })).toEqual([])
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

describe('CspIndex.save', () => {
  let dir: string

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'csp-save-'))
  })
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  function readJson(name: string): unknown {
    return JSON.parse(readFileSync(join(dir, name), 'utf8'))
  }

  it('writes all index artifacts to the target directory', async () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'A'),
      makeChunk('b.ts', 1, 5, 'python', 'B'),
    ]
    const idx = buildIndex(chunks)
    await idx.save(dir)

    for (const name of ['manifest.json', 'chunks.json', 'bm25.json', 'vectors.bin', 'args.json'])
      expect(existsSync(join(dir, name))).toBe(true)
  })

  it('creates the target directory if it does not exist', async () => {
    const nested = join(dir, 'a', 'b', 'idx')
    const idx = buildIndex([makeChunk('a.ts', 1, 10)])
    await idx.save(nested)
    expect(existsSync(join(nested, 'manifest.json'))).toBe(true)
  })

  it('writes a manifest with schema version, content, source id, and model id', async () => {
    const chunks: Chunk[] = [makeChunk('a.ts', 1, 10, 'typescript', 'A')]
    const idx = buildIndex(chunks)
    await idx.save(dir)

    const manifest = readJson('manifest.json') as Record<string, unknown>
    expect(manifest.schemaVersion).toBe(1)
    expect(manifest.content).toEqual([...DEFAULT_CONTENT])
    // buildIndex sets root: null → sourceId is null.
    expect(manifest.sourceId).toBeNull()
    expect(manifest.modelId).toBe('test-model')
    // contentHash is deterministic and non-empty.
    expect(typeof manifest.contentHash).toBe('string')
    expect((manifest.contentHash as string).length).toBeGreaterThan(0)
  })

  it('serializes chunks in camelCase (chunkToDict) form, preserving order', async () => {
    const chunks: Chunk[] = [
      makeChunk('a.ts', 1, 10, 'typescript', 'A'),
      makeChunk('b.ts', 1, 5, 'python', 'B'),
    ]
    const idx = buildIndex(chunks)
    await idx.save(dir)

    const serialized = readJson('chunks.json') as Array<Record<string, unknown>>
    expect(serialized.length).toBe(2)
    expect(serialized.map(c => c.filePath)).toEqual(['a.ts', 'b.ts'])
    const first = serialized[0]!
    expect(first.content).toBe('A')
    expect(first.startLine).toBe(1)
    expect(first.endLine).toBe(10)
    expect(first.language).toBe('typescript')
    expect(first.location).toBe('a.ts:1-10')
    // snake_case wire keys must NOT leak into the round-trip format.
    expect(first.file_path).toBeUndefined()
  })

  it('produces a deterministic contentHash for identical chunks', async () => {
    const make = (): CspIndex =>
      buildIndex([makeChunk('a.ts', 1, 10, 'typescript', 'A')])

    const dir2 = mkdtempSync(join(tmpdir(), 'csp-save-2-'))
    try {
      await make().save(dir)
      await make().save(dir2)
      const h1 = (JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8')) as Record<string, unknown>).contentHash
      const h2 = (JSON.parse(readFileSync(join(dir2, 'manifest.json'), 'utf8')) as Record<string, unknown>).contentHash
      expect(h1).toBe(h2)
    } finally {
      rmSync(dir2, { recursive: true, force: true })
    }
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
    const idx = await CspIndex.fromPath(dir, { content: ContentType.CODE })
    expect(idx.stats.totalChunks).toBeGreaterThan(0)
    expect(idx.stats.indexedFiles).toBe(1)
    expect(idx.chunks[0]!.filePath).toBe('sample.ts')
  })
})

describe('CspIndex.fromGit', () => {
  let workdir: string
  let repoDir: string

  /** Run a git command in `cwd`, throwing with stderr on failure. */
  function git(cwd: string, ...args: string[]): void {
    const res = spawnSync('git', args, {
      cwd,
      encoding: 'utf8',
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    })
    if (res.status !== 0)
      throw new Error(`git ${args.join(' ')} failed: ${res.stderr}`)
  }

  /** Count leftover clone temp dirs so we can assert cleanup. */
  function cloneTempDirCount(): number {
    return readdirSync(tmpdir()).filter(name => name.startsWith('csp-git-')).length
  }

  beforeEach(() => {
    workdir = mkdtempSync(join(tmpdir(), 'csp-git-src-'))
    // A real, non-bare local repo with one committed TS file. `git clone` can
    // shallow-clone this over a file:// URL with no network.
    repoDir = join(workdir, 'repo')
    spawnSync('git', ['init', repoDir], { encoding: 'utf8' })
    git(repoDir, 'config', 'user.email', 'test@example.com')
    git(repoDir, 'config', 'user.name', 'Test')
    git(repoDir, 'config', 'commit.gpgsign', 'false')
    writeFileSync(
      join(repoDir, 'sample.ts'),
      'export function greet(name: string) {\n  return `hi ${name}`\n}\n',
    )
    git(repoDir, 'add', '.')
    git(repoDir, 'commit', '-m', 'initial')
  })
  afterEach(() => {
    rmSync(workdir, { recursive: true, force: true })
  })

  it('shallow-clones the repo and builds a populated index', async () => {
    const before = cloneTempDirCount()
    const idx = await CspIndex.fromGit(`file://${repoDir}`, {
      content: ContentType.CODE,
    })
    expect(idx.stats.totalChunks).toBeGreaterThan(0)
    expect(idx.stats.indexedFiles).toBe(1)
    expect(idx.chunks[0]!.filePath).toBe('sample.ts')
    // The temporary checkout must be cleaned up (no leak) after success.
    expect(cloneTempDirCount()).toBe(before)
  })

  it('cleans up the temp checkout even when clone fails', async () => {
    const before = cloneTempDirCount()
    const bogus = join(workdir, 'does-not-exist.git')
    expect(existsSync(bogus)).toBe(false)
    await expect(
      CspIndex.fromGit(`file://${bogus}`, { content: ContentType.CODE }),
    ).rejects.toThrow(/clone/i)
    // Failure path must not leak the temp checkout directory either.
    expect(cloneTempDirCount()).toBe(before)
  })
})
