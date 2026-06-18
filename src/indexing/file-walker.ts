// Port of src/semble/index/file_walker.py
import { promises as fs } from 'node:fs'
import path from 'node:path'

// The `ignore` package provides gitignore-style pattern matching.
// We use it as a fast matcher, but we also keep a parallel list of
// `{ pattern, negated, hasExtSuffix }` entries to recreate the
// Python negation-with-extension bypass logic that the npm package
// does not expose.
//
// TODO(integration): use 'ignore' package once Unit 0 lands. Until then,
// the package is referenced via dynamic import below so the rest of the
// surface compiles even when the dep is missing from the lockfile.
// The `ignore` package is published as CommonJS with `export = ignore`, so
// `typeof import('ignore')` is the factory function itself (not a module object
// with a `.default`). Treat the imported type as the callable factory.
type IgnoreFactory = typeof import('ignore')
type IgnoreInstance = ReturnType<IgnoreFactory>

interface ParsedPattern {
  /** Pattern string as written in the gitignore file, without the leading "!" if any. */
  pattern: string
  /** True when the original line started with "!" (a negation pattern). */
  negated: boolean
  /** True when the pattern (with any trailing "/" stripped) has a file-extension suffix. */
  hasExtSuffix: boolean
  /** Per-pattern matcher (built from `ignore` package) used to test a single pattern. */
  matcher: IgnoreInstance
}

export interface IgnoreSpec {
  /** Base directory the patterns were sourced from. Paths are matched relative to this. */
  base: string
  /**
   * Aggregate ignore-package matcher containing every pattern in this spec.
   * Used as a fast pre-check via `.test()` in `_isIgnored`; the per-pattern
   * walk is only consulted when a negation pattern with an extension suffix
   * could win, so the bypass-extension-filter (`found`) decision can be made.
   */
  spec: IgnoreInstance
  /** Parsed pattern list (in source order) used for the negation-bypass logic. */
  patterns: readonly ParsedPattern[]
  /**
   * Pre-computed flag: true when at least one pattern in this spec is both
   * negated (`!`) and has a file-extension suffix. When false, `_isIgnored`
   * can skip the per-pattern walk after consulting the aggregate matcher.
   */
  hasNegatedExtPattern: boolean
}

/**
 * Default directories that are always ignored when walking. Trailing "/" matches
 * directory semantics (gitignore-style). The Python original uses ".semble/" —
 * for csp we replace it with ".csp/".
 */
export const DEFAULT_IGNORED_DIRS: ReadonlySet<string> = new Set([
  '.git/',
  '.hg/',
  '.svn/',
  '__pycache__/',
  'node_modules/',
  '.venv/',
  'venv/',
  '.tox/',
  '.mypy_cache/',
  '.pytest_cache/',
  '.ruff_cache/',
  '.cache/',
  '.csp/',
  '.next/',
  'dist/',
  'build/',
  '.eggs/',
])

let cachedIgnoreFactory: IgnoreFactory | undefined

/**
 * Resolve the `ignore` package factory lazily so this file can be imported even
 * when the dep is not yet installed in the worktree.
 */
async function getIgnoreFactory(): Promise<IgnoreFactory> {
  if (cachedIgnoreFactory) {
    return cachedIgnoreFactory
  }
  const mod = await import('ignore')
  // The CJS package exports the factory as the default export under ESM interop.
  const factory = ((mod as { default?: IgnoreFactory }).default
    ?? mod) as unknown as IgnoreFactory
  cachedIgnoreFactory = factory
  return factory
}

function hasExtensionSuffix(pattern: string): boolean {
  const stripped = pattern.replace(/\/+$/, '')
  return path.extname(stripped) !== ''
}

async function buildSpec(base: string, lines: readonly string[]): Promise<IgnoreSpec> {
  const factory = await getIgnoreFactory()
  const aggregate = factory({ allowRelativePaths: true })
  const patterns: ParsedPattern[] = []

  for (const rawLine of lines) {
    const line = rawLine.replace(/\r$/, '')
    const trimmed = line.trim()
    if (trimmed === '' || trimmed.startsWith('#')) {
      continue
    }

    aggregate.add(line)

    const negated = trimmed.startsWith('!')
    const pattern = negated ? trimmed.slice(1) : trimmed
    if (pattern === '') {
      continue
    }

    const matcher = factory({ allowRelativePaths: true }).add(pattern)
    patterns.push({
      pattern,
      negated,
      hasExtSuffix: hasExtensionSuffix(pattern),
      matcher,
    })
  }

  const hasNegatedExtPattern = patterns.some(p => p.negated && p.hasExtSuffix)

  return { base, spec: aggregate, patterns, hasNegatedExtPattern }
}

/**
 * Loads `.gitignore` and `.cspignore` from the given directory and merges them
 * into a single IgnoreSpec, or returns `null` when neither file is present.
 */
export async function _loadIgnoreForDir(directory: string): Promise<IgnoreSpec | null> {
  const gitignorePath = path.join(directory, '.gitignore')
  const cspignorePath = path.join(directory, '.cspignore')

  const lines: string[] = []
  for (const file of [gitignorePath, cspignorePath]) {
    try {
      const stat = await fs.stat(file)
      if (!stat.isFile()) {
        continue
      }
      const text = await fs.readFile(file, 'utf8')
      lines.push(...text.split(/\r?\n/))
    }
    catch {
      // missing file — fine
    }
  }

  if (lines.length === 0) {
    return null
  }
  return buildSpec(directory, lines)
}

/**
 * Result of `_isIgnored`. `ignored` is the final gitignore decision; `found`
 * signals that a negation pattern with a file-extension suffix matched, which
 * lets the file bypass the extension-allowlist filter (mirrors semble).
 */
export interface IgnoreCheck {
  ignored: boolean
  found: boolean
}

/**
 * Check whether a path is ignored by any of the provided ignore specs.
 *
 * Port of `_is_ignored` in semble. Each spec's patterns are checked in source
 * order; later matches override earlier ones (standard gitignore semantics).
 * When the *winning* match is a negation pattern with a file-extension suffix
 * (e.g. `!special.kjs`, `!*.py`), `found` becomes true so that the caller can
 * include the file even if its extension is not in the allowlist.
 *
 * Hot-path optimization: the aggregate `ignore`-package matcher is consulted
 * first via `.test()`. If no pattern in the spec matches at all, we carry the
 * outer state forward. If a pattern matches and the spec contains no negated
 * extension patterns, the answer is fully determined by the aggregate and the
 * per-pattern walk is skipped. The per-pattern walk runs only when a negation
 * could win AND the spec carries at least one negated extension pattern —
 * i.e. when `found` could change to `true`.
 */
export function _isIgnored(
  filePath: string,
  isDir: boolean,
  specs: readonly IgnoreSpec[],
): IgnoreCheck {
  let ignored = false
  let found = false

  for (const ignoreSpec of specs) {
    const relative = path.relative(ignoreSpec.base, filePath)
    if (relative === '' || relative.startsWith('..') || path.isAbsolute(relative)) {
      // Not under this spec's base — skip.
      continue
    }

    const posixRelative = relative.split(path.sep).join('/')
    const candidate = isDir ? `${posixRelative}/` : posixRelative

    let aggregateResult: { ignored: boolean, unignored: boolean }
    try {
      aggregateResult = ignoreSpec.spec.test(candidate)
    }
    catch {
      // The `ignore` package rejects a few edge cases (e.g. paths outside
      // the cwd when allowRelativePaths is off); treat as non-match.
      aggregateResult = { ignored: false, unignored: false }
    }

    const { ignored: isIgnoredBySpec, unignored: isUnignoredBySpec } = aggregateResult

    if (!isIgnoredBySpec && !isUnignoredBySpec) {
      // No pattern in this spec matched — preserve outer state.
      continue
    }

    if (isIgnoredBySpec) {
      // Winning pattern is a non-negated ignore. The original loop would set
      // `ignored = true; found = false` here regardless of pattern suffix.
      ignored = true
      found = false
      continue
    }

    // isUnignoredBySpec: a negation pattern won in this spec.
    if (!ignoreSpec.hasNegatedExtPattern) {
      // No negation pattern in this spec has an extension suffix, so `found`
      // cannot become true here.
      ignored = false
      found = false
      continue
    }

    // Fall back to the per-pattern walk to determine `found` accurately.
    for (const pattern of ignoreSpec.patterns) {
      let matched = false
      try {
        matched = pattern.matcher.ignores(candidate)
      }
      catch {
        matched = false
      }

      if (!matched) {
        continue
      }

      // Last winning pattern wins.
      ignored = !pattern.negated
      found = !ignored && pattern.hasExtSuffix
    }
  }

  return { ignored, found }
}

/**
 * Recursively walk `directory`, yielding files matching `extensions`. Hidden
 * directories are not implicitly skipped — the caller controls this via the
 * default-ignored set passed to `walkFiles`.
 */
export async function* _walk(
  directory: string,
  inheritedSpecs: readonly IgnoreSpec[],
  extensions: ReadonlySet<string>,
): AsyncIterable<string> {
  const dirSpec = await _loadIgnoreForDir(directory)
  const specs: readonly IgnoreSpec[] = dirSpec
    ? [...inheritedSpecs, dirSpec]
    : inheritedSpecs

  let entries: import('node:fs').Dirent[]
  try {
    entries = await fs.readdir(directory, { withFileTypes: true })
  }
  catch {
    return
  }

  entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))

  for (const entry of entries) {
    if (entry.isSymbolicLink()) {
      continue
    }
    const full = path.join(directory, entry.name)
    const isDir = entry.isDirectory()
    const { ignored, found } = _isIgnored(full, isDir, specs)
    if (ignored) {
      continue
    }

    if (isDir) {
      yield* _walk(full, specs, extensions)
    }
    else if (entry.isFile()) {
      if (found || extensions.has(path.extname(entry.name).toLowerCase())) {
        yield full
      }
    }
  }
}

/**
 * Yield files under `root` whose extension is in `extensions`, skipping ignored
 * paths. Default-ignored directories (see `DEFAULT_IGNORED_DIRS`) are always
 * skipped, plus any extra patterns in `ignore`. `.gitignore` / `.cspignore`
 * files encountered during traversal are honoured recursively.
 *
 * @param root Root directory to walk.
 * @param extensions Allowed file extensions (lowercase, including the leading dot).
 * @param ignore Additional gitignore-style patterns to ignore.
 */
export async function* walkFiles(
  root: string,
  extensions: readonly string[],
  ignore?: readonly string[],
): AsyncIterable<string> {
  const extensionsSet: ReadonlySet<string> = new Set(extensions.map(e => e.toLowerCase()))
  const dirPatterns: string[] = [
    ...[...DEFAULT_IGNORED_DIRS].sort(),
    ...(ignore ?? []),
  ]
  const baseSpec = await buildSpec(root, dirPatterns)
  yield* _walk(root, [baseSpec], extensionsSet)
}
