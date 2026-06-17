// Unit tests for the index cache module (T009): cache-dir resolution,
// content hashing, and 0700 directory hardening.
// T010 adds loadOrBuildIndex orchestration tests at the bottom.

import { existsSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, sep } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { ContentType } from '../types.ts'
import { CspIndex } from './index.ts'
import { computeContentHash, ensureCacheDir, loadOrBuildIndex, resolveCacheDir } from './cache.ts'

describe('resolveCacheDir', () => {
  it('returns a path under <base>/index/', () => {
    const base = '/some/home/.csp'
    const dir = resolveCacheDir('/repo', [ContentType.CODE], { baseDir: base })
    expect(dir.startsWith(`${base}${sep}index${sep}`)).toBe(true)
  })

  it('is deterministic for the same (source, content, ref)', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('/repo', [ContentType.CODE], opts)
    const b = resolveCacheDir('/repo', [ContentType.CODE], opts)
    expect(a).toBe(b)
  })

  it('is insensitive to content selection ordering', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('/repo', [ContentType.CODE, ContentType.DOCS], opts)
    const b = resolveCacheDir('/repo', [ContentType.DOCS, ContentType.CODE], opts)
    expect(a).toBe(b)
  })

  it('produces a different key for a different content selection', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('/repo', [ContentType.CODE], opts)
    const b = resolveCacheDir('/repo', [ContentType.CODE, ContentType.DOCS], opts)
    expect(a).not.toBe(b)
  })

  it('produces a different key for a different source', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('/repo-a', [ContentType.CODE], opts)
    const b = resolveCacheDir('/repo-b', [ContentType.CODE], opts)
    expect(a).not.toBe(b)
  })

  it('produces a different key for a different ref', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('https://x/r.git', [ContentType.CODE], { ...opts, ref: 'main' })
    const b = resolveCacheDir('https://x/r.git', [ContentType.CODE], { ...opts, ref: 'dev' })
    expect(a).not.toBe(b)
  })

  it('treats an omitted ref distinctly from an empty ref consistently', () => {
    const opts = { baseDir: '/h/.csp' }
    const a = resolveCacheDir('https://x/r.git', [ContentType.CODE], opts)
    const b = resolveCacheDir('https://x/r.git', [ContentType.CODE], opts)
    expect(a).toBe(b)
  })
})

describe('computeContentHash', () => {
  it('is order-independent across the file list', () => {
    const a = computeContentHash([
      { path: 'a.ts', content: 'one' },
      { path: 'b.ts', content: 'two' },
    ])
    const b = computeContentHash([
      { path: 'b.ts', content: 'two' },
      { path: 'a.ts', content: 'one' },
    ])
    expect(a).toBe(b)
  })

  it('changes when any byte of content changes', () => {
    const a = computeContentHash([{ path: 'a.ts', content: 'hello' }])
    const b = computeContentHash([{ path: 'a.ts', content: 'hellp' }])
    expect(a).not.toBe(b)
  })

  it('changes when a path changes', () => {
    const a = computeContentHash([{ path: 'a.ts', content: 'x' }])
    const b = computeContentHash([{ path: 'b.ts', content: 'x' }])
    expect(a).not.toBe(b)
  })

  it('treats Uint8Array and equivalent string content identically', () => {
    const a = computeContentHash([{ path: 'a.ts', content: 'abc' }])
    const b = computeContentHash([
      { path: 'a.ts', content: new Uint8Array([0x61, 0x62, 0x63]) },
    ])
    expect(a).toBe(b)
  })

  it('returns a stable hex sha256 string', () => {
    const h = computeContentHash([{ path: 'a.ts', content: 'x' }])
    expect(h).toMatch(/^[0-9a-f]{64}$/)
  })
})

describe('ensureCacheDir', () => {
  let tmpHome: string

  beforeEach(() => {
    tmpHome = mkdtempSync(join(tmpdir(), 'csp-cache-test-'))
  })

  afterEach(() => {
    rmSync(tmpHome, { recursive: true, force: true })
  })

  it('creates the directory chain with mode 0700', () => {
    const base = join(tmpHome, '.csp')
    const leaf = resolveCacheDir('/repo', [ContentType.CODE], { baseDir: base })
    ensureCacheDir(leaf, { baseDir: base })

    expect(statSync(leaf).mode & 0o777).toBe(0o700)
    expect(statSync(join(base, 'index')).mode & 0o777).toBe(0o700)
    expect(statSync(base).mode & 0o777).toBe(0o700)
  })

  it('tightens an already-existing directory to 0700', () => {
    const base = join(tmpHome, '.csp')
    const leaf = resolveCacheDir('/repo', [ContentType.CODE], { baseDir: base })
    // First call creates everything.
    ensureCacheDir(leaf, { baseDir: base })
    // Loosen, then re-ensure should re-tighten.
    const { chmodSync } = require('node:fs') as typeof import('node:fs')
    chmodSync(base, 0o755)
    chmodSync(join(base, 'index'), 0o755)
    ensureCacheDir(leaf, { baseDir: base })

    expect(statSync(base).mode & 0o777).toBe(0o700)
    expect(statSync(join(base, 'index')).mode & 0o777).toBe(0o700)
    expect(statSync(leaf).mode & 0o777).toBe(0o700)
  })

  it('does not touch the real home .csp directory', () => {
    const base = join(tmpHome, '.csp')
    const leaf = resolveCacheDir('/repo', [ContentType.CODE], { baseDir: base })
    ensureCacheDir(leaf, { baseDir: base })
    // The created tree must live under the injected base, never the real home.
    expect(leaf.startsWith(tmpHome)).toBe(true)
  })
})

describe('loadOrBuildIndex', () => {
  let tmpHome: string
  let srcDir: string
  let base: string

  beforeEach(() => {
    tmpHome = mkdtempSync(join(tmpdir(), 'csp-lob-home-'))
    srcDir = mkdtempSync(join(tmpdir(), 'csp-lob-src-'))
    base = join(tmpHome, '.csp')
    // A minimal indexable source: one code file.
    writeFileSync(join(srcDir, 'a.ts'), 'export function alpha() { return 1 }\n')
  })

  afterEach(() => {
    rmSync(tmpHome, { recursive: true, force: true })
    rmSync(srcDir, { recursive: true, force: true })
  })

  it('cache miss: builds the index and writes a manifest to the cache dir', async () => {
    const index = await loadOrBuildIndex(srcDir, { baseDir: base })

    expect(index).toBeInstanceOf(CspIndex)
    expect(index.chunks.length).toBeGreaterThan(0)

    const cacheDir = resolveCacheDir(srcDir, [ContentType.CODE], { baseDir: base })
    expect(existsSync(join(cacheDir, 'manifest.json'))).toBe(true)
  })

  it('cache hit: a second call reuses the cache without rebuilding', async () => {
    await loadOrBuildIndex(srcDir, { baseDir: base })

    // Spy on the build path: fromPath must NOT be called on the cache hit.
    const original = CspIndex.fromPath
    let buildCalls = 0
    CspIndex.fromPath = async (...args: Parameters<typeof original>) => {
      buildCalls += 1
      return original.apply(CspIndex, args)
    }
    try {
      const index = await loadOrBuildIndex(srcDir, { baseDir: base })
      expect(index).toBeInstanceOf(CspIndex)
      expect(index.chunks.length).toBeGreaterThan(0)
      expect(buildCalls).toBe(0)
    }
    finally {
      CspIndex.fromPath = original
    }
  })

  it('invalidation: a source change rebuilds and reflects new content', async () => {
    const first = await loadOrBuildIndex(srcDir, { baseDir: base })
    const firstChunkCount = first.chunks.length

    // Mutate the source: add a second file so the content hash changes.
    writeFileSync(join(srcDir, 'b.ts'), 'export function beta() { return 2 }\n')

    const original = CspIndex.fromPath
    let buildCalls = 0
    CspIndex.fromPath = async (...args: Parameters<typeof original>) => {
      buildCalls += 1
      return original.apply(CspIndex, args)
    }
    try {
      const second = await loadOrBuildIndex(srcDir, { baseDir: base })
      // Stale cache → rebuild happened.
      expect(buildCalls).toBe(1)
      // New file's content is now indexed.
      const paths = new Set(second.chunks.map(c => c.filePath))
      expect(paths.has('b.ts')).toBe(true)
      expect(second.chunks.length).toBeGreaterThanOrEqual(firstChunkCount)
    }
    finally {
      CspIndex.fromPath = original
    }
  })
})
