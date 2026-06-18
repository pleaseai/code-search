// Port of src/semble/mcp.py

import type { CspIndex } from '../indexing/index.ts'
import * as fs from 'node:fs/promises'

import * as path from 'node:path'
import process from 'node:process'
import { loadOrBuildIndex } from '../indexing/cache.ts'
import { loadModel } from '../indexing/index.ts'
import { ContentType } from '../types.ts'
import { formatResults, isGitUrl, resolveChunk } from '../utils.ts'
import { version } from '../version.ts'

const REPO_DESCRIPTION
  = 'https:// or http:// git URL (e.g. https://github.com/org/repo) or local directory path to index and search. '
    + 'Required when no default index was configured at startup. '
    + 'The index is cached after the first call, so repeat queries are fast.'

const CACHE_MAX_SIZE = 10 // Max number of cached indexes to keep in memory.

const SERVER_INSTRUCTIONS
  = 'Instant code search for any local or remote git repository. '
    + 'Call `search` to find relevant code; call `find_related` on a result to discover similar code elsewhere. '
    + 'When working in a local project, pass the project root as `repo`. '
    + 'For remote repos, pass an explicit https:// URL. Never guess or infer URLs. '
    + 'Prefer these tools over Grep, Glob, or Read for any question about how code works.'

/**
 * A deferred Promise — exposes its resolve/reject for use as a one-shot
 * readiness signal (the model-load latch in IndexCache).
 */
interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/** Resolve a local filesystem path to its canonical absolute form. */
async function resolvePath(p: string): Promise<string> {
  try {
    return await fs.realpath(p)
  }
  catch {
    return path.resolve(p)
  }
}

/**
 * Disk-cache seam: routes an in-memory cache miss through the shared
 * `~/.csp/index/<key>` disk cache. Mirrors the cli DI seam contract so cli and
 * mcp compute the same cache key for the same (source, content, ref) — see
 * `cli.ts`'s `_defaultLoadOrBuild`. Tests inject a stub to stay off the real
 * `~/.csp` home and the network.
 */
export type LoadOrBuildSeam = (
  source: string,
  opts: { content: ContentType[], ref?: string | undefined, modelPath?: string | undefined },
) => Promise<CspIndex>

export interface IndexCacheOptions {
  content?: ContentType[]
  /**
   * Override the disk-cache build path (defaults to {@link loadOrBuildIndex}).
   * Injected by tests to assert routing without touching `~/.csp` / network.
   */
  loadOrBuild?: LoadOrBuildSeam
}

/**
 * Default disk-cache seam: forward to {@link loadOrBuildIndex}, re-narrowing
 * `ref` so an absent ref is omitted rather than passed as explicit `undefined`
 * (required under `exactOptionalPropertyTypes`). Identical to cli's
 * `_defaultLoadOrBuild` so both layers key the cache the same way.
 */
async function defaultLoadOrBuild(
  source: string,
  opts: { content: ContentType[], ref?: string | undefined, modelPath?: string | undefined },
): Promise<CspIndex> {
  return loadOrBuildIndex(source, {
    content: opts.content,
    ...(opts.ref !== undefined ? { ref: opts.ref } : {}),
    ...(opts.modelPath !== undefined ? { modelPath: opts.modelPath } : {}),
  })
}

/**
 * Cache of indexed repos and local paths for the lifetime of the MCP server
 * process. LRU-bounded (10 entries) and deduplicates concurrent requests via
 * Promise caching.
 */
export class IndexCache {
  // Use a Map for insertion-order semantics (LRU via re-insert).
  private readonly tasks = new Map<string, Promise<CspIndex>>()
  private readonly content: ContentType[]
  private readonly loadOrBuild: LoadOrBuildSeam
  private readonly modelReady: Deferred<string>
  private modelPath: string | null = null
  private modelError: unknown = null
  private modelLoadStarted = false
  private watcherClose: (() => Promise<void>) | null = null

  constructor(options: IndexCacheOptions = {}) {
    this.content = options.content ?? [ContentType.CODE]
    this.loadOrBuild = options.loadOrBuild ?? defaultLoadOrBuild
    this.modelReady = createDeferred<string>()
    // Prevent unhandled promise rejection warnings if the model fails to load
    // before any caller awaits the promise. Callers of awaitModel() still
    // observe the rejection because they await the same promise themselves.
    this.modelReady.promise.catch(() => {})
  }

  /**
   * Begin loading the embedding model (idempotent). Call from `serve` to
   * run model load in parallel with starting the server. If never called
   * explicitly, the first `get()` will trigger it.
   */
  ensureModelLoading(): void {
    if (this.modelLoadStarted) {
      return
    }
    this.modelLoadStarted = true
    void (async () => {
      try {
        const [, modelPath] = await loadModel()
        this.modelPath = modelPath
        this.modelReady.resolve(modelPath)
      }
      catch (err) {
        this.modelError = err
        this.modelReady.reject(err)
      }
    })()
  }

  private async awaitModel(): Promise<string> {
    this.ensureModelLoading()
    if (this.modelError !== null) {
      throw this.modelError
    }
    return this.modelReady.promise
  }

  private async computeCacheKey(source: string, ref?: string): Promise<string> {
    if (isGitUrl(source)) {
      return ref !== undefined && ref !== '' ? `${source}@${ref}` : source
    }
    return resolvePath(source)
  }

  /**
   * Return an index for the requested source, building and caching it on
   * first access. Concurrent calls for the same key share a single Promise.
   */
  async get(source: string, ref?: string): Promise<CspIndex> {
    const cacheKey = await this.computeCacheKey(source, ref)

    const existing = this.tasks.get(cacheKey)
    if (existing !== undefined) {
      // Touch for LRU (move to most-recent end).
      this.tasks.delete(cacheKey)
      this.tasks.set(cacheKey, existing)
      try {
        return await existing
      }
      catch (err) {
        // Only evict if this task hasn't already been replaced.
        if (this.tasks.get(cacheKey) === existing) {
          this.tasks.delete(cacheKey)
        }
        throw err
      }
    }

    const modelPath = await this.awaitModel()

    // Re-check after the await: another caller may have populated the entry.
    const racedExisting = this.tasks.get(cacheKey)
    if (racedExisting !== undefined) {
      this.tasks.delete(cacheKey)
      this.tasks.set(cacheKey, racedExisting)
      return racedExisting
    }

    // LRU eviction: drop oldest entry (first inserted).
    if (this.tasks.size >= CACHE_MAX_SIZE) {
      const oldestKey = this.tasks.keys().next().value
      if (oldestKey !== undefined) {
        this.tasks.delete(oldestKey)
      }
    }

    // Route the in-memory miss through the shared disk cache. The seam owns the
    // `isGitUrl` branch and the `~/.csp/index/<key>` content-hash reuse/rebuild;
    // we only hand it the (source, content, ref) and the pre-warmed modelPath.
    // `ref` / `modelPath` are omitted when absent to satisfy
    // `exactOptionalPropertyTypes` and to match cli's cache-key contract.
    const buildPromise: Promise<CspIndex> = this.loadOrBuild(source, {
      content: this.content,
      ...(ref !== undefined ? { ref } : {}),
      ...(modelPath !== undefined ? { modelPath } : {}),
    })

    this.tasks.set(cacheKey, buildPromise)

    try {
      return await buildPromise
    }
    catch (err) {
      // Only evict if this task hasn't already been replaced.
      if (this.tasks.get(cacheKey) === buildPromise) {
        this.tasks.delete(cacheKey)
      }
      throw err
    }
  }

  /**
   * Remove the cached entry for `source`. Awaitable so callers (notably the
   * file watcher) can guarantee the deletion lands before the next `get()`.
   */
  async evict(source: string): Promise<void> {
    const cacheKey = await this.computeCacheKey(source)
    this.tasks.delete(cacheKey)
  }

  /** Number of currently cached entries (for tests / introspection). */
  get size(): number {
    return this.tasks.size
  }

  /**
   * Start a background watcher that evicts + re-gets the index whenever
   * files at `path` change. Uses chokidar (debounced).
   *
   * Calling this more than once stops the previous watcher first to avoid
   * leaking file handles.
   */
  async startWatcher(watchPath: string): Promise<void> {
    // Stop any existing watcher before installing a new one.
    await this.stopWatcher()

    interface ChokidarWatcher {
      on: (event: string, cb: () => void) => void
      close: () => Promise<void>
    }
    interface ChokidarModule {
      watch: (
        watchPath: string,
        opts: { ignoreInitial: boolean, persistent: boolean },
      ) => ChokidarWatcher
    }
    let chokidar: ChokidarModule
    try {
      // Resolve lazily so the module loads even when chokidar is absent.
      const mod = (await import('chokidar')) as { default?: ChokidarModule } & ChokidarModule
      chokidar = mod.default ?? mod
    }
    catch {
      // chokidar not installed — silently no-op so callers that don't need
      // watching still work.
      return
    }

    // Match semble: watch everything. Upstream relies on the underlying
    // walker's .gitignore handling to filter what actually ends up in the
    // index; the watcher itself doesn't filter, so projects rooted inside a
    // dotfile directory (e.g. ~/.config/proj) still re-index correctly.
    const watcher = chokidar.watch(watchPath, {
      ignoreInitial: true,
      persistent: true,
    })

    let debounce: ReturnType<typeof setTimeout> | null = null
    const onChange = (): void => {
      if (debounce !== null) {
        clearTimeout(debounce)
      }
      debounce = setTimeout(() => {
        debounce = null
        // Await evict before get so the rebuild sees a fresh cache slot.
        void (async () => {
          try {
            await this.evict(watchPath)
            await this.get(watchPath)
          }
          catch {
            // Swallow rebuild errors; the next explicit get() will surface them.
          }
        })()
      }, 250)
    }

    watcher.on('add', onChange)
    watcher.on('change', onChange)
    watcher.on('unlink', onChange)
    watcher.on('addDir', onChange)
    watcher.on('unlinkDir', onChange)

    this.watcherClose = async () => {
      if (debounce !== null) {
        clearTimeout(debounce)
      }
      await watcher.close()
    }
  }

  /** Stop the file watcher, if any. */
  async stopWatcher(): Promise<void> {
    if (this.watcherClose !== null) {
      const close = this.watcherClose
      this.watcherClose = null
      await close()
    }
  }
}

/**
 * Return a cached index for a repo, rejecting unsafe git transport schemes
 * and missing-source cases with descriptive errors.
 */
async function getIndex(
  repo: string | undefined,
  defaultSource: string | undefined,
  cache: IndexCache,
): Promise<CspIndex> {
  if (
    repo !== undefined
    && isGitUrl(repo)
    && !repo.startsWith('https://')
    && !repo.startsWith('http://')
  ) {
    throw new Error(
      `Only https://, http://, or local directory paths are accepted as \`repo\`. Got: ${JSON.stringify(repo)}`,
    )
  }
  const source = repo ?? defaultSource
  if (source === undefined || source === '') {
    throw new Error(
      'No repo specified and no default index. '
      + 'Pass an https:// or http:// git URL or local directory path as `repo`.',
    )
  }
  try {
    return await cache.get(source)
  }
  catch (exc) {
    const msg = exc instanceof Error ? exc.message : String(exc)
    throw new Error(`Failed to index ${JSON.stringify(source)}: ${msg}`)
  }
}

// Exported for tests so they can exercise the safety branches without the SDK.
export const _internal = { getIndex }

/** Configured MCP server (typed loosely so we don't depend on the SDK at compile time). */
export interface CspMcpServer {
  /** Tool registry — exposed for test/introspection. */
  readonly tools: ReadonlyMap<string, ToolDef>
  /** True when the real `@modelcontextprotocol/sdk` server backs this object. */
  readonly isPlaceholder: boolean
  /** Connect to a transport (no-op for the placeholder). */
  connect: (transport: unknown) => Promise<void>
  /** Underlying SDK server, if any. */
  readonly underlying: unknown
}

interface ToolDef {
  title: string
  description: string
  handler: (args: Record<string, unknown>) => Promise<string>
}

/**
 * Build and return a configured MCP server backed by the given cache.
 *
 * If `@modelcontextprotocol/sdk` is installed, this registers `search` and
 * `find_related` tools on a real `McpServer`. If it isn't (yet), a
 * placeholder is returned so the rest of the module remains usable and
 * testable.
 */
export async function createServer(
  cache: IndexCache,
  defaultSource?: string,
): Promise<CspMcpServer> {
  const searchTool: ToolDef = {
    title: 'Search a codebase with a natural-language or code query.',
    description:
      'Pass a git URL or local path as `repo` to index it on demand; indexes are cached for the session. '
      + 'Use this to find where something is implemented, understand a library, or locate related code.',
    handler: async (args) => {
      try {
        const query = String(args.query ?? '')
        const repo = args.repo === undefined ? undefined : String(args.repo)
        const topK
          = typeof args.top_k === 'number'
            ? args.top_k
            : typeof args.topK === 'number'
              ? args.topK
              : 5

        const index = await getIndex(repo, defaultSource, cache)
        const results = index.search(query, { topK })
        if (results.length === 0) {
          return JSON.stringify({ error: 'No results found.' })
        }
        return JSON.stringify(formatResults(query, results))
      }
      catch (err) {
        return err instanceof Error ? err.message : String(err)
      }
    },
  }

  const findRelatedTool: ToolDef = {
    title: 'Find code chunks semantically similar to a specific location in a file.',
    description:
      'Use after `search` to explore related implementations or callers. '
      + 'Pass file_path and line from a prior search result.',
    handler: async (args) => {
      try {
        const filePath = String(args.file_path ?? args.filePath ?? '')
        const line = Number(args.line ?? 0)
        const repo = args.repo === undefined ? undefined : String(args.repo)
        const topK
          = typeof args.top_k === 'number'
            ? args.top_k
            : typeof args.topK === 'number'
              ? args.topK
              : 5

        const index = await getIndex(repo, defaultSource, cache)
        const chunk = resolveChunk(index.chunks, filePath, line)
        if (chunk === null) {
          return (
            `No chunk found at ${filePath}:${line}. `
            + 'Make sure the file is indexed and the line number is within a known chunk.'
          )
        }
        const results = index.findRelated(chunk, { topK })
        if (results.length === 0) {
          return JSON.stringify({
            error: `No related chunks found for ${filePath}:${line}.`,
          })
        }
        return JSON.stringify(
          formatResults(`Chunks related to ${filePath}:${line}`, results),
        )
      }
      catch (err) {
        return err instanceof Error ? err.message : String(err)
      }
    },
  }

  const tools = new Map<string, ToolDef>([
    ['search', searchTool],
    ['find_related', findRelatedTool],
  ])

  // Try to wire up the real MCP SDK; fall back to a placeholder if it's not
  // installed (per the unit spec — Unit 0 may not be merged yet).
  type McpServerCtor = new (
    info: { name: string, version?: string },
    options?: { instructions?: string },
  ) => McpServerInstance
  interface McpServerInstance {
    registerTool: (
      name: string,
      config: { title: string, description: string, inputSchema?: unknown },
      handler: (args: Record<string, unknown>) => Promise<unknown>,
    ) => void
    connect: (transport: unknown) => Promise<void>
  }
  let McpServer: McpServerCtor | null = null
  try {
    const mod = (await import('@modelcontextprotocol/sdk/server/mcp.js')) as {
      McpServer: McpServerCtor
    }
    McpServer = mod.McpServer
  }
  catch {
    McpServer = null
  }

  if (McpServer === null) {
    return {
      tools,
      isPlaceholder: true,
      connect: async () => {
        throw new Error(
          '@modelcontextprotocol/sdk is not installed; createServer returned a placeholder.',
        )
      },
      underlying: null,
    }
  }

  const underlying = new McpServer(
    { name: 'csp', version },
    { instructions: SERVER_INSTRUCTIONS },
  )

  // The MCP SDK's `registerTool` `inputSchema` expects a Zod raw shape
  // (`Record<string, ZodSchema>`), not raw JSON Schema. zod is a transitive
  // dependency of @modelcontextprotocol/sdk, so if the SDK loaded we should
  // be able to load zod too. If it isn't reachable for any reason, fall back
  // to registering the tool without an input schema so it's still callable.
  interface ZodLikeSchema {
    optional: () => ZodLikeSchema
    describe: (desc: string) => ZodLikeSchema
    default: (value: unknown) => ZodLikeSchema
  }
  interface ZodLikeModule {
    string: () => ZodLikeSchema
    number: () => ZodLikeSchema
  }
  let z: ZodLikeModule | null = null
  try {
    const zmod = (await import('zod')) as { z?: ZodLikeModule } & ZodLikeModule
    z = zmod.z ?? zmod
  }
  catch {
    z = null
  }

  const searchSchema = z === null
    ? undefined
    : {
        query: z.string().describe('Natural language or code query.'),
        repo: z.string().describe(REPO_DESCRIPTION).optional(),
        top_k: z.number().describe('Number of results to return.').default(5),
      }

  const findRelatedSchema = z === null
    ? undefined
    : {
        file_path: z
          .string()
          .describe(
            'Path to the file as stored in the index (use file_path from a search result).',
          ),
        line: z.number().describe('Line number (1-indexed).'),
        repo: z.string().describe(REPO_DESCRIPTION).optional(),
        top_k: z.number().describe('Number of similar chunks to return.').default(5),
      }

  underlying.registerTool(
    'search',
    {
      title: searchTool.title,
      description: searchTool.description,
      ...(searchSchema !== undefined ? { inputSchema: searchSchema } : {}),
    },
    async args => ({
      content: [{ type: 'text', text: await searchTool.handler(args) }],
    }),
  )

  underlying.registerTool(
    'find_related',
    {
      title: findRelatedTool.title,
      description: findRelatedTool.description,
      ...(findRelatedSchema !== undefined ? { inputSchema: findRelatedSchema } : {}),
    },
    async args => ({
      content: [
        { type: 'text', text: await findRelatedTool.handler(args) },
      ],
    }),
  )

  return {
    tools,
    isPlaceholder: false,
    connect: async transport => underlying.connect(transport),
    underlying,
  }
}

export interface ServeOptions {
  ref?: string | undefined
  content?: ContentType[]
}

/**
 * Start an MCP stdio server, optionally pre-indexing a default source.
 *
 * Pre-warms the embedding model in parallel with starting the server and
 * starts a file watcher for local paths.
 */
export async function serve(path?: string, options: ServeOptions = {}): Promise<void> {
  const cache = new IndexCache({ content: options.content ?? [ContentType.CODE] })

  // Kick off model load + optional pre-index in parallel with server startup.
  const prewarm = (async (): Promise<void> => {
    try {
      cache.ensureModelLoading()
      // Wait for the model load to settle before pre-indexing.
      // awaitModel is private; ensure the model is ready by triggering and
      // catching get() — which itself awaits the model.
      if (path !== undefined && path !== '') {
        try {
          await cache.get(path, options.ref)
        }
        catch {
          // Pre-indexing failure shouldn't crash the server.
        }
        if (!isGitUrl(path)) {
          try {
            await cache.startWatcher(path)
          }
          catch {
            // Watcher failure is non-fatal.
          }
        }
      }
    }
    catch {
      // Already logged via modelError; the server can still report errors per-call.
    }
  })()

  const server = await createServer(cache, path)

  // Attempt to attach stdio transport from the SDK; if not available, log and exit cleanly.
  let StdioTransportCtor:
    | (new () => { close?: () => Promise<void> | void })
    | null = null
  try {
    const mod = (await import('@modelcontextprotocol/sdk/server/stdio.js')) as {
      StdioServerTransport: new () => { close?: () => Promise<void> | void }
    }
    StdioTransportCtor = mod.StdioServerTransport
  }
  catch {
    StdioTransportCtor = null
  }

  if (StdioTransportCtor === null || server.isPlaceholder) {
    // No SDK — nothing to serve. Await pre-warm so callers can inspect the
    // cache, then tear down the watcher so this path doesn't leak file
    // handles (the prewarm above may have started one).
    try {
      await prewarm
    }
    finally {
      await cache.stopWatcher()
    }
    return
  }

  // Hook into stdin EOF so we can return once the client disconnects, mirroring
  // semble's `run_stdio_async()` blocking semantics. Both listeners share a
  // single cleanup so whichever event fires first removes the other —
  // otherwise repeated `serve()` calls (tests, restarts) accumulate listeners
  // on `process.stdin` and trip MaxListenersExceededWarning.
  const stdinClosed = new Promise<void>((resolve) => {
    const cleanup = (): void => {
      process.stdin.removeListener('end', cleanup)
      process.stdin.removeListener('close', cleanup)
      resolve()
    }
    process.stdin.on('end', cleanup)
    process.stdin.on('close', cleanup)
  })

  const transport = new StdioTransportCtor()
  try {
    // connect() must be inside the try so a failure here still runs the
    // transport/watcher cleanup below.
    await server.connect(transport)
    // Block on stdin close — connect() returns immediately after handshake,
    // and we MUST NOT close the transport until the client disconnects.
    await stdinClosed
    // After the client disconnects, drain any pre-warm work that's still in
    // flight so we don't orphan promises.
    await prewarm
  }
  finally {
    if (typeof transport.close === 'function') {
      await transport.close()
    }
    await cache.stopWatcher()
  }
}
