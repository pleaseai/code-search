// Tests for src/indexing/create.ts

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { ContentType } from '../types.ts'
import { createIndexFromPath } from './create.ts'
import { makeStubModel } from './dense.ts'

describe('createIndexFromPath', () => {
  let dir: string

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'csp-create-'))
  })
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  it('builds chunks/bm25/semantic indexes for a small TS file', async () => {
    const src = join(dir, 'sample.ts')
    writeFileSync(
      src,
      'export function greet(name: string) {\n  return `hi ${name}`\n}\n',
    )
    const model = makeStubModel(4)
    const result = await createIndexFromPath(dir, { model, displayRoot: dir })
    expect(result.chunks.length).toBeGreaterThan(0)
    // Path is stored relative to displayRoot.
    expect(result.chunks[0]!.filePath).toBe('sample.ts')
    expect(result.semanticIndex.vectors.length).toBe(result.chunks.length)
    expect(result.bm25Index.documents.length).toBe(result.chunks.length)
  })

  it('throws when no supported files are found', async () => {
    // Only an unsupported binary extension present.
    writeFileSync(join(dir, 'data.bin'), 'binary')
    const model = makeStubModel(4)
    await expect(createIndexFromPath(dir, { model })).rejects.toThrow(
      /No supported files found/,
    )
  })

  it('respects an explicit extensions override', async () => {
    writeFileSync(join(dir, 'a.txt'), 'hello world')
    const model = makeStubModel(4)
    const result = await createIndexFromPath(dir, {
      model,
      extensions: ['.txt'],
      content: ContentType.DOCS,
      displayRoot: dir,
    })
    expect(result.chunks.length).toBe(1)
    expect(result.chunks[0]!.filePath).toBe('a.txt')
  })

  it('skips files larger than MAX_FILE_BYTES', async () => {
    // Write 2 MB of code-like content; should be skipped.
    const big = 'a'.repeat(2_000_000)
    writeFileSync(join(dir, 'big.ts'), big)
    writeFileSync(join(dir, 'small.ts'), 'export const x = 1\n')
    const model = makeStubModel(4)
    const result = await createIndexFromPath(dir, { model, displayRoot: dir })
    const paths = result.chunks.map(c => c.filePath)
    expect(paths).toContain('small.ts')
    expect(paths).not.toContain('big.ts')
  })

  it('descends into subdirectories', async () => {
    const sub = join(dir, 'sub')
    mkdirSync(sub)
    writeFileSync(join(sub, 'nested.ts'), 'const a = 1\n')
    const model = makeStubModel(4)
    const result = await createIndexFromPath(dir, { model, displayRoot: dir })
    const paths = result.chunks.map(c => c.filePath)
    expect(paths.some(p => p.endsWith('nested.ts'))).toBe(true)
  })
})
