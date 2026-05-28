// Port of src/semble/index/file_walker.py — tests
import { describe, expect, test, beforeEach, afterEach } from 'bun:test'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  DEFAULT_IGNORED_DIRS,
  _isIgnored,
  _loadIgnoreForDir,
  walkFiles,
} from './file-walker.ts'

let ignoreAvailable = true
try {
  await import('ignore')
}
catch {
  ignoreAvailable = false
}

const describeWithIgnore = ignoreAvailable ? describe : describe.skip

async function collect(iter: AsyncIterable<string>): Promise<string[]> {
  const out: string[] = []
  for await (const item of iter) out.push(item)
  return out
}

describe('DEFAULT_IGNORED_DIRS', () => {
  test('contains the csp cache dir instead of the semble one', () => {
    expect(DEFAULT_IGNORED_DIRS.has('.csp/')).toBe(true)
    expect(DEFAULT_IGNORED_DIRS.has('.semble/')).toBe(false)
  })

  test('contains canonical noisy directories', () => {
    for (const d of ['.git/', 'node_modules/', 'dist/', 'build/', '.next/', '__pycache__/']) {
      expect(DEFAULT_IGNORED_DIRS.has(d)).toBe(true)
    }
  })
})

describeWithIgnore('walkFiles', () => {
  let root: string

  beforeEach(() => {
    root = mkdtempSync(path.join(os.tmpdir(), 'csp-walker-'))
  })

  afterEach(() => {
    rmSync(root, { recursive: true, force: true })
  })

  test('yields all .ts files under root recursively', async () => {
    writeFileSync(path.join(root, 'a.ts'), 'a')
    mkdirSync(path.join(root, 'sub'))
    writeFileSync(path.join(root, 'sub', 'b.ts'), 'b')
    writeFileSync(path.join(root, 'sub', 'c.md'), 'c')
    mkdirSync(path.join(root, 'sub', 'nested'))
    writeFileSync(path.join(root, 'sub', 'nested', 'd.ts'), 'd')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['a.ts', path.join('sub', 'b.ts'), path.join('sub', 'nested', 'd.ts')])
  })

  test('skips symlinks', async () => {
    writeFileSync(path.join(root, 'real.ts'), 'real')
    try {
      symlinkSync(path.join(root, 'real.ts'), path.join(root, 'link.ts'))
    }
    catch {
      // Some sandboxes disallow symlinks — bail rather than fail.
      return
    }
    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['real.ts'])
  })

  test('always ignores .git/ and node_modules/', async () => {
    writeFileSync(path.join(root, 'keep.ts'), 'k')
    mkdirSync(path.join(root, '.git'))
    writeFileSync(path.join(root, '.git', 'hidden.ts'), 'h')
    mkdirSync(path.join(root, 'node_modules'))
    writeFileSync(path.join(root, 'node_modules', 'pkg.ts'), 'p')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['keep.ts'])
  })

  test('.gitignore excludes matching files', async () => {
    writeFileSync(path.join(root, '.gitignore'), '*.log\n')
    writeFileSync(path.join(root, 'foo.log'), 'foo')
    writeFileSync(path.join(root, 'bar.txt'), 'bar')

    const results = await collect(walkFiles(root, ['.log', '.txt']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['bar.txt'])
  })

  test('.gitignore negation-with-extension bypasses extension filter (found)', async () => {
    // `*.log` ignores everything ending in .log; `!special.log` un-ignores
    // special.log AND should be yielded even though `.log` is not in the
    // extension allowlist below.
    writeFileSync(path.join(root, '.gitignore'), '*.log\n!special.log\n')
    writeFileSync(path.join(root, 'foo.log'), 'foo')
    writeFileSync(path.join(root, 'special.log'), 'special')
    writeFileSync(path.join(root, 'keep.ts'), 'k')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['keep.ts', 'special.log'])
  })

  test('.cspignore is honoured in addition to .gitignore', async () => {
    writeFileSync(path.join(root, '.gitignore'), 'gitignored.ts\n')
    writeFileSync(path.join(root, '.cspignore'), 'cspignored.ts\n')
    writeFileSync(path.join(root, 'keep.ts'), 'k')
    writeFileSync(path.join(root, 'gitignored.ts'), 'g')
    writeFileSync(path.join(root, 'cspignored.ts'), 'c')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['keep.ts'])
  })

  test('respects nested .gitignore from subdirectories', async () => {
    writeFileSync(path.join(root, 'top.ts'), 't')
    mkdirSync(path.join(root, 'sub'))
    writeFileSync(path.join(root, 'sub', '.gitignore'), 'skip.ts\n')
    writeFileSync(path.join(root, 'sub', 'skip.ts'), 's')
    writeFileSync(path.join(root, 'sub', 'keep.ts'), 'k')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual([path.join('sub', 'keep.ts'), 'top.ts'])
  })

  test('honours the extra `ignore` arg', async () => {
    writeFileSync(path.join(root, 'foo.ts'), 'f')
    writeFileSync(path.join(root, 'bar.ts'), 'b')

    const results = await collect(walkFiles(root, ['.ts'], ['foo.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['bar.ts'])
  })

  test('filters by extension (case-insensitive)', async () => {
    writeFileSync(path.join(root, 'a.TS'), 'a')
    writeFileSync(path.join(root, 'b.ts'), 'b')
    writeFileSync(path.join(root, 'c.md'), 'c')

    const results = await collect(walkFiles(root, ['.ts']))
    const relative = results.map(p => path.relative(root, p)).sort()
    expect(relative).toEqual(['a.TS', 'b.ts'])
  })
})

describeWithIgnore('_loadIgnoreForDir', () => {
  let root: string

  beforeEach(() => {
    root = mkdtempSync(path.join(os.tmpdir(), 'csp-walker-load-'))
  })

  afterEach(() => {
    rmSync(root, { recursive: true, force: true })
  })

  test('returns null when neither ignore file exists', async () => {
    const spec = await _loadIgnoreForDir(root)
    expect(spec).toBeNull()
  })

  test('combines .gitignore and .cspignore lines', async () => {
    writeFileSync(path.join(root, '.gitignore'), 'a.ts\n')
    writeFileSync(path.join(root, '.cspignore'), 'b.ts\n')
    const spec = await _loadIgnoreForDir(root)
    expect(spec).not.toBeNull()
    expect(spec!.patterns.length).toBe(2)
    expect(spec!.patterns.map(p => p.pattern)).toEqual(['a.ts', 'b.ts'])
  })

  test('skips blank lines and comments', async () => {
    writeFileSync(path.join(root, '.gitignore'), '# comment\n\n*.log\n')
    const spec = await _loadIgnoreForDir(root)
    expect(spec).not.toBeNull()
    expect(spec!.patterns.length).toBe(1)
    expect(spec!.patterns[0]!.pattern).toBe('*.log')
  })
})

describeWithIgnore('_isIgnored', () => {
  let root: string

  beforeEach(() => {
    root = mkdtempSync(path.join(os.tmpdir(), 'csp-walker-isig-'))
  })

  afterEach(() => {
    rmSync(root, { recursive: true, force: true })
  })

  test('returns found=true for negation patterns with file extensions', async () => {
    writeFileSync(path.join(root, '.gitignore'), '*.log\n!special.log\n')
    const spec = await _loadIgnoreForDir(root)
    expect(spec).not.toBeNull()
    const check = _isIgnored(path.join(root, 'special.log'), false, [spec!])
    expect(check.ignored).toBe(false)
    expect(check.found).toBe(true)
  })

  test('returns found=false for negation patterns without file extensions', async () => {
    writeFileSync(path.join(root, '.gitignore'), 'vendor/\n!vendor/keep/\n')
    const spec = await _loadIgnoreForDir(root)
    expect(spec).not.toBeNull()
    // The negation pattern `!vendor/keep/` has no extension — should NOT set found.
    const check = _isIgnored(path.join(root, 'vendor', 'keep'), true, [spec!])
    expect(check.found).toBe(false)
  })

  test('returns ignored=true when pattern matches', async () => {
    writeFileSync(path.join(root, '.gitignore'), '*.log\n')
    const spec = await _loadIgnoreForDir(root)
    expect(spec).not.toBeNull()
    const check = _isIgnored(path.join(root, 'foo.log'), false, [spec!])
    expect(check.ignored).toBe(true)
  })
})
