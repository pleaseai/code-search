// Port of (none) — unit tests for src/cli.ts
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'

import { _agentPath, _readAgentFile, _resolveContent, _runInit, Agent, parseArgs, runCli } from './cli.ts'
import type { CspIndex } from './indexing/index.ts'
import { ContentType, type SearchResult } from './types.ts'

describe('Agent enum', () => {
  test('enum values', () => {
    expect(String(Agent.Claude)).toBe('claude')
    expect(String(Agent.Copilot)).toBe('copilot')
    expect(String(Agent.Cursor)).toBe('cursor')
    expect(String(Agent.Gemini)).toBe('gemini')
    expect(String(Agent.Kiro)).toBe('kiro')
    expect(String(Agent.Opencode)).toBe('opencode')
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
    // Second call should exit with code 1; we intercept process.exit.
    let exitCode: number | undefined
    const origExit = process.exit
    process.exit = ((code?: number) => {
      exitCode = code
      throw new Error('__test_exit__')
    }) as typeof process.exit
    const origStderr = process.stderr.write.bind(process.stderr)
    process.stderr.write = (() => true) as typeof process.stderr.write
    try {
      await _runInit({
        agent: Agent.Claude,
        cwd: tmpDir,
        readAgentFile: async () => 'second\n',
      })
    }
    catch (err) {
      // Expected: we threw inside the fake exit.
      expect((err as Error).message).toBe('__test_exit__')
    }
    finally {
      process.exit = origExit
      process.stderr.write = origStderr
    }
    expect(exitCode).toBe(1)
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
        fromPath: async () => fakeIndex as CspIndex,
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
        fromPath: async () => fakeIndex as CspIndex,
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
        fromPath: async () => ({ chunks: [] }) as unknown as CspIndex,
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
})
