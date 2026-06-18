// Port of src/semble/chunking/core.py
//
// AST-based chunker built on top of tree-sitter with a line-based fallback.
//
// Tree-sitter integration uses `@kreuzberg/tree-sitter-language-pack`, a NAPI
// binding that exposes the raw `Parser`/`Tree`/`Node` API (see its `index.d.ts`).
// The dependency itself is owned by Unit 0 — we import lazily so this module
// loads even when the package is not yet installed, falling back to the
// line chunker in that case.

import { ALL_LANGUAGES } from '../indexing/files.ts'

export const RECURSION_DEPTH = 500
export const MIN_CHUNK_SIZE = 50

export interface ChunkBoundary {
  start: number
  end: number
}

/** Minimal structural shape of a tree-sitter Node we depend on. */
interface TreeSitterNode {
  startByte: () => number
  endByte: () => number
  childCount: () => number
  child: (index: number) => TreeSitterNode | null
}

interface TreeSitterParser {
  parse: (source: string) => { rootNode: () => TreeSitterNode } | null
}

/** Cache of language → parser (or null when load fails). */
const _parserCache = new Map<string, TreeSitterParser | null>()

/**
 * Lazily load `@kreuzberg/tree-sitter-language-pack`'s `getParser`.
 * Returns null if the dependency is unavailable.
 */
async function _loadGetParser(): Promise<((name: string) => TreeSitterParser) | null> {
  try {
    // eslint-disable-next-line ts/ban-ts-comment
    // @ts-ignore -- optional dep owned by Unit 0
    const mod = await import('@kreuzberg/tree-sitter-language-pack')
    const getParser = (mod as { getParser?: (name: string) => TreeSitterParser }).getParser
    return typeof getParser === 'function' ? getParser : null
  }
  catch {
    return null
  }
}

let _getParserPromise: Promise<((name: string) => TreeSitterParser) | null> | null = null

async function _cachedGetParser(language: string): Promise<TreeSitterParser | null> {
  if (_parserCache.has(language)) {
    return _parserCache.get(language) ?? null
  }
  _getParserPromise ??= _loadGetParser()
  const getParser = await _getParserPromise
  if (getParser === null) {
    _parserCache.set(language, null)
    return null
  }
  try {
    const parser = getParser(language)
    _parserCache.set(language, parser)
    return parser
  }
  catch {
    _parserCache.set(language, null)
    return null
  }
}

/** Visible for tests. */
export function _resetParserCacheForTests(): void {
  _parserCache.clear()
  _getParserPromise = null
}

/** Check if the language is supported by tree-sitter (matches ALL_LANGUAGES). */
export function isSupportedLanguage(language: string): boolean {
  return ALL_LANGUAGES.has(language)
}

/** Merge adjacent chunks up to the desired length. */
export function _mergeAdjacentChunks(
  chunks: readonly ChunkBoundary[],
  desiredLength: number,
): ChunkBoundary[] {
  if (chunks.length === 0) {
    return []
  }

  const merged: ChunkBoundary[] = []

  const first = chunks[0]!
  let currentStart = first.start
  let currentEnd = first.end
  let currentLength = currentEnd - currentStart

  for (let i = 1; i < chunks.length; i++) {
    const group = chunks[i]!
    const { start, end } = group
    const length = end - start

    if (currentLength + length > desiredLength) {
      merged.push({ start: currentStart, end: currentEnd })
      currentStart = start
      currentEnd = end
      currentLength = length
      continue
    }

    currentEnd = end
    currentLength += length
  }

  merged.push({ start: currentStart, end: currentEnd })

  return merged
}

function _children(node: TreeSitterNode): TreeSitterNode[] {
  const count = node.childCount()
  const out: TreeSitterNode[] = []
  for (let i = 0; i < count; i++) {
    const c = node.child(i)
    if (c !== null) {
      out.push(c)
    }
  }
  return out
}

/** Recursively merge and split nodes. */
export function _mergeNodeInner(
  node: TreeSitterNode,
  desiredLength: number,
  depth: number,
): ChunkBoundary[] {
  const children = _children(node)

  // If there are no child nodes, the only thing we can do is return the current node.
  if (children.length === 0) {
    return [{ start: node.startByte(), end: node.endByte() }]
  }

  const length = node.endByte() - node.startByte()

  // Prevent recursion issues. A depth of > 500 is unlikely.
  if (depth > RECURSION_DEPTH) {
    return [{ start: node.startByte(), end: node.endByte() }]
  }

  // Prevent recursing into short chunks.
  if (length < MIN_CHUNK_SIZE) {
    return [{ start: node.startByte(), end: node.endByte() }]
  }

  const groups: ChunkBoundary[] = []
  let index = 0

  while (index < children.length) {
    let child = children[index]!
    const start = child.startByte()
    let end = child.endByte()
    let runLength = end - start

    // Increment the pointer, as we accessed a child node.
    index += 1

    // If this single chunk is longer than the desired length, try to split it again.
    if (runLength > desiredLength) {
      groups.push(..._mergeNodeInner(child, desiredLength, depth + 1))
      continue
    }

    while (index < children.length) {
      // Extend the current group with one or more children, if they fit.
      child = children[index]!
      const childLength = child.endByte() - child.startByte()

      if (runLength + childLength > desiredLength) {
        break
      }

      end = child.endByte()
      runLength += childLength
      index += 1
    }

    groups.push({ start, end })
  }

  return groups
}

/** Recursively turn nodes into chunks, then merge adjacent chunks. */
export function _mergeNode(node: TreeSitterNode, desiredLength: number): ChunkBoundary[] {
  const rawChunks = _mergeNodeInner(node, desiredLength, 0)
  return _mergeAdjacentChunks(rawChunks, desiredLength)
}

/**
 * Split `text` into lines preserving the trailing newline on each line —
 * equivalent to Python's `str.splitlines(keepends=True)`.
 */
function _splitLinesKeepEnds(text: string): string[] {
  if (text.length === 0) {
    return []
  }

  const lines: string[] = []
  let start = 0
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (ch === '\n') {
      lines.push(text.slice(start, i + 1))
      start = i + 1
    }
    else if (ch === '\r') {
      // Handle \r\n and bare \r as line separators (matches Python's splitlines).
      const next = text[i + 1]
      if (next === '\n') {
        lines.push(text.slice(start, i + 2))
        i += 1
        start = i + 1
      }
      else {
        lines.push(text.slice(start, i + 1))
        start = i + 1
      }
    }
  }
  if (start < text.length) {
    lines.push(text.slice(start))
  }

  return lines
}

/** Chunk source code by line. */
export function chunkLines(text: string, desiredLength: number): ChunkBoundary[] {
  if (text.trim().length === 0) {
    return []
  }

  const linesAsGroups: ChunkBoundary[] = []
  let index = 0
  for (const line of _splitLinesKeepEnds(text)) {
    linesAsGroups.push({ start: index, end: index + line.length })
    index += line.length
  }

  return _mergeAdjacentChunks(linesAsGroups, desiredLength)
}

/**
 * Chunk source code via tree-sitter. Returns null when no parser is
 * available for `language` (caller falls back to `chunkLines`).
 *
 * Async because parser loading is lazy — see `_loadGetParser`.
 */
export async function chunk(
  text: string,
  language: string,
  desiredLength: number,
): Promise<ChunkBoundary[] | null> {
  if (text.trim().length === 0) {
    return []
  }

  const parser = await _cachedGetParser(language)
  if (parser === null) {
    return null
  }

  const tree = parser.parse(text)
  if (tree === null) {
    return null
  }
  const root = tree.rootNode()

  const asBytes = new TextEncoder().encode(text)
  const decoder = new TextDecoder('utf-8')

  // Convert byte offsets to character offsets in a single pass. Boundaries are
  // sorted by their start offset, so we maintain running byte/char cursors and
  // decode each byte exactly once — avoids O(M×N) re-decoding the prefix per
  // chunk.
  const chunks: ChunkBoundary[] = []
  let cursorByte = 0
  let cursorChar = 0
  const byteToChar = (byteOffset: number): number => {
    if (byteOffset > cursorByte) {
      cursorChar += decoder.decode(asBytes.subarray(cursorByte, byteOffset)).length
      cursorByte = byteOffset
    }
    return cursorChar
  }

  for (const boundary of _mergeNode(root, desiredLength)) {
    const startChar = byteToChar(boundary.start)
    const endChar = byteToChar(boundary.end)
    chunks.push({ start: startChar, end: endChar })
  }

  return chunks
}
