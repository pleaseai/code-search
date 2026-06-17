// Port of (none) — unit tests for src/cli.ts
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'

import { _agentPath, _readAgentFile, _resolveContent, _runInit, Agent, parseArgs, runCli } from './cli.ts'
import type { CspIndex } from './indexing/index.ts'
import { ContentType, type SearchResult } from './types.ts'

describe('Agent enum', () => {
  test('enum values', () => {
    expect(String(Agent.Antigravity)).toBe('antigravity')
    expect(String(Agent.Claude)).toBe('claude')
    expect(String(Agent.Commandcode)).toBe('commandcode')
    expect(String(Agent.Copilot)).toBe('copilot')
    expect(String(Agent.Cursor)).toBe('cursor')
    expect(String(Agent.Gemini)).toBe('gemini')
    expect(String(Agent.Kiro)).toBe('kiro')
    expect(String(Agent.Opencode)).toBe('opencode')
    expect(String(Agent.Pi)).toBe('pi')
    expect(String(Agent.Reasonix)).toBe('reasonix')
  })
})

describe('_agentPath', () => {
  test('claude → .claude/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Claude)).toBe('.claude/agents/csp-search.md')
  })
  test('copilot → .github/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Copilot)).toBe('.github/agents/csp-search.md')
  })
  test('cursor → .cursor/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Cursor)).toBe('.cursor/agents/csp-search.md')
  })
  test('opencode → .opencode/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Opencode)).toBe('.opencode/agents/csp-search.md')
  })
  test('antigravity → .antigravity/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Antigravity)).toBe('.antigravity/agents/csp-search.md')
  })
  test('reasonix → .reasonix/agents/csp-search.md', () => {
    expect(_agentPath(Agent.Reasonix)).toBe('.reasonix/agents/csp-search.md')
  })
})

describe('parseArgs', () => {
  test('subcommand and positional', () => {
    const r = parseArgs(['search', 'foo', '.'])
    expect(r.command).toBe('search')
    expect(r.positional).toEqual(['foo', '.'])
  })
  test('--flag value', () => {
    const r = parseArgs(['index', '.', '--out', 'idx'])
    expect(r.flags['out']).toBe('idx')
  })
  test('--flag=value', () => {
    const r = parseArgs(['search', 'q', '--top-k=10'])
    expect(r.flags['top-k']).toBe('10')
  })
  test('boolean flag', () => {
    const r = parseArgs(['savings', '--verbose'])
    expect(r.flags['verbose']).toBe(true)
  })
  test('multi-value --content', () => {
    const r = parseArgs(['search', 'q', '--content', 'code', 'docs'])
    expect(r.flags['content']).toEqual(['code', 'docs'])
  })
  test('short -k', () => {
    const r = parseArgs(['search', 'q', '-k', '20'])
    expect(r.flags['k']).toBe('20')
  })
})

describe('_resolveContent', () => {
  test('default code', () => {
    expect(_resolveContent(['code'], false)).toEqual([ContentType.CODE])
  })
  test('all expands', () => {
    expect(_resolveContent(['all'], false)).toEqual([ContentType.CODE, ContentType.DOCS, ContentType.CONFIG])
  })
  test('--include-text-files expands like all', () => {
    expect(_resolveContent(['code'], true)).toEqual([ContentType.CODE, ContentType.DOCS, ContentType.CONFIG])
  })
  test('multiple types', () => {
    expect(_resolveContent(['code', 'docs'], false)).toEqual([ContentType.CODE, ContentType.DOCS])
  })
  test('unknown throws', () => {
    expect(() => _resolveContent(['bogus'], false)).toThrow()
  })
})

describe('runCli --help', () => {
  test('help mentions all subcommands', async () => {
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['--help'])
      expect(code).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
    }
    const out = writes.join('')
    expect(out).toContain('search')
    expect(out).toContain('index')
    expect(out).toContain('find-related')
    expect(out).toContain('init')
    expect(out).toContain('savings')
    expect(out).toContain('mcp')
  })
})

describe('csp init', () => {
  let tmpDir: string
  let origCwd: string

  beforeEach(async () => {
    tmpDir = await mkdtemp(join(tmpdir(), 'csp-cli-test-'))
    origCwd = process.cwd()
    process.chdir(tmpDir)
  })

  afterEach(async () => {
    process.chdir(origCwd)
    await rm(tmpDir, { recursive: true, force: true })
  })

  test('--agent claude writes .claude/agents/csp-search.md', async () => {
    await _runInit({
      agent: Agent.Claude,
      cwd: tmpDir,
      readAgentFile: async () => '# stub agent file\n',
    })
    const path = join(tmpDir, '.claude/agents/csp-search.md')
    expect(existsSync(path)).toBe(true)
    const content = await readFile(path, 'utf8')
    expect(content).toBe('# stub agent file\n')
  })

  test('--agent copilot writes .github/agents/csp-search.md', async () => {
    await _runInit({
      agent: Agent.Copilot,
      cwd: tmpDir,
      readAgentFile: async () => '# stub copilot\n',
    })
    const path = join(tmpDir, '.github/agents/csp-search.md')
    expect(existsSync(path)).toBe(true)
  })

  test('without --force errors if file exists', async () => {
    await _runInit({
      agent: Agent.Claude,
      cwd: tmpDir,
      readAgentFile: async () => 'first\n',
    })
    // Second call should reject with an "already exists" error — callers
    // (i.e. runCli) translate this into exit code 1 + stderr message.
    await expect(_runInit({
      agent: Agent.Claude,
      cwd: tmpDir,
      readAgentFile: async () => 'second\n',
    })).rejects.toThrow('already exists')
    // Original content preserved.
    const content = await readFile(join(tmpDir, '.claude/agents/csp-search.md'), 'utf8')
    expect(content).toBe('first\n')
  })

  test('--force overwrites', async () => {
    await _runInit({
      agent: Agent.Claude,
      cwd: tmpDir,
      readAgentFile: async () => 'first\n',
    })
    await _runInit({
      agent: Agent.Claude,
      force: true,
      cwd: tmpDir,
      readAgentFile: async () => 'second\n',
    })
    const content = await readFile(join(tmpDir, '.claude/agents/csp-search.md'), 'utf8')
    expect(content).toBe('second\n')
  })
})

describe('csp search (stub-mocked)', () => {
  test('calls index.search with topK', async () => {
    let captured: { query?: string, topK?: number } = {}
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      search: async (query: string, opts?: { topK?: number }): Promise<SearchResult[]> => {
        captured = { query, ...(opts ?? {}) }
        return []
      },
    }
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['search', 'foo', '.', '-k', '7'], {
        loadOrBuild: async () => fakeIndex as CspIndex,
      })
      expect(code).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
    }
    expect(captured).toEqual({ query: 'foo', topK: 7 })
    // Output should be JSON {"error":"No results found."}
    const out = writes.join('').trim()
    expect(JSON.parse(out)).toEqual({ error: 'No results found.' })
  })

  test('formats non-empty results as JSON', async () => {
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      search: async () => [
        {
          chunk: { content: 'def foo()', filePath: 'a.py', startLine: 1, endLine: 3, language: 'python' },
          score: 0.9,
          // Mirrors search.ts's snake_case wire format that utils.formatResults consumes.
          toDict: () => ({
            chunk: {
              content: 'def foo()',
              file_path: 'a.py',
              start_line: 1,
              end_line: 3,
              language: 'python',
              location: 'a.py:1-3',
            },
            score: 0.9,
          }),
        },
      ],
    }
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      await runCli(['search', 'foo', '.'], {
        loadOrBuild: async () => fakeIndex as CspIndex,
      })
    }
    finally {
      process.stdout.write = origWrite
    }
    const out = JSON.parse(writes.join('').trim())
    expect(out.query).toBe('foo')
    expect(out.results).toHaveLength(1)
    expect(out.results[0].chunk.file_path).toBe('a.py')
    expect(out.results[0].chunk.location).toBe('a.py:1-3')
  })
})

describe('csp savings', () => {
  test('prints the report', async () => {
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['savings'], {
        formatSavings: ({ verbose }) => `SAVINGS verbose=${verbose ? '1' : '0'}`,
      })
      expect(code).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
    }
    expect(writes.join('')).toBe('SAVINGS verbose=0')
  })

  test('--verbose is forwarded', async () => {
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      await runCli(['savings', '--verbose'], {
        formatSavings: ({ verbose }) => `SAVINGS verbose=${verbose ? '1' : '0'}`,
      })
    }
    finally {
      process.stdout.write = origWrite
    }
    expect(writes.join('')).toBe('SAVINGS verbose=1')
  })
})

describe('csp clear', () => {
  function captureStdout(): { writes: string[], restore: () => void } {
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    return { writes, restore: () => { process.stdout.write = origWrite } }
  }

  test('clear savings deletes the file and reports the path', async () => {
    const { writes, restore } = captureStdout()
    let called = 0
    try {
      const code = await runCli(['clear', 'savings'], {
        clearSavings: () => { called++; return { path: '/tmp/x/savings.jsonl', cleared: true } },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(called).toBe(1)
    expect(writes.join('')).toContain('Cleared savings at `/tmp/x/savings.jsonl`')
  })

  test('clear savings reports when no file exists', async () => {
    const { writes, restore } = captureStdout()
    try {
      await runCli(['clear', 'savings'], {
        clearSavings: () => ({ path: '/tmp/x/savings.jsonl', cleared: false }),
      })
    }
    finally {
      restore()
    }
    expect(writes.join('')).toContain('No savings file found at `/tmp/x/savings.jsonl`')
  })

  test('clear index notes there is no managed index cache', async () => {
    const { writes, restore } = captureStdout()
    let called = 0
    try {
      await runCli(['clear', 'index'], {
        clearSavings: () => { called++; return { path: '/tmp/x/savings.jsonl', cleared: true } },
      })
    }
    finally {
      restore()
    }
    expect(called).toBe(0) // index-only must not touch savings
    expect(writes.join('')).toContain('No index cache to clear')
  })

  test('clear all clears savings and notes the index', async () => {
    const { writes, restore } = captureStdout()
    try {
      await runCli(['clear', 'all'], {
        clearSavings: () => ({ path: '/tmp/x/savings.jsonl', cleared: true }),
      })
    }
    finally {
      restore()
    }
    const out = writes.join('')
    expect(out).toContain('No index cache to clear')
    expect(out).toContain('Cleared savings at')
  })

  test('clear with an invalid type exits 1', async () => {
    const code = await runCli(['clear', 'bogus'], {
      clearSavings: () => ({ path: '/tmp/x/savings.jsonl', cleared: true }),
    })
    expect(code).toBe(1)
  })

  test('clear with no type exits 1', async () => {
    const code = await runCli(['clear'], {
      clearSavings: () => ({ path: '/tmp/x/savings.jsonl', cleared: true }),
    })
    expect(code).toBe(1)
  })
})

describe('csp mcp', () => {
  test('dispatches to serve with path and content', async () => {
    let captured: { path?: string | undefined, ref?: string | undefined, content?: ContentType[] } = {}
    const code = await runCli(['mcp', '.', '--ref', 'main', '--content', 'all'], {
      serveMcp: async (p, o) => {
        captured = { path: p, ref: o.ref, content: o.content }
      },
    })
    expect(code).toBe(0)
    expect(captured.path).toBe('.')
    expect(captured.ref).toBe('main')
    expect(captured.content).toEqual([ContentType.CODE, ContentType.DOCS, ContentType.CONFIG])
  })

  test('mcp with no path forwards undefined', async () => {
    let captured: { path?: string | undefined } = {}
    const code = await runCli(['mcp'], {
      serveMcp: async (p) => {
        captured = { path: p }
      },
    })
    expect(code).toBe(0)
    expect(captured.path).toBeUndefined()
  })
})

describe('csp find-related validates line', () => {
  test('non-integer line errors with code 1', async () => {
    const errs: string[] = []
    const origStderr = process.stderr.write.bind(process.stderr)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    try {
      const code = await runCli(['find-related', 'src/auth.ts', '42abc', '.'], {
        loadOrBuild: async () => ({ chunks: [] }) as unknown as CspIndex,
      })
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origStderr
    }
    expect(errs.join('')).toContain('line must be an integer')
  })
})

describe('_readAgentFile', () => {
  test('reads src/agents/claude.md', async () => {
    const text = await _readAgentFile(Agent.Claude)
    expect(text.length).toBeGreaterThan(0)
    expect(text).toContain('csp')
  })
  test('a bundled template exists for every agent', async () => {
    for (const agent of Object.values(Agent)) {
      const text = await _readAgentFile(agent)
      expect(text).toContain('name: csp-search')
      expect(text).toContain('csp search')
    }
  })
})

describe('runCli error handling', () => {
  test('unknown subcommand returns exit 1', async () => {
    const errs: string[] = []
    const outs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    const origOut = process.stdout.write.bind(process.stdout)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    process.stdout.write = ((chunk: string | Uint8Array) => {
      outs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['bogus-cmd'])
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origErr
      process.stdout.write = origOut
    }
    expect(errs.join('')).toContain('Unknown command: bogus-cmd')
  })

  test('invalid --agent returns exit 1 with stderr message', async () => {
    const errs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    try {
      const code = await runCli(['init', '--agent', 'bogus'])
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origErr
    }
    expect(errs.join('')).toContain('Invalid agent: bogus')
  })

  test('invalid --content returns exit 1 with stderr message', async () => {
    const errs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    try {
      const code = await runCli(['search', 'foo', '--content', 'bogus'], {
        loadOrBuild: async () => ({ chunks: [] }) as unknown as CspIndex,
      })
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origErr
    }
    expect(errs.join('')).toContain('Invalid content type: bogus')
  })

  test('init rejection is translated to exit 1 by runCli', async () => {
    const tmp = await mkdtemp(join(tmpdir(), 'csp-cli-runcli-'))
    const errs: string[] = []
    const outs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    const origOut = process.stdout.write.bind(process.stdout)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    process.stdout.write = ((chunk: string | Uint8Array) => {
      outs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      // First run: succeeds.
      const code1 = await runCli(['init', '--agent', 'claude'], {
        cwd: () => tmp,
        readAgentFile: async () => '# stub\n',
      })
      expect(code1).toBe(0)
      // Second run without --force: should exit 1 with stderr message, not crash.
      const code2 = await runCli(['init', '--agent', 'claude'], {
        cwd: () => tmp,
        readAgentFile: async () => '# stub\n',
      })
      expect(code2).toBe(1)
    }
    finally {
      process.stderr.write = origErr
      process.stdout.write = origOut
      await rm(tmp, { recursive: true, force: true })
    }
    expect(errs.join('')).toContain('already exists')
  })
})

describe('csp index --content', () => {
  test('passes resolved content types to fromPath', async () => {
    let captured: { path?: string, content?: ContentType[] } = {}
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      save: async () => {
        // no-op
      },
    }
    const tmp = await mkdtemp(join(tmpdir(), 'csp-cli-index-'))
    try {
      const code = await runCli(['index', '.', '-o', join(tmp, 'idx'), '--content', 'all'], {
        fromPath: async (p, o) => {
          captured = { path: p, content: o.content }
          return fakeIndex as CspIndex
        },
      })
      expect(code).toBe(0)
    }
    finally {
      await rm(tmp, { recursive: true, force: true })
    }
    expect(captured.path).toBe('.')
    expect(captured.content).toEqual([ContentType.CODE, ContentType.DOCS, ContentType.CONFIG])
  })
})

describe('csp index -o (explicit path persistence)', () => {
  test('saves the built index to the explicit -o directory', async () => {
    let savedTo: string | undefined
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      save: async (dir: string) => { savedTo = dir },
    }
    const tmp = await mkdtemp(join(tmpdir(), 'csp-cli-index-out-'))
    const out = join(tmp, 'idx')
    try {
      const code = await runCli(['index', '.', '-o', out], {
        fromPath: async () => fakeIndex as CspIndex,
      })
      expect(code).toBe(0)
    }
    finally {
      await rm(tmp, { recursive: true, force: true })
    }
    // The explicit -o path must be the directory passed to save (no cache rerouting).
    expect(savedTo).toBe(out)
  })

  test('without -o keeps the required-flag error and exits 1', async () => {
    const errs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    try {
      const code = await runCli(['index', '.'], {
        fromPath: async () => ({ chunks: [], save: async () => {} }) as unknown as CspIndex,
      })
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origErr
    }
    expect(errs.join('')).toContain('--out / -o is required for `index`')
  })
})

describe('csp search/find-related --index (explicit path respected)', () => {
  test('search --index loads via loadFromDisk seam with the explicit path', async () => {
    let loadedFrom: string | undefined
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      search: (): SearchResult[] => [],
    }
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['search', 'foo', '--index', '/some/explicit/idx'], {
        readIndex: async (p: string) => { loadedFrom = p; return fakeIndex as CspIndex },
        // fromPath provided to prove it is NOT used when --index is set.
        fromPath: async () => { throw new Error('fromPath must not run when --index is given') },
      })
      expect(code).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
    }
    expect(loadedFrom).toBe('/some/explicit/idx')
    expect(JSON.parse(writes.join('').trim())).toEqual({ error: 'No results found.' })
  })

  test('find-related --index loads via loadFromDisk seam with the explicit path', async () => {
    let loadedFrom: string | undefined
    const seedChunk = { content: 'x', filePath: 'a.ts', startLine: 1, endLine: 5, language: 'typescript' }
    const fakeIndex: Partial<CspIndex> = {
      chunks: [seedChunk],
      findRelated: (): SearchResult[] => [],
    }
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      const code = await runCli(['find-related', 'a.ts', '2', '--index', '/explicit/idx2'], {
        readIndex: async (p: string) => { loadedFrom = p; return fakeIndex as CspIndex },
        fromPath: async () => { throw new Error('fromPath must not run when --index is given') },
      })
      expect(code).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
    }
    expect(loadedFrom).toBe('/explicit/idx2')
  })

  test('search --index with a missing path surfaces a clear error and exits 1', async () => {
    const errs: string[] = []
    const origErr = process.stderr.write.bind(process.stderr)
    process.stderr.write = ((chunk: string | Uint8Array) => {
      errs.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stderr.write
    const missing = join(tmpdir(), `csp-no-such-index-${Date.now()}`)
    try {
      // No readIndex seam → real CspIndex.loadFromDisk runs and must throw a clear error.
      const code = await runCli(['search', 'foo', '--index', missing])
      expect(code).toBe(1)
    }
    finally {
      process.stderr.write = origErr
    }
    expect(errs.join('')).toContain('Index not found:')
    expect(errs.join('')).toContain(missing)
  })
})

describe('csp index -o → search --index (real roundtrip, no seams)', () => {
  test('persisted index is loadable and searchable via the explicit path', async () => {
    const tmp = await mkdtemp(join(tmpdir(), 'csp-cli-roundtrip-'))
    const out = join(tmp, 'idx')
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    try {
      // Build a real CspIndex from a tiny source dir, persist via `csp index -o`.
      const src = join(tmp, 'src')
      await mkdir(src, { recursive: true })
      await writeFile(join(src, 'auth.ts'), 'export function login(user: string) { return user }\n', 'utf8')
      const idxCode = await runCli(['index', src, '-o', out])
      expect(idxCode).toBe(0)
      // The manifest proves persistence happened at the explicit path.
      expect(existsSync(join(out, 'manifest.json'))).toBe(true)

      // Load it back through the explicit --index path and search.
      const searchCode = await runCli(['search', 'login', '--index', out, '-k', '3'])
      expect(searchCode).toBe(0)
    }
    finally {
      process.stdout.write = origWrite
      await rm(tmp, { recursive: true, force: true })
    }
    // A non-empty result set (or an explicit "No results") must be valid JSON.
    const out2 = JSON.parse(writes.join('').trim().split('\n').pop() ?? '{}')
    expect(out2).toBeDefined()
  })
})

describe('csp search/find-related (no --index) auto-caches via loadOrBuildIndex (T011)', () => {
  function captureStdout(): { writes: string[], restore: () => void } {
    const writes: string[] = []
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = ((chunk: string | Uint8Array) => {
      writes.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
      return true
    }) as typeof process.stdout.write
    return { writes, restore: () => { process.stdout.write = origWrite } }
  }

  test('search without --index routes through the loadOrBuild seam with source + content + topK', async () => {
    let captured: { source?: string, content?: ContentType[], ref?: string | undefined } = {}
    const fakeIndex: Partial<CspIndex> = {
      chunks: [],
      search: (): SearchResult[] => [],
    }
    const { writes, restore } = captureStdout()
    try {
      const code = await runCli(['search', 'foo', './my-project', '-k', '3'], {
        loadOrBuild: async (source, opts) => {
          captured = { source, content: opts.content, ref: opts.ref }
          return fakeIndex as CspIndex
        },
        // fromPath must NOT be used for the build branch anymore.
        fromPath: async () => { throw new Error('fromPath must not run when auto-cache is wired') },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(captured.source).toBe('./my-project')
    expect(captured.content).toEqual([ContentType.CODE])
    expect(JSON.parse(writes.join('').trim())).toEqual({ error: 'No results found.' })
  })

  test('search without a path argument defaults the source to "."', async () => {
    let capturedSource: string | undefined
    const fakeIndex: Partial<CspIndex> = { chunks: [], search: (): SearchResult[] => [] }
    const { restore } = captureStdout()
    try {
      const code = await runCli(['search', 'foo'], {
        loadOrBuild: async (source) => { capturedSource = source; return fakeIndex as CspIndex },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(capturedSource).toBe('.')
  })

  test('find-related without --index routes through the loadOrBuild seam with its path source', async () => {
    let capturedSource: string | undefined
    const seedChunk = { content: 'x', filePath: 'a.ts', startLine: 1, endLine: 5, language: 'typescript' }
    const fakeIndex: Partial<CspIndex> = {
      chunks: [seedChunk],
      findRelated: (): SearchResult[] => [],
    }
    const { restore } = captureStdout()
    try {
      const code = await runCli(['find-related', 'a.ts', '2', './repo'], {
        loadOrBuild: async (source) => { capturedSource = source; return fakeIndex as CspIndex },
        fromPath: async () => { throw new Error('fromPath must not run when auto-cache is wired') },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(capturedSource).toBe('./repo')
  })

  test('--index still bypasses the auto-cache seam (T008 guarantee preserved)', async () => {
    let loadedFrom: string | undefined
    let autoCacheCalled = false
    const fakeIndex: Partial<CspIndex> = { chunks: [], search: (): SearchResult[] => [] }
    const { restore } = captureStdout()
    try {
      const code = await runCli(['search', 'foo', '--index', '/explicit/idx'], {
        readIndex: async (p: string) => { loadedFrom = p; return fakeIndex as CspIndex },
        loadOrBuild: async () => { autoCacheCalled = true; return fakeIndex as CspIndex },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(loadedFrom).toBe('/explicit/idx')
    expect(autoCacheCalled).toBe(false)
  })

  test('ref flag is forwarded to the loadOrBuild seam', async () => {
    let capturedRef: string | undefined
    const fakeIndex: Partial<CspIndex> = { chunks: [], search: (): SearchResult[] => [] }
    const { restore } = captureStdout()
    try {
      const code = await runCli(['search', 'foo', 'https://github.com/o/r', '--ref', 'v1.2.3'], {
        loadOrBuild: async (_source, opts) => { capturedRef = opts.ref; return fakeIndex as CspIndex },
      })
      expect(code).toBe(0)
    }
    finally {
      restore()
    }
    expect(capturedRef).toBe('v1.2.3')
  })
})
