#!/usr/bin/env node
// Port of src/semble/cli.py
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

// TODO(integration): replace stub when sibling modules land
import { clearIndexCache, loadOrBuildIndex } from './indexing/cache.ts'
import { CspIndex } from './indexing/index.ts'
import { serve } from './mcp/server.ts'
import { clearSavings, formatSavingsReport } from './stats.ts'
import { ContentType } from './types.ts'
import { formatResults, isGitUrl, resolveChunk } from './utils.ts'
import { version } from './version.ts'

export enum Agent {
  Antigravity = 'antigravity',
  Claude = 'claude',
  Commandcode = 'commandcode',
  Copilot = 'copilot',
  Cursor = 'cursor',
  Gemini = 'gemini',
  Kiro = 'kiro',
  Opencode = 'opencode',
  Pi = 'pi',
  Reasonix = 'reasonix',
}

const DEFAULT_AGENT = Agent.Claude
const CLI_DISPATCH_ARGS = new Set([
  'search',
  'find-related',
  'init',
  'savings',
  'clear',
  'index',
  'mcp',
  '-h',
  '--help',
])

const CLEAR_CHOICES = ['all', 'index', 'savings'] as const

const CONTENT_CHOICES = ['code', 'docs', 'config', 'all'] as const

export function _agentPath(agent: Agent): string {
  const baseDir = agent === Agent.Copilot ? '.github' : `.${agent}`
  return `${baseDir}/agents/csp-search.md`
}

export interface ParsedArgs {
  command: string | null
  positional: string[]
  flags: Record<string, string | boolean | string[]>
}

export function parseArgs(argv: string[]): ParsedArgs {
  const positional: string[] = []
  const flags: Record<string, string | boolean | string[]> = {}
  let command: string | null = null
  let i = 0

  if (argv.length > 0 && !argv[0]!.startsWith('-')) {
    command = argv[0]!
    i = 1
  }

  while (i < argv.length) {
    const token = argv[i]!
    if (token === '--') {
      for (let j = i + 1; j < argv.length; j++) {
        positional.push(argv[j]!)
      }
      break
    }
    if (token.startsWith('--')) {
      const eqIdx = token.indexOf('=')
      let name: string
      let value: string | undefined
      if (eqIdx !== -1) {
        name = token.slice(2, eqIdx)
        value = token.slice(eqIdx + 1)
      }
      else {
        name = token.slice(2)
      }
      // collect multi-value flag (e.g. --content code docs)
      if (name === 'content' && value === undefined) {
        const values: string[] = []
        let j = i + 1
        while (j < argv.length && !argv[j]!.startsWith('-')) {
          values.push(argv[j]!)
          j++
        }
        if (values.length > 0) {
          flags[name] = values
          i = j
          continue
        }
      }
      if (value === undefined) {
        // boolean or value-from-next
        const next = argv[i + 1]
        if (next !== undefined && !next.startsWith('-')) {
          flags[name] = next
          i += 2
          continue
        }
        flags[name] = true
        i += 1
        continue
      }
      flags[name] = value
      i += 1
      continue
    }
    if (token.startsWith('-') && token.length > 1) {
      // short flag
      const name = token.slice(1)
      const next = argv[i + 1]
      if (next !== undefined && !next.startsWith('-')) {
        flags[name] = next
        i += 2
        continue
      }
      flags[name] = true
      i += 1
      continue
    }
    positional.push(token)
    i += 1
  }

  return { command, positional, flags }
}

function _getFlag(flags: Record<string, string | boolean | string[]>, ...names: string[]): string | boolean | string[] | undefined {
  for (const name of names) {
    if (name in flags) {
      return flags[name]
    }
  }
  return undefined
}

function _getStringFlag(flags: Record<string, string | boolean | string[]>, ...names: string[]): string | undefined {
  const v = _getFlag(flags, ...names)
  if (typeof v === 'string') {
    return v
  }
  return undefined
}

function _getNumberFlag(flags: Record<string, string | boolean | string[]>, ...names: string[]): number | undefined {
  const s = _getStringFlag(flags, ...names)
  if (s === undefined) {
    return undefined
  }
  const n = Number(s)
  if (Number.isNaN(n)) {
    return undefined
  }
  return n
}

function _getBoolFlag(flags: Record<string, string | boolean | string[]>, ...names: string[]): boolean {
  const v = _getFlag(flags, ...names)
  return v === true
}

function _getContentFlag(flags: Record<string, string | boolean | string[]>): string[] {
  const v = flags.content
  if (Array.isArray(v)) {
    return v
  }
  if (typeof v === 'string') {
    return [v]
  }
  return ['code']
}

export function _resolveContent(content: string[], includeTextFiles: boolean): ContentType[] {
  if (includeTextFiles) {
    process.emitWarning(
      '--include-text-files is deprecated and will be removed in a future version. Use --content all instead.',
      'DeprecationWarning',
    )
  }
  if (includeTextFiles || content.includes('all')) {
    return [ContentType.CODE, ContentType.DOCS, ContentType.CONFIG]
  }
  const result: ContentType[] = []
  for (const c of content) {
    if (c === 'code') {
      result.push(ContentType.CODE)
    }
    else if (c === 'docs') {
      result.push(ContentType.DOCS)
    }
    else if (c === 'config') {
      result.push(ContentType.CONFIG)
    }
    else { throw new Error(`Invalid content type: ${c}. Choices: ${CONTENT_CHOICES.join(', ')}`) }
  }
  return result
}

function _printHelp(): void {
  const help = `csp — Instant local code search for agents.

Usage:
  csp <command> [options]

Commands:
  search <query> [path]         Search a codebase.
  index <path>                  Index and store a codebase.
  find-related <file> <line> [path]  Find code similar to a specific location.
  init                          Write a csp sub-agent file for your coding agent.
  savings                       Show token savings and usage stats.
  clear <all|index|savings>     Clear cached data (savings telemetry).
  mcp [path]                    Start the MCP server (optionally pre-index path).

Common options:
  --top-k <n>, -k <n>           Number of results (default: 5).
  --content <types...>          Content types: code, docs, config, all (default: code).
  --index <path>                Path to a pre-built index.
  --agent <name>, -a <name>     One of: antigravity, claude, commandcode, copilot, cursor, gemini, kiro, opencode, pi, reasonix.
  --force                       Overwrite if file already exists (init).
  -o, --out <path>              Write the pre-built index to this path (index).
  --ref <ref>                   Branch or tag for git URLs (mcp).
  --verbose                     Verbose output (savings).
  --include-text-files          Deprecated. Use --content all instead.

Examples:
  csp search "authentication flow" ./my-project
  csp index ./my-project -o my_index
  csp find-related src/auth.ts 42 ./my-project
  csp init --agent claude
  csp savings --verbose
  csp mcp ./my-project
`
  process.stdout.write(help)
}

interface RunOptions {
  readIndex?: (path: string) => Promise<CspIndex>
  /**
   * Build-or-reuse seam for the auto-cache path (search/find-related without
   * `--index`). Defaults to {@link loadOrBuildIndex}; tests inject it to avoid
   * touching the real `~/.csp` home.
   */
  loadOrBuild?: (source: string, opts: { content: ContentType[], ref?: string | undefined }) => Promise<CspIndex>
  fromPath?: (path: string, opts: { content: ContentType[] }) => Promise<CspIndex>
  fromGit?: (path: string, opts: { content: ContentType[] }) => Promise<CspIndex>
  serveMcp?: (path: string | undefined, opts: { ref?: string | undefined, content: ContentType[] }) => Promise<void>
  writeFileImpl?: (path: string, content: string) => Promise<void>
  readAgentFile?: (agent: Agent) => Promise<string>
  formatSavings?: (opts: { verbose: boolean }) => string
  clearSavings?: () => { path: string, cleared: boolean }
  /**
   * Index-cache clearing seam for `clear index` / `clear all`. Defaults to
   * {@link clearIndexCache} (which targets `~/.csp/index`); tests inject it with
   * a temp `baseDir` so the real home is never touched.
   */
  clearIndex?: () => { path: string, cleared: boolean, entries: number }
  cwd?: () => string
}

export async function _readAgentFile(agent: Agent): Promise<string> {
  const url = new URL(`./agents/${agent}.md`, import.meta.url)
  return readFile(fileURLToPath(url), 'utf8')
}

export async function _runInit(opts: {
  agent?: Agent
  force?: boolean
  cwd?: string
  readAgentFile?: (agent: Agent) => Promise<string>
  writeFileImpl?: (path: string, content: string) => Promise<void>
}): Promise<void> {
  const agent = opts.agent ?? DEFAULT_AGENT
  const force = opts.force ?? false
  const cwd = opts.cwd ?? process.cwd()
  const relDest = _agentPath(agent)
  const dest = resolve(cwd, relDest)

  let exists = false
  try {
    await stat(dest)
    exists = true
  }
  catch {
    exists = false
  }
  if (exists && !force) {
    throw new Error(`${relDest} already exists. Run with --force to overwrite.`)
  }

  await mkdir(dirname(dest), { recursive: true })
  const readAgent = opts.readAgentFile ?? _readAgentFile
  const content = await readAgent(agent)
  const write = opts.writeFileImpl ?? (async (p: string, c: string) => writeFile(p, c, 'utf8'))
  await write(dest, content)
  process.stdout.write(`Created ${relDest}\n`)
}

/**
 * Default auto-cache seam: forward to {@link loadOrBuildIndex}, re-narrowing
 * `ref` so an absent ref is omitted rather than passed as explicit `undefined`
 * (required under `exactOptionalPropertyTypes`).
 */
async function _defaultLoadOrBuild(
  source: string,
  opts: { content: ContentType[], ref?: string | undefined },
): Promise<CspIndex> {
  return loadOrBuildIndex(source, {
    content: opts.content,
    ...(opts.ref !== undefined ? { ref: opts.ref } : {}),
  })
}

async function _runIndex(opts: {
  path: string
  out: string
  content: ContentType[]
  fromPath?: (path: string, opts: { content: ContentType[] }) => Promise<CspIndex>
  fromGit?: (path: string, opts: { content: ContentType[] }) => Promise<CspIndex>
}): Promise<void> {
  const { path, out, content } = opts
  const fromPath = opts.fromPath ?? (async (p: string, o: { content: ContentType[] }) => CspIndex.fromPath(p, o))
  const fromGit = opts.fromGit ?? (async (p: string, o: { content: ContentType[] }) => CspIndex.fromGit(p, o))
  const index = isGitUrl(path)
    ? await fromGit(path, { content })
    : await fromPath(path, { content })
  await mkdir(out, { recursive: true })
  await index.save(out)
}

/** Report the outcome of an index-cache clear to stdout. */
function _reportIndexClear(result: { path: string, cleared: boolean, entries: number }): void {
  process.stdout.write(
    result.cleared
      ? `Cleared ${result.entries} cached index entries at \`${result.path}\`\n`
      : `No index cache found at \`${result.path}\`\n`,
  )
}

/** Report the outcome of a savings clear to stdout. */
function _reportSavingsClear(result: { path: string, cleared: boolean }): void {
  process.stdout.write(
    result.cleared
      ? `Cleared savings at \`${result.path}\`\n`
      : `No savings file found at \`${result.path}\`\n`,
  )
}

/**
 * Run the `clear` subcommand.
 *
 * `clear index` deletes the global on-disk index cache at `~/.csp/index/`.
 * `clear savings` deletes the `~/.csp/savings.jsonl` telemetry file. `clear all`
 * runs **both** as two independent actions — the index root is removed first,
 * then `clearSavings()` is called separately, so removing the index never
 * affects savings and vice versa. The `~/.csp` home itself is never deleted.
 */
export function _runClear(
  type: string,
  clearSavingsImpl: () => { path: string, cleared: boolean } = clearSavings,
  clearIndexImpl: () => { path: string, cleared: boolean, entries: number } = clearIndexCache,
): number {
  if (!(CLEAR_CHOICES as readonly string[]).includes(type)) {
    process.stderr.write(`Invalid clear type: ${type}. Choices: ${CLEAR_CHOICES.join(', ')}\n`)
    return 1
  }

  if (type === 'index' || type === 'all') {
    _reportIndexClear(clearIndexImpl())
  }

  if (type === 'savings' || type === 'all') {
    _reportSavingsClear(clearSavingsImpl())
  }

  return 0
}

export async function runCli(argv: string[], options: RunOptions = {}): Promise<number> {
  // Bare invocation prints help and exits 0; unknown subcommands are handled
  // below (after parsing) so they exit 1.
  if (argv.length === 0) {
    _printHelp()
    return 0
  }

  if (argv[0] === '-h' || argv[0] === '--help') {
    _printHelp()
    return 0
  }

  if (argv[0] === '-V' || argv[0] === '--version') {
    process.stdout.write(`csp ${version}\n`)
    return 0
  }

  try {
    const { command, positional, flags } = parseArgs(argv)

    if (command === null || !CLI_DISPATCH_ARGS.has(command)) {
      process.stderr.write(`Unknown command: ${command ?? '<none>'}\n`)
      _printHelp()
      return 1
    }

    if (command === 'init') {
      const agentRaw = _getStringFlag(flags, 'agent', 'a') ?? DEFAULT_AGENT
      const agent = _coerceAgent(agentRaw)
      const force = _getBoolFlag(flags, 'force')
      await _runInit({
        agent,
        force,
        ...(options.cwd ? { cwd: options.cwd() } : {}),
        ...(options.readAgentFile ? { readAgentFile: options.readAgentFile } : {}),
        ...(options.writeFileImpl ? { writeFileImpl: options.writeFileImpl } : {}),
      })
      return 0
    }

    if (command === 'index') {
      const path = positional[0] ?? '.'
      const out = _getStringFlag(flags, 'out', 'o')
      if (out === undefined) {
        process.stderr.write('--out / -o is required for `index`.\n')
        return 1
      }
      const content = _resolveContent(_getContentFlag(flags), _getBoolFlag(flags, 'include-text-files'))
      await _runIndex({
        path,
        out,
        content,
        ...(options.fromPath ? { fromPath: options.fromPath } : {}),
        ...(options.fromGit ? { fromGit: options.fromGit } : {}),
      })
      return 0
    }

    if (command === 'savings') {
      const verbose = _getBoolFlag(flags, 'verbose')
      const fmt = options.formatSavings ?? formatSavingsReport
      process.stdout.write(fmt({ verbose }))
      return 0
    }

    if (command === 'clear') {
      const type = positional[0]
      if (type === undefined) {
        process.stderr.write(`clear requires a type. Choices: ${CLEAR_CHOICES.join(', ')}\n`)
        return 1
      }
      const clearSavingsImpl = options.clearSavings ?? clearSavings
      const clearIndexImpl = options.clearIndex ?? clearIndexCache
      return _runClear(type, clearSavingsImpl, clearIndexImpl)
    }

    if (command === 'mcp') {
      const path = positional[0]
      const ref = _getStringFlag(flags, 'ref')
      const content = _resolveContent(_getContentFlag(flags), _getBoolFlag(flags, 'include-text-files'))
      const serveImpl = options.serveMcp ?? (async (p, o) => serve(p, o))
      await serveImpl(path, { ref, content })
      return 0
    }

    // search and find-related share index loading
    if (command === 'search' || command === 'find-related') {
      const indexPath = _getStringFlag(flags, 'index')
      let index: CspIndex
      if (indexPath !== undefined) {
        // Explicit `--index`: load the pre-built index verbatim. The auto-cache
        // is intentionally bypassed so an explicit path is always honored.
        const loadImpl = options.readIndex ?? (async (p: string) => CspIndex.loadFromDisk(p))
        index = await loadImpl(indexPath)
      }
      else {
        // No `--index`: route through the on-disk auto-cache, which keys on the
        // source (local path or git URL), content selection, and git ref, then
        // reuses a fresh entry or builds + persists one under `~/.csp/index/`.
        const pathArg = command === 'search' ? positional[1] ?? '.' : positional[2] ?? '.'
        const content = _resolveContent(_getContentFlag(flags), _getBoolFlag(flags, 'include-text-files'))
        const ref = _getStringFlag(flags, 'ref')
        const loadOrBuild = options.loadOrBuild ?? _defaultLoadOrBuild
        index = await loadOrBuild(pathArg, { content, ...(ref !== undefined ? { ref } : {}) })
      }

      const topK = _getNumberFlag(flags, 'top-k', 'k') ?? 5

      if (command === 'search') {
        const query = positional[0]
        if (query === undefined) {
          process.stderr.write('search requires a <query>.\n')
          return 1
        }
        const results = index.search(query, { topK })
        const out = results.length === 0
          ? { error: 'No results found.' }
          : formatResults(query, results)
        process.stdout.write(`${JSON.stringify(out)}\n`)
        return 0
      }

      // find-related
      const filePath = positional[0]
      const lineRaw = positional[1]
      if (filePath === undefined || lineRaw === undefined) {
        process.stderr.write('find-related requires <file_path> <line>.\n')
        return 1
      }
      if (!/^-?\d+$/.test(lineRaw)) {
        process.stderr.write(`line must be an integer, got: ${lineRaw}\n`)
        return 1
      }
      const line = Number.parseInt(lineRaw, 10)
      const chunk = resolveChunk(index.chunks, filePath, line)
      if (chunk === undefined || chunk === null) {
        process.stderr.write(`No chunk found at ${filePath}:${line}.\n`)
        return 1
      }
      const related = index.findRelated(chunk, { topK })
      const out = related.length === 0
        ? { error: `No related chunks found for ${filePath}:${line}.` }
        : formatResults(`Chunks related to ${filePath}:${line}`, related)
      process.stdout.write(`${JSON.stringify(out)}\n`)
      return 0
    }

    // Unreachable: CLI_DISPATCH_ARGS gate above filters unknown commands.
    process.stderr.write(`Unknown command: ${command}\n`)
    _printHelp()
    return 1
  }
  catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    process.stderr.write(`${message}\n`)
    return 1
  }
}

function _coerceAgent(raw: string): Agent {
  const candidates: Agent[] = [
    Agent.Antigravity,
    Agent.Claude,
    Agent.Commandcode,
    Agent.Copilot,
    Agent.Cursor,
    Agent.Gemini,
    Agent.Kiro,
    Agent.Opencode,
    Agent.Pi,
    Agent.Reasonix,
  ]
  for (const a of candidates) {
    if (a === raw) {
      return a
    }
  }
  throw new Error(`Invalid agent: ${raw}. Choices: ${candidates.join(', ')}`)
}

async function main(): Promise<void> {
  const argv = process.argv.slice(2)
  const code = await runCli(argv)
  if (code !== 0) {
    process.exit(code)
  }
}

// Run main only when invoked directly (not when imported as a module / under bun:test)
const invokedDirectly = (() => {
  if (typeof process === 'undefined') {
    return false
  }
  // process.argv[1] points at the entrypoint script — match against this module's URL
  const entry = process.argv[1]
  if (entry === undefined) {
    return false
  }
  try {
    const here = fileURLToPath(import.meta.url)
    return entry === here || entry.endsWith('/cli.ts') || entry.endsWith('/cli.mjs') || entry.endsWith('/cli.js')
  }
  catch {
    return false
  }
})()

if (invokedDirectly) {
  void main()
}
