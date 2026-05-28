// Tests for src/stats.ts — port of src/semble/stats.py
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { appendFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  BucketStats,
  buildSavingsSummary,
  formatSavingsReport,
  resetStatsFile,
  saveSearchStats,
  setStatsFile,
  type StatsSearchResult,
} from './stats.ts'

function makeResult(content: string, filePath: string): StatsSearchResult {
  return { chunk: { content, filePath } }
}

let tmpDir: string
let statsFile: string

beforeEach(() => {
  tmpDir = mkdtempSync(path.join(tmpdir(), 'csp-stats-'))
  statsFile = path.join(tmpDir, 'savings.jsonl')
  setStatsFile(statsFile)
})

afterEach(() => {
  resetStatsFile()
  rmSync(tmpDir, { recursive: true, force: true })
})

describe('BucketStats', () => {
  test('add accumulates fields and clamps savedChars to >= 0', () => {
    const b = new BucketStats()
    b.add(100, 400)
    b.add(100, 400)
    expect(b.calls).toBe(2)
    expect(b.snippetChars).toBe(200)
    expect(b.fileChars).toBe(800)
    expect(b.savedChars).toBe(600)
  })

  test('add does not produce negative savedChars when snippet > file', () => {
    const b = new BucketStats()
    b.add(500, 100)
    expect(b.savedChars).toBe(0)
    expect(b.snippetChars).toBe(500)
    expect(b.fileChars).toBe(100)
  })
})

describe('saveSearchStats', () => {
  test('appends one valid JSONL line per call', () => {
    const results = [makeResult('hello world', '/a.ts'), makeResult('foo bar baz', '/b.ts')]
    saveSearchStats(results, 'search', { '/a.ts': 100, '/b.ts': 200 })

    const content = readFileSync(statsFile, 'utf8')
    const lines = content.split('\n').filter(l => l.length > 0)
    expect(lines).toHaveLength(1)
    const record = JSON.parse(lines[0]!) as Record<string, unknown>
    expect(record.call).toBe('search')
    expect(record.results).toBe(2)
    expect(record.snippet_chars).toBe('hello world'.length + 'foo bar baz'.length)
    expect(record.file_chars).toBe(300)
    expect(typeof record.ts).toBe('number')
  })

  test('two calls produce two lines', () => {
    saveSearchStats([makeResult('abc', '/a.ts')], 'search', { '/a.ts': 50 })
    saveSearchStats([makeResult('xy', '/b.ts')], 'find_related', { '/b.ts': 20 })

    const content = readFileSync(statsFile, 'utf8')
    const lines = content.split('\n').filter(l => l.length > 0)
    expect(lines).toHaveLength(2)
    const r1 = JSON.parse(lines[0]!) as Record<string, unknown>
    const r2 = JSON.parse(lines[1]!) as Record<string, unknown>
    expect(r1.call).toBe('search')
    expect(r2.call).toBe('find_related')
  })

  test('deduplicates file_chars per unique filePath', () => {
    // Same path twice — file should only count once toward file_chars.
    const results = [makeResult('aaa', '/a.ts'), makeResult('bbb', '/a.ts')]
    saveSearchStats(results, 'search', { '/a.ts': 100 })

    const content = readFileSync(statsFile, 'utf8')
    const lines = content.split('\n').filter(l => l.length > 0)
    const record = JSON.parse(lines[0]!) as Record<string, unknown>
    expect(record.file_chars).toBe(100)
    expect(record.snippet_chars).toBe(6)
  })

  test('ignores paths missing from fileSizes', () => {
    const results = [makeResult('aaa', '/a.ts'), makeResult('bbb', '/missing.ts')]
    saveSearchStats(results, 'search', { '/a.ts': 100 })

    const content = readFileSync(statsFile, 'utf8')
    const lines = content.split('\n').filter(l => l.length > 0)
    const record = JSON.parse(lines[0]!) as Record<string, unknown>
    expect(record.file_chars).toBe(100)
  })

  test('never throws on I/O error', () => {
    // Point stats file at a path whose parent cannot be created (a regular
    // file used as a directory). saveSearchStats must swallow the error.
    const conflictFile = path.join(tmpDir, 'conflict')
    writeFileSync(conflictFile, 'not a directory')
    setStatsFile(path.join(conflictFile, 'nested', 'savings.jsonl'))

    expect(() => {
      saveSearchStats([makeResult('x', '/a.ts')], 'search', { '/a.ts': 10 })
    }).not.toThrow()
  })
})

describe('buildSavingsSummary', () => {
  test('parses all valid lines and skips malformed ones', () => {
    const now = Date.now() / 1000
    const lines = [
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
      'this is not json',
      JSON.stringify({ ts: now, call: 'find_related', results: 1, snippet_chars: 50, file_chars: 200 }),
      '{"incomplete": ',
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
    ]
    writeFileSync(statsFile, `${lines.join('\n')}\n`)

    const summary = buildSavingsSummary()
    expect(summary.buckets['All time']!.calls).toBe(3)
    expect(summary.callTypeCounts).toEqual({ search: 2, find_related: 1 })
  })

  test('bucket math: 2 search calls with snippet=100/file=400 → savedChars=600, ratio 0.75', () => {
    const now = Date.now() / 1000
    const lines = [
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
    ]
    writeFileSync(statsFile, `${lines.join('\n')}\n`)

    const summary = buildSavingsSummary()
    const all = summary.buckets['All time']!
    expect(all.calls).toBe(2)
    expect(all.snippetChars).toBe(200)
    expect(all.fileChars).toBe(800)
    expect(all.savedChars).toBe(600)
    expect(all.savedChars / all.fileChars).toBe(0.75)

    expect(summary.buckets['Today']!.calls).toBe(2)
    expect(summary.buckets['Last 7 days']!.calls).toBe(2)
  })

  test('older entries fall outside Today and Last 7 days buckets', () => {
    const now = Date.now() / 1000
    const tenDaysAgo = now - 10 * 24 * 60 * 60
    const lines = [
      JSON.stringify({ ts: tenDaysAgo, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
    ]
    writeFileSync(statsFile, `${lines.join('\n')}\n`)

    const summary = buildSavingsSummary()
    expect(summary.buckets['All time']!.calls).toBe(2)
    expect(summary.buckets['Last 7 days']!.calls).toBe(1)
    expect(summary.buckets['Today']!.calls).toBe(1)
  })

  test('missing stats file returns empty summary', () => {
    const summary = buildSavingsSummary(path.join(tmpDir, 'does-not-exist.jsonl'))
    expect(summary.buckets['All time']!.calls).toBe(0)
    expect(summary.callTypeCounts).toEqual({})
  })

  test('skips records with NaN numeric fields', () => {
    // `typeof NaN === 'number'` would otherwise let these through and
    // poison date formatting / bucket math with NaN.
    const now = Date.now() / 1000
    const lines = [
      // NaN serializes as `null` in JSON.stringify, so emit NaN literally.
      '{"ts": NaN, "call": "search", "results": 1, "snippet_chars": 0, "file_chars": 0}',
      '{"ts": 0, "call": "search", "results": 1, "snippet_chars": NaN, "file_chars": 0}',
      '{"ts": 0, "call": "search", "results": 1, "snippet_chars": 0, "file_chars": NaN}',
      JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 }),
    ]
    writeFileSync(statsFile, `${lines.join('\n')}\n`)

    const summary = buildSavingsSummary()
    // Only the last valid record is counted.
    expect(summary.buckets['All time']!.calls).toBe(1)
    expect(summary.callTypeCounts).toEqual({ search: 1 })
  })

  test('call types matching built-in object properties do not collide', () => {
    // Without Object.create(null), `callTypeCounts["toString"]` would
    // resolve to Function.prototype.toString and arithmetic would coerce
    // it to a string instead of incrementing.
    const now = Date.now() / 1000
    const lines = [
      JSON.stringify({ ts: now, call: 'toString', results: 1, snippet_chars: 1, file_chars: 1 }),
      JSON.stringify({ ts: now, call: 'toString', results: 1, snippet_chars: 1, file_chars: 1 }),
      JSON.stringify({ ts: now, call: 'hasOwnProperty', results: 1, snippet_chars: 1, file_chars: 1 }),
    ]
    writeFileSync(statsFile, `${lines.join('\n')}\n`)

    const summary = buildSavingsSummary()
    expect(summary.callTypeCounts).toEqual({ toString: 2, hasOwnProperty: 1 })
  })
})

describe('formatSavingsReport', () => {
  test('shows "Csp Token Savings" header and bucket labels', () => {
    const now = Date.now() / 1000
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 })}\n`,
    )

    const report = formatSavingsReport()
    expect(report).toContain('Csp Token Savings')
    expect(report).toContain('Today')
    expect(report).toContain('Last 7 days')
    expect(report).toContain('All time')
    expect(report).not.toContain('Semble Token Savings')
  })

  test('empty / missing stats file returns the "no stats yet" message', () => {
    expect(existsSync(statsFile)).toBe(false)
    expect(formatSavingsReport()).toBe('No stats yet. Run a search first.')
  })

  test('formats saved tokens with ~Nk suffix at 1500 → ~1.5k', () => {
    // file=6400, snippet=400 ⇒ saved=6000 chars ⇒ 6000/4 = 1500 tokens ⇒ "~1.5k"
    const now = Date.now() / 1000
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 400, file_chars: 6400 })}\n`,
    )

    const report = formatSavingsReport()
    expect(report).toContain('~1.5k')
  })

  test('verbose appends Usage Breakdown section with sorted call types', () => {
    const now = Date.now() / 1000
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 })}\n`,
    )
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'find_related', results: 1, snippet_chars: 50, file_chars: 200 })}\n`,
    )

    const report = formatSavingsReport({ verbose: true })
    expect(report).toContain('Usage Breakdown')
    expect(report).toContain('Call type')
    expect(report).toContain('search')
    expect(report).toContain('find_related')
    // Sorted alphabetically — find_related should appear before search.
    const findIdx = report.indexOf('find_related')
    const searchHeadingsStripped = report.replace('Csp Token Savings', '')
    const searchIdx = searchHeadingsStripped.indexOf('search')
    expect(findIdx).toBeLessThan(searchIdx + 'Csp Token Savings'.length)
  })

  test('renders bar with filled blocks proportional to ratio', () => {
    const now = Date.now() / 1000
    // ratio = 0.75 ⇒ 12 filled / 4 empty out of 16.
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 100, file_chars: 400 })}\n`,
    )
    const report = formatSavingsReport()
    expect(report).toContain('[████████████░░░░]')
    expect(report).toContain('(75%)')
  })

  test('formats saved tokens with ~N.NM suffix at 1M+ tokens', () => {
    // saved_chars = 4_000_000 ⇒ tokens = 1_000_000 ⇒ "~1.0M"
    const now = Date.now() / 1000
    appendFileSync(
      statsFile,
      `${JSON.stringify({ ts: now, call: 'search', results: 1, snippet_chars: 0, file_chars: 4_000_000 })}\n`,
    )
    const report = formatSavingsReport()
    expect(report).toContain('~1.0M')
  })
})
