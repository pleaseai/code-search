import { beforeEach, describe, expect, it, mock } from 'bun:test'

// Mock the indexing module so we can control CspIndex.fromPath/fromGit and
// loadModel without spinning up real embeddings.
let fromPathCalls = 0
let fromGitCalls = 0
let fromPathImpl: () => Promise<unknown> = async () => makeIndex()
let fromGitImpl: () => Promise<unknown> = async () => makeIndex()

let makeIndex: () => FakeIndex = () => new FakeIndex([])

class FakeIndex {
  readonly chunks: Array<{
    content: string
    filePath: string
    startLine: number
    endLine: number
  }>

  constructor(chunks: FakeIndex['chunks'] = []) {
    this.chunks = chunks
  }

  search(_q: string, _opts?: { topK?: number }): Array<{
    chunk: FakeIndex['chunks'][number]
    score: number
    toDict: () => Record<string, unknown>
  }> {
    return []
  }

  findRelated(_c: FakeIndex['chunks'][number], _opts?: { topK?: number }): Array<{
    chunk: FakeIndex['chunks'][number]
    score: number
    toDict: () => Record<string, unknown>
  }> {
    return []
  }
}

class MockedCspIndex extends FakeIndex {
  static async fromPath(..._args: unknown[]): Promise<FakeIndex> {
    fromPathCalls++
    return fromPathImpl() as Promise<FakeIndex>
  }

  static async fromGit(..._args: unknown[]): Promise<FakeIndex> {
    fromGitCalls++
    return fromGitImpl() as Promise<FakeIndex>
  }
}

// Wire makeIndex to return instances of the mocked class so instanceof checks
// in the tests pass.
makeIndex = () => new MockedCspIndex([])

await mock.module('../indexing/index.ts', () => ({
  CspIndex: MockedCspIndex,
  loadModel: async (): Promise<[unknown, string]> => [null, '/tmp/fake-model'],
}))

// Import AFTER mocking so server.ts picks up the mocked module.
const { _internal, createServer, IndexCache } = await import('./server.ts')
const { ContentType } = await import('../types.ts')
const indexing = await import('../indexing/index.ts')

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
