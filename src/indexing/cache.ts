// Global on-disk index cache location + content hashing (T009).
//
// The cache lives under `~/.csp/index/<key>/`, sharing the `~/.csp/` home that
// `stats.ts` already uses for `savings.jsonl`. This module covers the *pure*
// pieces of the caching model:
//   - `resolveCacheDir`   — deterministic cache directory for a (source,
//                           content, ref) triple.
//   - `computeContentHash`— order-independent hash of a file set's contents.
//   - `ensureCacheDir`    — create the `~/.csp` → `~/.csp/index` → leaf chain
//                           with 0700 permissions (NFR-003), tightening any
//                           pre-existing directory.
//
// The auto build/reuse orchestration (`loadOrBuildIndex`) lands in T010 and
// composes these primitives.

import type { ContentType } from '../types.ts'
import type { CspIndexFromGitOptions } from './index.ts'
import { createHash } from 'node:crypto'
import { chmodSync, existsSync, mkdirSync, readdirSync, realpathSync, rmSync } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import { homedir } from 'node:os'
import { basename, dirname, join, normalize, relative } from 'node:path'
import { isGitUrl } from '../utils.ts'
import { MAX_FILE_BYTES } from './create.ts'
import { walkFiles } from './file-walker.ts'
import { getExtensions } from './files.ts'
import { CspIndex, DEFAULT_CONTENT, parseManifest } from './index.ts'

/** Directory permissions for every cache directory (owner-only). NFR-003. */
const CACHE_DIR_MODE = 0o700

/** Length of the hex cache key kept from the full sha256 digest. */
const KEY_LENGTH = 32

/**
 * Options shared by the cache helpers. `baseDir` overrides the `~/.csp` home,
 * which keeps tests from touching the real user home — production callers omit
 * it and get `homedir()/.csp`.
 */
export interface CacheLocationOptions {
  /** Override for the `~/.csp` home directory (defaults to `homedir()/.csp`). */
  baseDir?: string
  /** Git ref (branch/tag/SHA) participating in the cache key, for `fromGit`. */
  ref?: string
}

/** A single file's identity for content hashing: relative path + raw content. */
export interface CacheFile {
  path: string
  content: string | Uint8Array
}

/** Resolve the `~/.csp` home, honoring an explicit `baseDir` override. */
function cacheHome(options: CacheLocationOptions): string {
  return options.baseDir ?? join(homedir(), '.csp')
}

/**
 * Resolve the cache directory for an indexed source.
 *
 * The key is a sha256 over the source identity, the (order-normalized) content
 * selection, and the optional git ref — so the same inputs always map to the
 * same directory, and a change in source / content / ref maps elsewhere. Local
 * paths are normalized so equivalent spellings collapse to one key; git URLs
 * are used verbatim (plus ref).
 *
 * @returns an absolute path of the form `<home>/index/<key>`.
 */
export function resolveCacheDir(
  source: string,
  content: readonly ContentType[],
  options: CacheLocationOptions = {},
): string {
  const sourceId = normalizeSource(source)
  // Sort content so selection ordering does not change the key.
  const contentKey = [...content].map(String).sort()
  const ref = options.ref ?? null

  const digest = createHash('sha256')
    .update(JSON.stringify({ sourceId, content: contentKey, ref }))
    .digest('hex')
    .slice(0, KEY_LENGTH)

  return join(cacheHome(options), 'index', digest)
}

/**
 * Resolve the root directory that holds every cached index, i.e. the parent of
 * all {@link resolveCacheDir} leaves. Returns `<home>/index`, reusing the same
 * `~/.csp` home (and `baseDir` override) as the rest of the cache helpers.
 *
 * This is the *only* directory `csp clear index` may remove — never the
 * `~/.csp` home itself (which also holds `savings.jsonl`).
 */
export function resolveIndexRoot(options: CacheLocationOptions = {}): string {
  return join(cacheHome(options), 'index')
}

/**
 * Compute a deterministic, order-independent content hash for a file set.
 *
 * Files are sorted by path, then each path and its content are folded into a
 * single sha256 in order. Equivalent string / `Uint8Array` content hashes
 * identically. The same file set in any order yields the same digest; a change
 * to any path or byte yields a different one.
 */
export function computeContentHash(files: readonly CacheFile[]): string {
  const sorted = [...files].sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
  const hash = createHash('sha256')
  for (const file of sorted) {
    // Length-prefix the path so path/content boundaries are unambiguous.
    hash.update(`${file.path.length}:${file.path}`)
    hash.update(toBytes(file.content))
  }
  return hash.digest('hex')
}

/**
 * Ensure the cache directory chain exists with 0700 permissions.
 *
 * Creates every directory from the `~/.csp` home down to `dir` (a leaf returned
 * by {@link resolveCacheDir}). A recursive `mkdir` only applies the mode to
 * directories it newly creates, so any pre-existing directory in the chain is
 * separately tightened with `chmod 0700` (NFR-003).
 */
export function ensureCacheDir(dir: string, options: CacheLocationOptions = {}): void {
  mkdirSync(dir, { recursive: true, mode: CACHE_DIR_MODE })
  for (const segment of chainTo(dir, cacheHome(options))) {
    chmodSync(segment, CACHE_DIR_MODE)
  }
}

/** Outcome of {@link clearIndexCache}: the targeted path, whether it was removed, and the entry count. */
export interface ClearIndexResult {
  /** The index root that was targeted (`<home>/index`). */
  path: string
  /** True when an existing index root was removed; false when none existed. */
  cleared: boolean
  /** Number of top-level cache entries removed (0 when nothing existed). */
  entries: number
}

/**
 * Remove the cached-index root (`<home>/index`) and report how many entries it
 * held. **Safety-critical (AC-015):** this deletes *only* the `index` directory
 * — never the `~/.csp` home or its `savings.jsonl`. The target is asserted to
 * end with the `index` segment and to differ from the home before any removal,
 * so a misconfigured `baseDir` cannot escalate into a home-wide rmtree.
 *
 * Returns `{ cleared: false, entries: 0 }` when no index root exists (not an
 * error — the CLI reports it as "No index cache found").
 */
export function clearIndexCache(options: CacheLocationOptions = {}): ClearIndexResult {
  const home = cacheHome(options)
  const indexRoot = resolveIndexRoot(options)

  if (!existsSync(indexRoot)) {
    return { path: indexRoot, cleared: false, entries: 0 }
  }

  // Resolve symlinks before the guard so a symlinked `index` (or home) cannot
  // redirect the delete outside the cache tree: rmSync follows the link and
  // would otherwise wipe the target's contents. realpath needs the path to
  // exist, which the existsSync above guarantees for indexRoot.
  const realIndexRoot = realpathSync(indexRoot)
  const realHome = existsSync(home) ? realpathSync(home) : normalize(home)

  // Guard: the (resolved) deletion target must be the **direct** `index` child
  // of the resolved home. Checking the parent (not just `basename === 'index'`)
  // also rejects a symlinked `index` that resolves to some *other* `.../index`
  // directory outside the cache home. If the invariant fails we delete nothing.
  if (basename(realIndexRoot) !== 'index' || normalize(dirname(realIndexRoot)) !== normalize(realHome)) {
    throw new Error(`Refusing to clear unsafe index path: ${realIndexRoot}`)
  }

  let entries = 0
  try {
    entries = readdirSync(realIndexRoot).length
  }
  catch {
    entries = 0
  }

  rmSync(realIndexRoot, { recursive: true, force: true })
  return { path: indexRoot, cleared: true, entries }
}

/**
 * Directories from the `~/.csp` home down to `leaf` (inclusive), ordered
 * home-first. When `leaf` is not under `home`, only `leaf` itself is returned
 * so we never chmod paths outside the cache tree.
 */
function chainTo(leaf: string, home: string): string[] {
  const normalizedHome = normalize(home)
  const segments: string[] = []
  let current = normalize(leaf)
  while (true) {
    segments.push(current)
    if (current === normalizedHome) {
      break
    }
    const parent = dirname(current)
    if (parent === current || !current.startsWith(normalizedHome)) {
      break
    }
    current = parent
  }
  return segments.reverse()
}

/** Normalize a source identity: local paths are path-normalized, URLs kept verbatim. */
function normalizeSource(source: string): string {
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(source) || source.startsWith('git@')) {
    return source
  }
  return normalize(source)
}

/** Coerce string / `Uint8Array` content to bytes for hashing. */
function toBytes(content: string | Uint8Array): Uint8Array {
  return typeof content === 'string' ? new TextEncoder().encode(content) : content
}

/** Options for {@link loadOrBuildIndex}. */
export interface LoadOrBuildOptions extends CacheLocationOptions {
  /** Content selection to index (defaults to {@link DEFAULT_CONTENT}). */
  content?: readonly ContentType[]
  /** Embedding model identifier forwarded to the build path. */
  modelPath?: string
}

/**
 * Collect the source files {@link CspIndex.fromPath} would index, as
 * {@link CacheFile} entries (relative path + raw content), for content hashing.
 *
 * Uses the same walk + extension resolution as `createIndexFromPath`: the
 * configured content selection drives `getExtensions`, `walkFiles` applies the
 * `.gitignore`/`.cspignore` + default-ignore rules, and over-large files are
 * skipped (matching the index's own `MAX_FILE_BYTES` cutoff). Paths are made
 * relative to `root` so the hash is stable across machines / mount points.
 */
async function collectSourceFiles(
  root: string,
  content: readonly ContentType[],
): Promise<CacheFile[]> {
  const extensions = getExtensions(content.map(c => c as `${ContentType}`), undefined)
  const files: CacheFile[] = []
  for await (const filePath of walkFiles(root, extensions)) {
    let size: number
    try {
      size = (await stat(filePath)).size
    }
    catch {
      continue
    }
    if (size > MAX_FILE_BYTES) {
      continue
    }
    let raw: string
    try {
      raw = await readFile(filePath, 'utf8')
    }
    catch {
      continue
    }
    files.push({ path: relative(root, filePath), content: raw })
  }
  return files
}

/**
 * Load a cached index for `source` if one exists and is still valid, otherwise
 * build it, persist it to the cache, and return it.
 *
 * Local paths: the live source file set is hashed ({@link computeContentHash})
 * and compared against the cached manifest's `contentHash`. A match means the
 * cache is fresh → reuse via {@link CspIndex.loadFromDisk}. A mismatch (the
 * source changed) invalidates the cache → rebuild and overwrite. The source
 * hash is injected into {@link CspIndex.save} so the manifest records a value
 * recomputed the same way on the next call.
 *
 * Git URLs (T009 STOP fallback): re-hashing a remote without a clone is not
 * possible, and a temp checkout's metadata makes a content hash
 * non-deterministic — so git sources are keyed by URL + ref alone
 * ({@link resolveCacheDir}). An existing cache for that key is reused; otherwise
 * the index is cloned, built, and saved (with the build-time content hash
 * recorded for transparency, not validation).
 */
export async function loadOrBuildIndex(
  source: string,
  options: LoadOrBuildOptions = {},
): Promise<CspIndex> {
  const content = options.content ?? DEFAULT_CONTENT
  const { baseDir, ref, modelPath } = options
  const isGit = isGitUrl(source)

  const locationOptions: CacheLocationOptions = {}
  if (baseDir !== undefined) {
    locationOptions.baseDir = baseDir
  }
  if (ref !== undefined) {
    locationOptions.ref = ref
  }

  const cacheDir = resolveCacheDir(source, content, locationOptions)
  ensureCacheDir(cacheDir, baseDir !== undefined ? { baseDir } : {})

  // The source-file hash is the cache-validity oracle for local paths; git
  // sources have no cheap live hash, so their key alone gates reuse.
  const sourceHash = isGit ? null : computeContentHash(await collectSourceFiles(source, content))

  const cached = await tryReuse(cacheDir, isGit, sourceHash)
  if (cached !== null) {
    return cached
  }

  const buildOptions: { ref?: string, modelPath?: string } = {}
  if (ref !== undefined) {
    buildOptions.ref = ref
  }
  if (modelPath !== undefined) {
    buildOptions.modelPath = modelPath
  }

  const index = await buildIndex(source, isGit, content, buildOptions)
  await index.save(cacheDir, sourceHash !== null ? { contentHash: sourceHash } : {})
  return index
}

/**
 * Reuse a cached index when present and valid, else `null`. For git sources a
 * present manifest is enough (URL+ref keyed); for local paths the manifest's
 * `contentHash` must equal the live `sourceHash`.
 */
async function tryReuse(
  cacheDir: string,
  isGit: boolean,
  sourceHash: string | null,
): Promise<CspIndex | null> {
  const manifestPath = join(cacheDir, 'manifest.json')
  if (!existsSync(manifestPath)) {
    return null
  }

  // For local sources, compare the content hash *before* the expensive full
  // load (chunks + bm25 + dense vectors + model). On a cache miss this skips
  // loading an index we are about to discard. Git sources are URL+ref keyed,
  // so a present manifest is sufficient.
  if (!isGit) {
    let manifest
    try {
      manifest = parseManifest(JSON.parse(await readFile(manifestPath, 'utf8')))
    }
    catch {
      // Corrupt/partial manifest — treat as a miss and rebuild.
      return null
    }
    if (manifest.contentHash !== sourceHash) {
      return null
    }
  }

  try {
    return await CspIndex.loadFromDisk(cacheDir)
  }
  catch {
    // Corrupt/partial cache entry — treat as a miss and rebuild.
    return null
  }
}

/** Build a fresh index from a local path or git URL. */
async function buildIndex(
  source: string,
  isGit: boolean,
  content: readonly ContentType[],
  options: { ref?: string, modelPath?: string },
): Promise<CspIndex> {
  if (isGit) {
    const gitOptions: CspIndexFromGitOptions = { content }
    if (options.ref !== undefined) {
      gitOptions.ref = options.ref
    }
    if (options.modelPath !== undefined) {
      gitOptions.modelPath = options.modelPath
    }
    return CspIndex.fromGit(source, gitOptions)
  }
  const fromPathOptions: { content: readonly ContentType[], modelPath?: string } = { content }
  if (options.modelPath !== undefined) {
    fromPathOptions.modelPath = options.modelPath
  }
  return CspIndex.fromPath(source, fromPathOptions)
}
