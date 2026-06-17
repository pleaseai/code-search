import { afterAll, beforeEach, describe, expect, it } from 'bun:test'
import { _internal, createServer, IndexCache } from './server.ts'
import { ContentType } from '../types.ts'
import * as indexing from '../indexing/index.ts'
import { CspIndex } from '../indexing/index.ts'
import { makeStubModel, SelectableBasicBackend } from '../indexing/dense.ts'
import { Bm25Index } from '../indexing/sparse.ts'

// We intercept CspIndex.fromPath/fromGit by reassigning the static methods on
// the *real* class object (the same reference server.ts imports) rather than
// `mock.module`. Bun's `mock.module` mutates the process-wide module registry
// irreversibly — it would leak the stub into sibling test files (notably
// ../indexing/index.test.ts) that exercise the genuine CspIndex. Static-method
// reassignment is plain property mutation, so `afterAll` can restore it.
let fromPathCalls = 0
let fromGitCalls = 0
let fromPathImpl: () => Promise<CspIndex> = async () => makeIndex()
let fromGitImpl: () => Promise<CspIndex> = async () => makeIndex()

// A real, empty CspIndex instance: `instanceof CspIndex` holds and `search`
// returns [] for an empty index, matching what these tests assert.
function makeIndex(chunks: CspIndex['chunks'] = []): CspIndex {
  const vectors = chunks.map(() => new Float32Array(4))
  return new CspIndex({
    model: makeStubModel(4),
    bm25Index: Bm25Index.build(chunks.map(() => ['x'])),
    semanticIndex: new SelectableBasicBackend(vectors),
    chunks,
    modelPath: '/tmp/fake-model',
    root: null,
    content: [ContentType.CODE],
  })
}

const realFromPath = CspIndex.fromPath
const realFromGit = CspIndex.fromGit

CspIndex.fromPath = async (..._args: Parameters<typeof realFromPath>): Promise<CspIndex> => {
  fromPathCalls++
  return fromPathImpl()
}
CspIndex.fromGit = async (..._args: Parameters<typeof realFromGit>): Promise<CspIndex> => {
  fromGitCalls++
  return fromGitImpl()
}

afterAll(() => {
  // Restore the genuine static methods so later test files see real behavior.
  CspIndex.fromPath = realFromPath
  CspIndex.fromGit = realFromGit
})

beforeEach(() => {
  fromPathCalls = 0
  fromGitCalls = 0
  fromPathImpl = async () => makeIndex()
  fromGitImpl = async () => makeIndex()
})

describe('IndexCache', () => {
  it('caches results — second call returns the cached value', async () => {
    const cache = new IndexCache({ content: [ContentType.CODE] })
    const first = await cache.get('/tmp/some-repo')
    const second = await cache.get('/tmp/some-repo')
    expect(second).toBe(first)
    expect(fromPathCalls).toBe(1)
  })

  it('deduplicates concurrent get() for the same source', async () => {
    const cache = new IndexCache()
    const [a, b] = await Promise.all([
      cache.get('/tmp/dedup-repo'),
      cache.get('/tmp/dedup-repo'),
    ])
    expect(a).toBe(b)
    expect(fromPathCalls).toBe(1)
  })

  it('evict() removes the cached entry so the next get() rebuilds', async () => {
    const cache = new IndexCache()
    await cache.get('/tmp/repo-to-evict')
    expect(fromPathCalls).toBe(1)

    await cache.evict('/tmp/repo-to-evict')

    await cache.get('/tmp/repo-to-evict')
    expect(fromPathCalls).toBe(2)
  })

  it('LRU: the 11th distinct source evicts the oldest', async () => {
    const cache = new IndexCache()
    for (let i = 0; i < 10; i++)
      await cache.get(`/tmp/lru-${i}`)
    expect(cache.size).toBe(10)

    await cache.get('/tmp/lru-10')
    expect(cache.size).toBe(10)

    // /tmp/lru-0 was the oldest and should have been evicted — refetch triggers rebuild.
    const before = fromPathCalls
    await cache.get('/tmp/lru-0')
    expect(fromPathCalls).toBe(before + 1)
  })

  it('treats git URLs differently from local paths', async () => {
    const cache = new IndexCache()
    await cache.get('https://github.com/org/repo')
    expect(fromGitCalls).toBe(1)
    expect(fromPathCalls).toBe(0)

    await cache.get('/tmp/local-path')
    expect(fromPathCalls).toBe(1)
  })

  it('evict() awaitably blocks until the cache entry is gone', async () => {
    const cache = new IndexCache()
    await cache.get('/tmp/await-evict')
    expect(cache.size).toBe(1)
    await cache.evict('/tmp/await-evict')
    expect(cache.size).toBe(0)
  })

  it('failed get() does not poison the cache entry', async () => {
    fromPathImpl = async () => {
      throw new Error('boom')
    }

    const cache = new IndexCache()
    await expect(cache.get('/tmp/will-fail')).rejects.toThrow('boom')

    // After failure, the next call retries.
    fromPathImpl = async () => makeIndex()
    await expect(cache.get('/tmp/will-fail')).resolves.toBeInstanceOf(indexing.CspIndex)
  })
})

describe('getIndex (safety layer)', () => {
  it('rejects ssh:// git URLs', async () => {
    const cache = new IndexCache()
    await expect(
      _internal.getIndex('ssh://git@github.com/org/repo.git', undefined, cache),
    ).rejects.toThrow(/Only https:\/\/, http:\/\//)
  })

  it('rejects git:// git URLs', async () => {
    const cache = new IndexCache()
    await expect(
      _internal.getIndex('git://github.com/org/repo.git', undefined, cache),
    ).rejects.toThrow(/Only https:\/\/, http:\/\//)
  })

  it('rejects file:// pseudo-URLs', async () => {
    const cache = new IndexCache()
    await expect(
      _internal.getIndex('file:///tmp/whatever', undefined, cache),
    ).rejects.toThrow(/Only https:\/\/, http:\/\//)
  })

  it('rejects when repo and defaultSource are both undefined', async () => {
    const cache = new IndexCache()
    await expect(_internal.getIndex(undefined, undefined, cache)).rejects.toThrow(
      /No repo specified/,
    )
  })

  it('falls back to defaultSource when repo is undefined', async () => {
    const cache = new IndexCache()
    const result = await _internal.getIndex(undefined, '/tmp/default-repo', cache)
    expect(result).toBeInstanceOf(indexing.CspIndex)
    expect(fromPathCalls).toBe(1)
  })

  it('accepts https:// git URLs', async () => {
    const cache = new IndexCache()
    const result = await _internal.getIndex(
      'https://github.com/org/repo',
      undefined,
      cache,
    )
    expect(result).toBeInstanceOf(indexing.CspIndex)
    expect(fromGitCalls).toBe(1)
  })

  it('wraps underlying index errors in a descriptive message', async () => {
    fromPathImpl = async () => {
      throw new Error('disk full')
    }
    const cache = new IndexCache()
    await expect(_internal.getIndex('/tmp/bad', undefined, cache)).rejects.toThrow(
      /Failed to index .*disk full/,
    )
  })
})

describe('createServer', () => {
  it('returns a server object exposing `search` and `find_related` tools', async () => {
    const cache = new IndexCache()
    const server = await createServer(cache, '/tmp/default')

    expect(server.tools.has('search')).toBe(true)
    expect(server.tools.has('find_related')).toBe(true)

    const searchTool = server.tools.get('search')!
    expect(searchTool.title).toBe(
      'Search a codebase with a natural-language or code query.',
    )

    const findRelatedTool = server.tools.get('find_related')!
    expect(findRelatedTool.title).toBe(
      'Find code chunks semantically similar to a specific location in a file.',
    )
  })

  it('`search` handler returns "No results" JSON when the index yields nothing', async () => {
    const cache = new IndexCache()
    const server = await createServer(cache, '/tmp/default')
    const searchTool = server.tools.get('search')!
    const out = await searchTool.handler({ query: 'foo' })
    expect(JSON.parse(out)).toEqual({ error: 'No results found.' })
  })

  it('`search` handler surfaces safety errors as plain strings', async () => {
    const cache = new IndexCache()
    const server = await createServer(cache) // no defaultSource
    const searchTool = server.tools.get('search')!
    const out = await searchTool.handler({ query: 'foo' }) // no repo either
    expect(out).toMatch(/No repo specified/)
  })

  it('`search` handler rejects ssh:// git URLs as a plain-string error', async () => {
    const cache = new IndexCache()
    const server = await createServer(cache)
    const searchTool = server.tools.get('search')!
    const out = await searchTool.handler({
      query: 'foo',
      repo: 'ssh://git@github.com/org/repo',
    })
    expect(out).toMatch(/Only https:\/\/, http:\/\//)
  })

  it('`find_related` handler returns a helpful message when the chunk is missing', async () => {
    const cache = new IndexCache()
    const server = await createServer(cache, '/tmp/default')
    const tool = server.tools.get('find_related')!
    const out = await tool.handler({ file_path: 'nope.ts', line: 42 })
    expect(out).toMatch(/No chunk found at nope.ts:42/)
  })
})
