// Port of src/semble/ranking/boosting.py

// TODO(integration): replace inline Chunk type with `import type { Chunk } from '../types.ts'`
//                    once Unit 1 lands in main.
export interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string
}

// TODO(integration): replace with import from '../tokens.ts' once Unit 2 lands in main.
const TOKEN_CAMEL_RE = /[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+/g

function splitIdentifier(token: string): string[] {
  const lower = token.toLowerCase()
  let parts: string[] = []

  if (token.includes('_')) {
    parts = lower.split('_').filter(p => p.length > 0)
  }
  else {
    parts = (token.match(TOKEN_CAMEL_RE) ?? []).map(m => m.toLowerCase())
  }

  if (parts.length >= 2) {
    return [lower, ...parts]
  }
  return [lower]
}

// Symbol-lookup queries: namespace-qualified, leading-underscore, or containing
// uppercase/underscore. Plain lowercase words (e.g. "session") are NL, not symbols.
export const SYMBOL_QUERY_RE = /^(?:[A-Z_a-z]\w*(?:(?:::|\\|->|\.)[A-Z_a-z]\w*)+|_\w*|[A-Za-z][A-Za-z0-9]*[A-Z_]\w*|[A-Z][A-Za-z0-9]*)$/

// CamelCase/camelCase identifiers embedded in a NL query; excludes plain words and pure acronyms.
export const EMBEDDED_SYMBOL_RE = /\b(?:[A-Z][a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*|[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]+)\b/g

// Minimum stem length for prefix-based non-candidate scan (avoids over-broad matches).
export const EMBEDDED_STEM_MIN_LEN = 4

// Half-strength: the symbol may be incidental to the NL query.
export const EMBEDDED_SYMBOL_BOOST_SCALE = 0.5

// Case-sensitive: IGNORECASE produces false positives like "Module" in Python docs
// or "Class" method calls in Ruby.
export const DEFINITION_KEYWORDS = [
  'class',
  'module',
  'defmodule', // Elixir
  'def',
  'interface',
  'struct',
  'enum',
  'trait',
  'type',
  'func',
  'function',
  'object',
  'abstract class',
  'data class',
  'fn',
  'fun', // Kotlin
  'package',
  'namespace',
  'protocol', // Swift
  'record', // C# 9+, Java 16+
  'typedef', // C/C++/Dart
] as const

// SQL DDL is conventionally all-caps or all-lowercase; match both via IGNORECASE.
export const SQL_DEFINITION_KEYWORDS = [
  'CREATE TABLE',
  'CREATE VIEW',
  'CREATE PROCEDURE',
  'CREATE FUNCTION',
] as const

// Additive boost multiplier for chunks that define a queried symbol.
export const DEFINITION_BOOST_MULTIPLIER = 3.0

// Additive boost multiplier for NL queries when file stems match query words.
export const STEM_BOOST_MULTIPLIER = 1.0

// Fraction of max_score added to each file's top chunk, scaled by its aggregate candidate score.
export const FILE_COHERENCE_BOOST_FRAC = 0.2

// Common English stopwords excluded from file-stem matching for NL queries.
export const STOPWORDS: ReadonlySet<string> = new Set(
  ('a an and are as at be by do does for from has have how if in is it not of on or the to was'
  + ' what when where which who why with').split(' '),
)

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** Find the max numeric value in an iterable without spreading (avoids argument-count limits). */
function maxValue(values: Iterable<number>): number {
  let m = Number.NEGATIVE_INFINITY
  for (const v of values) {
    if (v > m)
      m = v
  }
  return m
}

const KEYWORD_PREFIX = '(?:^|(?<=\\s))(?:'
const DEFINITION_KEYWORD_BODY = DEFINITION_KEYWORDS.map(escapeRegex).join('|')
const SQL_KEYWORD_BODY = SQL_DEFINITION_KEYWORDS.map(escapeRegex).join('|')

/** Return True if the query looks like a bare symbol or namespace-qualified identifier. */
export function isSymbolQuery(query: string): boolean {
  return SYMBOL_QUERY_RE.test(query.trim())
}

/** Apply query-type boosts to candidate scores. Returns a new Map. */
export function applyQueryBoost(
  combinedScores: Map<Chunk, number>,
  query: string,
  allChunks: Chunk[],
): Map<Chunk, number> {
  if (combinedScores.size === 0) {
    return combinedScores
  }

  const maxScore = maxValue(combinedScores.values())
  const boosted = new Map(combinedScores)

  if (isSymbolQuery(query)) {
    _boostSymbolDefinitions(boosted, query, maxScore, allChunks)
  }
  else {
    _boostStemMatches(boosted, query, maxScore)
    _boostEmbeddedSymbols(boosted, query, maxScore, allChunks)
  }

  return boosted
}

/** Promote files with multiple high-scoring chunks by boosting their top chunk (in-place). */
export function boostMultiChunkFiles(scores: Map<Chunk, number>): void {
  if (scores.size === 0) {
    return
  }

  const maxScore = maxValue(scores.values())
  if (maxScore === 0.0) {
    return
  }

  const fileSum = new Map<string, number>()
  const bestChunk = new Map<string, Chunk>()
  for (const [chunk, score] of scores) {
    const filePath = chunk.filePath
    fileSum.set(filePath, (fileSum.get(filePath) ?? 0.0) + score)
    const existingBest = bestChunk.get(filePath)
    if (existingBest === undefined || score > (scores.get(existingBest) ?? -Infinity)) {
      bestChunk.set(filePath, chunk)
    }
  }

  const maxFileSum = maxValue(fileSum.values())
  const boostUnit = maxScore * FILE_COHERENCE_BOOST_FRAC
  for (const [filePath, chunk] of bestChunk) {
    const sum = fileSum.get(filePath) ?? 0.0
    scores.set(chunk, (scores.get(chunk) ?? 0.0) + boostUnit * sum / maxFileSum)
  }
}

/**
 * Extract the final identifier from a possibly namespace-qualified query.
 *
 * Examples: "Sinatra::Base" → "Base", "Client" → "Client".
 */
export function _extractSymbolName(query: string): string {
  for (const separator of ['::', '\\', '->', '.']) {
    const idx = query.lastIndexOf(separator)
    if (idx !== -1) {
      return query.slice(idx + separator.length)
    }
  }
  return query.trim()
}

// LRU-ish cache for compiled definition patterns; simple FIFO eviction at 256 entries.
const DEFINITION_PATTERN_CACHE_MAX = 256
const _definitionPatternCache = new Map<string, [RegExp, RegExp]>()

export function _definitionPattern(symbolName: string): [RegExp, RegExp] {
  const cached = _definitionPatternCache.get(symbolName)
  if (cached !== undefined) {
    return cached
  }

  const escaped = escapeRegex(symbolName)
  const nsPrefix = '(?:[A-Z_a-z]\\w*(?:\\.|::))*'
  const suffix = `)\\s+${nsPrefix}${escaped}(?:\\s|[<({:\\[;]|$)`
  const general = new RegExp(KEYWORD_PREFIX + DEFINITION_KEYWORD_BODY + suffix, 'm')
  const sql = new RegExp(KEYWORD_PREFIX + SQL_KEYWORD_BODY + suffix, 'im')
  const entry: [RegExp, RegExp] = [general, sql]

  if (_definitionPatternCache.size >= DEFINITION_PATTERN_CACHE_MAX) {
    // FIFO eviction: drop the oldest entry.
    const firstKey = _definitionPatternCache.keys().next().value
    if (firstKey !== undefined) {
      _definitionPatternCache.delete(firstKey)
    }
  }
  _definitionPatternCache.set(symbolName, entry)
  return entry
}

/**
 * Return True if the chunk contains a definition of *symbolName*.
 *
 * Case-sensitive for general keywords, case-insensitive for SQL DDL.
 * Also matches namespace-qualified forms (e.g. `defmodule Phoenix.Router` for `Router`).
 */
export function _chunkDefinesSymbol(chunk: Chunk, symbolName: string): boolean {
  const [general, sql] = _definitionPattern(symbolName)
  return general.test(chunk.content) || sql.test(chunk.content)
}

/** Return True if *stem* matches *name* (exact, snake_case-normalised, or plural). */
export function _stemMatches(stem: string, name: string): boolean {
  const stemNorm = stem.replace(/_/g, '')
  const stripS = (s: string): string => s.endsWith('s') ? s.replace(/s+$/, '') : s
  return stem === name || stemNorm === name || stripS(stem) === name || stripS(stemNorm) === name
}

function pathStemLower(filePath: string): string {
  // Match Python's pathlib.Path.stem: filename without suffix; handles both / and \.
  const sepIdx = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\'))
  const base = sepIdx === -1 ? filePath : filePath.slice(sepIdx + 1)
  const dotIdx = base.lastIndexOf('.')
  // Path.stem leaves leading-dot files untouched (".gitignore" → ".gitignore").
  const stem = dotIdx <= 0 ? base : base.slice(0, dotIdx)
  return stem.toLowerCase()
}

function pathParentName(filePath: string): string {
  // Strip trailing separators, then take the segment before the basename.
  const cleaned = filePath.replace(/[/\\]+$/, '')
  const sepIdx = Math.max(cleaned.lastIndexOf('/'), cleaned.lastIndexOf('\\'))
  if (sepIdx === -1)
    return ''
  const parent = cleaned.slice(0, sepIdx)
  const parentSepIdx = Math.max(parent.lastIndexOf('/'), parent.lastIndexOf('\\'))
  return parentSepIdx === -1 ? parent : parent.slice(parentSepIdx + 1)
}

/** Return the boost amount for a chunk that defines one of *names* (0.0 if none match). */
export function _definitionTier(chunk: Chunk, names: Set<string>, boostUnit: number): number {
  let matches = false
  for (const name of names) {
    if (_chunkDefinesSymbol(chunk, name)) {
      matches = true
      break
    }
  }
  if (!matches)
    return 0.0
  const stem = pathStemLower(chunk.filePath)
  for (const name of names) {
    if (_stemMatches(stem, name.toLowerCase())) {
      return boostUnit * 1.5
    }
  }
  return boostUnit * 1.0
}

/** Boost non-candidate chunks whose lowercased file stem satisfies stemOk (in-place). */
export function _scanNonCandidates(
  boosted: Map<Chunk, number>,
  names: Set<string>,
  boostUnit: number,
  allChunks: Chunk[],
  stemOk: (stem: string) => boolean,
): void {
  for (const chunk of allChunks) {
    if (boosted.has(chunk))
      continue
    if (!stemOk(pathStemLower(chunk.filePath)))
      continue
    const tier = _definitionTier(chunk, names, boostUnit)
    if (tier !== 0.0) {
      boosted.set(chunk, tier)
    }
  }
}

/** Boost chunks that define the queried symbol, scanning candidates and stem-matched non-candidates (in-place). */
export function _boostSymbolDefinitions(
  boosted: Map<Chunk, number>,
  query: string,
  maxScore: number,
  allChunks: Chunk[],
): void {
  const symbolName = _extractSymbolName(query)
  const names = new Set<string>([symbolName])
  const trimmed = query.trim()
  if (symbolName !== trimmed) {
    names.add(trimmed)
  }

  const boostUnit = maxScore * DEFINITION_BOOST_MULTIPLIER

  for (const chunk of Array.from(boosted.keys())) {
    const tier = _definitionTier(chunk, names, boostUnit)
    if (tier !== 0.0) {
      boosted.set(chunk, (boosted.get(chunk) ?? 0.0) + tier)
    }
  }

  const symbolLower = symbolName.toLowerCase()
  _scanNonCandidates(
    boosted,
    names,
    boostUnit,
    allChunks,
    stem => _stemMatches(stem, symbolLower),
  )
}

/**
 * Boost chunks defining CamelCase/camelCase symbols embedded in NL queries (in-place).
 *
 * Half-strength vs pure symbol queries. Non-candidate scan uses stem-prefix match
 * so e.g. `state.ts` is found for symbol `StateManager`.
 */
export function _boostEmbeddedSymbols(
  boosted: Map<Chunk, number>,
  query: string,
  maxScore: number,
  allChunks: Chunk[],
): void {
  const names = new Set<string>(query.match(EMBEDDED_SYMBOL_RE) ?? [])
  if (names.size === 0)
    return

  const boostUnit = maxScore * DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE

  for (const chunk of Array.from(boosted.keys())) {
    const tier = _definitionTier(chunk, names, boostUnit)
    if (tier !== 0.0) {
      boosted.set(chunk, (boosted.get(chunk) ?? 0.0) + tier)
    }
  }

  const symbolsLower: string[] = Array.from(names, s => s.toLowerCase())
  for (const chunk of allChunks) {
    if (boosted.has(chunk))
      continue
    const stem = pathStemLower(chunk.filePath)
    const stemNorm = stem.replace(/_/g, '')
    let matches = false
    for (const symbolLower of symbolsLower) {
      if (
        stem === symbolLower
        || stemNorm === symbolLower
        || (stem.length >= EMBEDDED_STEM_MIN_LEN && symbolLower.startsWith(stem))
        || (stemNorm.length >= EMBEDDED_STEM_MIN_LEN && symbolLower.startsWith(stemNorm))
      ) {
        matches = true
        break
      }
    }
    if (!matches)
      continue
    const tier = _definitionTier(chunk, names, boostUnit)
    if (tier !== 0.0) {
      boosted.set(chunk, tier)
    }
  }
}

/** Count query keywords that match path parts, allowing prefix overlap (min 3 chars). */
export function _countKeywordMatches(keywords: Set<string>, parts: Set<string>): number {
  let exactCount = 0
  const exact = new Set<string>()
  for (const k of keywords) {
    if (parts.has(k)) {
      exact.add(k)
      exactCount++
    }
  }
  if (exactCount === keywords.size) {
    return exactCount
  }
  let nMatches = exactCount
  for (const keyword of keywords) {
    if (exact.has(keyword))
      continue
    for (const part of parts) {
      const [shorter, longer] = keyword.length <= part.length ? [keyword, part] : [part, keyword]
      if (shorter.length >= 3 && longer.startsWith(shorter)) {
        nMatches++
        break
      }
    }
  }
  return nMatches
}

const QUERY_WORD_RE = /[A-Z_a-z]\w*/g

/**
 * Boost chunks whose file paths match NL query keywords (in-place).
 *
 * Uses prefix matching for morphological variants (e.g. "dependency" matches
 * "dependencies"). Matches file stems and the immediate parent directory name.
 */
export function _boostStemMatches(
  boosted: Map<Chunk, number>,
  query: string,
  maxScore: number,
): void {
  const keywords = new Set<string>()
  for (const word of query.match(QUERY_WORD_RE) ?? []) {
    if (word.length > 2) {
      const lower = word.toLowerCase()
      if (!STOPWORDS.has(lower)) {
        keywords.add(lower)
      }
    }
  }
  if (keywords.size === 0)
    return

  const boost = maxScore * STEM_BOOST_MULTIPLIER
  const pathCache = new Map<string, Set<string>>()
  for (const chunk of Array.from(boosted.keys())) {
    let parts = pathCache.get(chunk.filePath)
    if (parts === undefined) {
      // Use original-case stem so splitIdentifier sees camelCase boundaries.
      parts = new Set<string>(splitIdentifier(pathStemOriginal(chunk.filePath)))
      const parentName = pathParentName(chunk.filePath)
      if (parentName !== '' && parentName !== '.' && parentName !== '/' && parentName !== '..') {
        for (const p of splitIdentifier(parentName)) {
          parts.add(p)
        }
      }
      pathCache.set(chunk.filePath, parts)
    }
    const nMatches = _countKeywordMatches(keywords, parts)
    if (nMatches > 0) {
      const matchRatio = nMatches / keywords.size
      if (matchRatio >= 0.10) {
        boosted.set(chunk, (boosted.get(chunk) ?? 0.0) + boost * matchRatio)
      }
    }
  }
}

function pathStemOriginal(filePath: string): string {
  const sepIdx = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\'))
  const base = sepIdx === -1 ? filePath : filePath.slice(sepIdx + 1)
  const dotIdx = base.lastIndexOf('.')
  return dotIdx <= 0 ? base : base.slice(0, dotIdx)
}
