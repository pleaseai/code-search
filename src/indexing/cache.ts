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

import { createHash } from 'node:crypto'
import { chmodSync, mkdirSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join, normalize } from 'node:path'
import type { ContentType } from '../types.ts'

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
  for (const segment of chainTo(dir, cacheHome(options)))
    chmodSync(segment, CACHE_DIR_MODE)
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
    if (current === normalizedHome)
      break
    const parent = dirname(current)
    if (parent === current || !current.startsWith(normalizedHome))
      break
    current = parent
  }
  return segments.reverse()
}

/** Normalize a source identity: local paths are path-normalized, URLs kept verbatim. */
function normalizeSource(source: string): string {
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(source) || source.startsWith('git@'))
    return source
  return normalize(source)
}

/** Coerce string / `Uint8Array` content to bytes for hashing. */
function toBytes(content: string | Uint8Array): Uint8Array {
  return typeof content === 'string' ? new TextEncoder().encode(content) : content
}
