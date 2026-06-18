// Port of src/semble/index/create.py

import type { Chunk } from '../types.ts'
import type { Model } from './dense.ts'
import { readFileSync, statSync } from 'node:fs'
import { relative } from 'node:path'
import { chunkSource } from '../chunking/chunk-source.ts'
import { tokenize } from '../tokens.ts'
import { ContentType } from '../types.ts'
import { embedChunks, SelectableBasicBackend } from './dense.ts'
import { walkFiles } from './file-walker.ts'
import { detectLanguage, getExtensions } from './files.ts'
import { Bm25Index, enrichForBm25 } from './sparse.ts'

/** 1 MB max file size to read and index. */
export const MAX_FILE_BYTES = 1_000_000

export interface CreateIndexOptions {
  model: Model
  extensions?: readonly string[]
  content?: ContentType | readonly ContentType[]
  displayRoot?: string
}

export interface CreateIndexResult {
  bm25Index: Bm25Index
  semanticIndex: SelectableBasicBackend
  chunks: Chunk[]
}

/**
 * Create an index from a resolved directory.
 *
 * Walks files matching `extensions`, chunks them, enriches text for BM25,
 * tokenizes it, embeds chunks, and returns the populated indexes.
 *
 * @throws if no chunks are produced.
 */
export async function createIndexFromPath(
  path: string,
  options: CreateIndexOptions,
): Promise<CreateIndexResult> {
  const { model, extensions, content, displayRoot } = options

  const normalized: readonly ContentType[] = normalizeContent(content)
  const resolvedExtensions = getExtensions(normalized, extensions)

  const chunks: Chunk[] = []
  for await (const filePath of walkFiles(path, resolvedExtensions)) {
    const language = detectLanguage(filePath)
    let size: number
    try {
      size = statSync(filePath).size
    }
    catch {
      continue
    }
    if (size > MAX_FILE_BYTES) {
      continue
    }
    let source: string
    try {
      source = readFileSync(filePath, 'utf8')
    }
    catch {
      continue
    }
    const chunkPath = displayRoot !== undefined ? relative(displayRoot, filePath) : filePath
    chunks.push(...(await chunkSource(source, chunkPath, language ?? null)))
  }

  if (chunks.length === 0) {
    throw new Error(`No supported files found under ${path}.`)
  }

  const embeddings = embedChunks(model, chunks)
  const bm25Index = Bm25Index.build(chunks.map(c => tokenize(enrichForBm25(c))))
  const semanticIndex = new SelectableBasicBackend(embeddings)

  return { bm25Index, semanticIndex, chunks }
}

function normalizeContent(
  content: ContentType | readonly ContentType[] | undefined,
): readonly ContentType[] {
  if (content === undefined) {
    // Default: code-only. Mirrors _DEFAULT_CONTENT in semble.
    return [ContentType.CODE]
  }
  if (Array.isArray(content)) {
    return content as readonly ContentType[]
  }
  return [content as ContentType]
}
