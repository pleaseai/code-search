// Port of src/semble/stats.py
import { appendFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'

/**
 * Call type for token-savings tracking.
 *
 * Mirrors `CallType` from `src/semble/types.py`. Defined here as a minimal
 * type to avoid creating a cross-unit dependency before `src/types.ts`
 * lands. Once that exists, this should be re-exported from there.
 */
export type CallType = 'search' | 'find_related'

/**
 * Minimal chunk shape needed by `saveSearchStats`.
 *
 * Uses camelCase fields per the csp public API surface.
 */
export interface StatsChunk {
  content: string
  filePath: string
}

/**
 * Minimal search-result shape needed by `saveSearchStats`.
 */
export interface StatsSearchResult {
  chunk: StatsChunk
}

/**
 * Per-bucket aggregate counters for the savings report.
 */
export class BucketStats {
  calls: number = 0
  snippetChars: number = 0
  fileChars: number = 0
  savedChars: number = 0

  /** Update stats with a call and its character counts. */
  add(snippetChars: number, fileChars: number): void {
    this.calls += 1
    this.snippetChars += snippetChars
    this.fileChars += fileChars
    this.savedChars += Math.max(0, fileChars - snippetChars)
  }
}

/**
 * Aggregated savings, grouped into time buckets plus per-call-type counts.
 */
export interface SavingsSummary {
  buckets: Record<string, BucketStats>
  callTypeCounts: Record<string, number>
}

const DEFAULT_STATS_FILE = path.join(homedir(), '.csp', 'savings.jsonl')

let _STATS_FILE = DEFAULT_STATS_FILE

/**
 * Override the stats file location. Intended for tests only — production
 * callers should leave the default in place so behavior matches semble.
 */
export function setStatsFile(filePath: string): void {
  _STATS_FILE = filePath
}

/** Return the current stats file path. */
export function getStatsFile(): string {
  return _STATS_FILE
}

/** Reset the stats file path back to the default `~/.csp/savings.jsonl`. */
export function resetStatsFile(): void {
  _STATS_FILE = DEFAULT_STATS_FILE
}

/**
 * Save stats about a search or find_related call to the stats file.
 *
 * Best-effort: any I/O error is silently swallowed so stats writes never
 * impact a live search.
 */
export function saveSearchStats(
  results: StatsSearchResult[],
  callType: CallType,
  fileSizes: Record<string, number>,
): void {
  try {
    const snippetChars = results.reduce((sum, r) => sum + r.chunk.content.length, 0)
    const uniquePaths = new Set(results.map(r => r.chunk.filePath))
    let fileChars = 0
    for (const p of uniquePaths) {
      if (Object.prototype.hasOwnProperty.call(fileSizes, p)) {
        fileChars += fileSizes[p] ?? 0
      }
    }

    const record = {
      ts: Date.now() / 1000,
      call: callType,
      results: results.length,
      snippet_chars: snippetChars,
      file_chars: fileChars,
    }
    const dir = path.dirname(_STATS_FILE)
    mkdirSync(dir, { recursive: true })
    appendFileSync(_STATS_FILE, `${JSON.stringify(record)}\n`)
  }
  catch {
    // Swallow — stats writes must never throw.
  }
}

interface StatsRecord {
  ts: number
  call: string
  results: number
  snippet_chars: number
  file_chars: number
}

function isStatsRecord(value: unknown): value is StatsRecord {
  if (value === null || typeof value !== 'object')
    return false
  const v = value as Record<string, unknown>
  // Reject NaN explicitly: `typeof NaN === 'number'` is true, but NaN
  // values would propagate into date formatting ("NaN-NaN-NaN") and
  // bucket arithmetic. Treat such lines as malformed.
  return (
    typeof v.ts === 'number'
    && !Number.isNaN(v.ts)
    && typeof v.call === 'string'
    && typeof v.snippet_chars === 'number'
    && !Number.isNaN(v.snippet_chars)
    && typeof v.file_chars === 'number'
    && !Number.isNaN(v.file_chars)
  )
}

function ymdUtc(timestampSeconds: number): string {
  const d = new Date(timestampSeconds * 1000)
  const y = d.getUTCFullYear()
  const m = String(d.getUTCMonth() + 1).padStart(2, '0')
  const day = String(d.getUTCDate()).padStart(2, '0')
  return `${y}-${m}-${day}`
}

/**
 * Read `savings.jsonl` and return a {@link SavingsSummary}.
 *
 * Malformed lines are skipped silently. If the file is missing, an empty
 * summary is returned.
 */
export function buildSavingsSummary(filePath?: string): SavingsSummary {
  const target = filePath ?? _STATS_FILE
  const now = new Date()
  const nowSec = now.getTime() / 1000
  const today = ymdUtc(nowSec)
  const sevenDaysAgo = ymdUtc(nowSec - 7 * 24 * 60 * 60)

  const buckets: Record<string, BucketStats> = {
    'Today': new BucketStats(),
    'Last 7 days': new BucketStats(),
    'All time': new BucketStats(),
  }
  // Use a prototype-less object so JSONL `call` values like "toString" or
  // "__proto__" can't collide with built-in object properties.
  const callTypeCounts: Record<string, number> = Object.create(null) as Record<string, number>

  if (!existsSync(target))
    return { buckets, callTypeCounts }

  const raw = readFileSync(target, 'utf8')
  const lines = raw.split('\n')
  for (const line of lines) {
    if (line.length === 0)
      continue
    let record: unknown
    try {
      record = JSON.parse(line)
    }
    catch {
      // Match semble: skip malformed lines silently (semble logs a warning;
      // we omit the warning to keep stats imports side-effect-free).
      continue
    }
    if (!isStatsRecord(record))
      continue

    const snippetChars = record.snippet_chars
    const fileChars = record.file_chars
    const callType = record.call
    callTypeCounts[callType] = (callTypeCounts[callType] ?? 0) + 1

    const day = ymdUtc(record.ts)
    const inToday = day === today
    const inLast7 = day > sevenDaysAgo

    buckets['All time']!.add(snippetChars, fileChars)
    if (inLast7)
      buckets['Last 7 days']!.add(snippetChars, fileChars)
    if (inToday)
      buckets['Today']!.add(snippetChars, fileChars)
  }

  return { buckets, callTypeCounts }
}

function padRight(s: string, width: number): string {
  if (s.length >= width)
    return s
  return s + ' '.repeat(width - s.length)
}

function formatSavedTokens(savedTokens: number): string {
  if (savedTokens >= 1_000_000)
    return `~${(savedTokens / 1_000_000).toFixed(1)}M`
  if (savedTokens >= 1000)
    return `~${(savedTokens / 1000).toFixed(1)}k`
  return `~${savedTokens}`
}

function formatCalls(calls: number): string {
  return calls >= 1000 ? `${(calls / 1000).toFixed(1)}k` : String(calls)
}

export interface FormatSavingsReportOptions {
  path?: string
  verbose?: boolean
}

/**
 * Return a formatted token-savings report.
 *
 * Output mirrors semble's ASCII bar chart byte-for-byte, with the header
 * swapped from "Semble Token Savings" → "Csp Token Savings".
 */
export function formatSavingsReport(options: FormatSavingsReportOptions = {}): string {
  const target = options.path ?? _STATS_FILE
  const verbose = options.verbose ?? false

  if (!existsSync(target))
    return 'No stats yet. Run a search first.'

  const summary = buildSavingsSummary(target)
  const barWidth = 16
  const heavyLine = `  ${'═'.repeat(64)}`
  const lightLine = `  ${'─'.repeat(64)}`

  const lines: string[] = [
    '',
    '  Csp Token Savings',
    heavyLine,
    `  ${padRight('Period', 12)}  ${padRight('Calls', 6)}  Savings`,
    lightLine,
  ]

  for (const [label, bucket] of Object.entries(summary.buckets)) {
    const savedTokens = Math.floor(bucket.savedChars / 4) // ~4 chars/token approximation
    const savedStr = formatSavedTokens(savedTokens)
    const callsStr = formatCalls(bucket.calls)
    if (bucket.fileChars > 0) {
      const ratio = bucket.savedChars / bucket.fileChars
      const filled = Math.round(ratio * barWidth)
      const bar = '█'.repeat(filled) + '░'.repeat(barWidth - filled)
      const pct = Math.round(ratio * 100)
      lines.push(`  ${padRight(label, 12)}  ${padRight(callsStr, 6)}  [${bar}]  ${savedStr} tokens (${pct}%)`)
    }
    else {
      lines.push(`  ${padRight(label, 12)}  ${padRight(callsStr, 6)}  [${'░'.repeat(barWidth)}]  ${savedStr} tokens`)
    }
  }

  const callTypeEntries = Object.entries(summary.callTypeCounts)
  if (verbose && callTypeEntries.length > 0) {
    lines.push('', '  Usage Breakdown', lightLine, `  ${padRight('Call type', 16)}  Calls`)
    const sorted = callTypeEntries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    for (const [callType, count] of sorted) {
      const countStr = count >= 1000 ? `${(count / 1000).toFixed(1)}k` : String(count)
      lines.push(`  ${padRight(callType, 16)}  ${countStr}`)
    }
    lines.push(heavyLine)
  }
  lines.push('')
  return lines.join('\n')
}
