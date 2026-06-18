// Port of src/semble/stats.py
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import process from 'node:process'

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
      if (Object.hasOwn(fileSizes, p)) {
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

/**
 * Delete the savings stats file if it exists.
 *
 * Deletion (not truncation) mirrors semble's `clear` (`path.unlink()`) and
 * lets `csp savings` fall back to the "No stats yet" message — a truncated,
 * still-present file would instead render an all-zero report. Best-effort:
 * a permission error or broken symlink is swallowed and reported as
 * `cleared: false` rather than crashing the CLI.
 */
export function clearSavings(): { path: string, cleared: boolean } {
  if (!existsSync(_STATS_FILE)) {
    return { path: _STATS_FILE, cleared: false }
  }
  try {
    rmSync(_STATS_FILE)
    return { path: _STATS_FILE, cleared: true }
  }
  catch {
    return { path: _STATS_FILE, cleared: false }
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
  if (value === null || typeof value !== 'object') {
    return false
  }
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

  if (!existsSync(target)) {
    return { buckets, callTypeCounts }
  }

  const raw = readFileSync(target, 'utf8')
  const lines = raw.split('\n')
  for (const line of lines) {
    if (line.length === 0) {
      continue
    }
    let record: unknown
    try {
      record = JSON.parse(line)
    }
    catch {
      // Match semble: skip malformed lines silently (semble logs a warning;
      // we omit the warning to keep stats imports side-effect-free).
      continue
    }
    if (!isStatsRecord(record)) {
      continue
    }

    const snippetChars = record.snippet_chars
    const fileChars = record.file_chars
    const callType = record.call
    callTypeCounts[callType] = (callTypeCounts[callType] ?? 0) + 1

    const day = ymdUtc(record.ts)
    const inToday = day === today
    const inLast7 = day > sevenDaysAgo

    buckets['All time']!.add(snippetChars, fileChars)
    if (inLast7) {
      buckets['Last 7 days']!.add(snippetChars, fileChars)
    }
    if (inToday) {
      buckets.Today!.add(snippetChars, fileChars)
    }
  }

  return { buckets, callTypeCounts }
}

function padRight(s: string, width: number): string {
  if (s.length >= width) {
    return s
  }
  return s + ' '.repeat(width - s.length)
}

function padLeft(s: string, width: number): string {
  if (s.length >= width) {
    return s
  }
  return ' '.repeat(width - s.length) + s
}

/**
 * Whether ANSI colors should be emitted. Mirrors semble's `_use_color`:
 * suppressed under `NO_COLOR`, a `dumb` terminal, or a non-TTY stdout.
 */
function useColor(): boolean {
  return !('NO_COLOR' in process.env)
    && process.env.TERM !== 'dumb'
    && Boolean(process.stdout.isTTY)
}

/** Wrap `text` in an ANSI color `code` when `enabled`. */
function color(code: string, text: string, enabled: boolean): string {
  return enabled ? `[${code}m${text}[0m` : text
}

/** Color a savings percentage by value: green ≥80, yellow ≥50, red below. */
function colorRatio(pct: number, enabled: boolean): string {
  const code = pct >= 80 ? '32' : pct >= 50 ? '33' : '31'
  return color(code, `${pct}%`, enabled)
}

function formatSavedTokens(savedTokens: number): string {
  if (savedTokens >= 1_000_000) {
    return `~${(savedTokens / 1_000_000).toFixed(1)}M`
  }
  if (savedTokens >= 1000) {
    return `~${(savedTokens / 1000).toFixed(1)}k`
  }
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
 * Adopts semble's redesigned layout (PR #197): a headline summary
 * (Total saved / Total calls / Efficiency bar) followed by a "By Period"
 * table, with ANSI color when stdout is a color-capable TTY. Two csp
 * divergences are preserved: the header reads "Csp Token Savings" (not
 * "Semble Token Savings"), and the "By Call Type" breakdown stays gated
 * behind `--verbose` rather than always shown.
 */
export function formatSavingsReport(options: FormatSavingsReportOptions = {}): string {
  const target = options.path ?? _STATS_FILE
  const verbose = options.verbose ?? false

  if (!existsSync(target)) {
    return 'No stats yet. Run a search first.'
  }

  const summary = buildSavingsSummary(target)
  const enabled = useColor()
  const barWidth = 24
  const borderWidth = 72
  const heavyLine = `  ${color('38;5;244', '═'.repeat(borderWidth), enabled)}`
  const lightLine = `  ${color('38;5;244', '─'.repeat(borderWidth), enabled)}`

  const allTime = summary.buckets['All time']!
  const totalSavedTokens = Math.floor(allTime.savedChars / 4) // ~4 chars/token approximation
  const overallPct = allTime.fileChars > 0
    ? Math.round((allTime.savedChars / allTime.fileChars) * 100)
    : 0
  const efficiencyFilled = Math.round((overallPct / 100) * barWidth)
  const efficiencyBar
    = color('32', '█'.repeat(efficiencyFilled), enabled)
      + color('38;5;244', '░'.repeat(barWidth - efficiencyFilled), enabled)

  const lines: string[] = [
    '',
    `  ${color('1;36', 'Csp Token Savings', enabled)}`,
    heavyLine,
    '',
    `  ${color('1', 'Total saved:', enabled)}  ${color('1;33', `${formatSavedTokens(totalSavedTokens)} tokens`, enabled)}  (${colorRatio(overallPct, enabled)})`,
    `  ${color('1', 'Total calls:', enabled)}  ${color('1;33', formatCalls(allTime.calls), enabled)}`,
    `  ${color('1', 'Efficiency:', enabled)}  ${efficiencyBar}  ${colorRatio(overallPct, enabled)}`,
    '',
    `  ${color('1', 'By Period', enabled)}`,
    lightLine,
    `  ${padRight('Period', 14)}  ${padLeft('Calls', 8)}  ${padLeft('Saved', 14)}  Ratio`,
    lightLine,
  ]

  for (const [label, bucket] of Object.entries(summary.buckets)) {
    const savedTokens = Math.floor(bucket.savedChars / 4)
    const savedStr = `${formatSavedTokens(savedTokens)} tokens`
    const callsStr = formatCalls(bucket.calls)
    let rowBar: string
    let ratioStr: string
    if (bucket.fileChars > 0) {
      const ratio = bucket.savedChars / bucket.fileChars
      const filled = Math.round(ratio * barWidth)
      rowBar = color('32', '█'.repeat(filled), enabled) + color('38;5;244', '░'.repeat(barWidth - filled), enabled)
      ratioStr = colorRatio(Math.round(ratio * 100), enabled)
    }
    else {
      rowBar = color('38;5;244', '░'.repeat(barWidth), enabled)
      ratioStr = color('38;5;244', '–', enabled)
    }
    lines.push(
      `  ${color('1', padRight(label, 14), enabled)}  ${color('1;33', padLeft(callsStr, 8), enabled)}  `
      + `${color('1;33', padLeft(savedStr, 14), enabled)}  ${rowBar}  ${ratioStr}`,
    )
  }

  const callTypeEntries = Object.entries(summary.callTypeCounts)
  if (verbose && callTypeEntries.length > 0) {
    lines.push(
      '',
      `  ${color('1', 'By Call Type', enabled)}`,
      lightLine,
      `  ${padRight('#', 4)}  ${padRight('Call type', 16)}  ${padLeft('Calls', 8)}  Share`,
      lightLine,
    )
    const total = callTypeEntries.reduce((sum, [, count]) => sum + count, 0)
    // Sort by call count descending; ties keep insertion order.
    const sorted = [...callTypeEntries].sort(([, a], [, b]) => b - a)
    sorted.forEach(([callType, count], i) => {
      const share = total > 0 ? count / total : 0
      const filled = Math.max(1, Math.round(share * 16))
      const bar = color('32', '█'.repeat(filled), enabled) + color('38;5;244', '░'.repeat(16 - filled), enabled)
      const rank = `${i + 1}.`
      lines.push(
        `  ${color('38;5;244', padRight(rank, 4), enabled)}  ${padRight(callType, 16)}  `
        + `${color('1;33', padLeft(formatCalls(count), 8), enabled)}  ${bar}  `
        + `${color('38;5;244', padLeft(`${Math.round(share * 100)}%`, 4), enabled)}`,
      )
    })
  }
  lines.push(heavyLine)
  lines.push('')
  return lines.join('\n')
}
