// Tests for src/indexing/types.ts

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { PersistencePath } from './types.ts'

describe('PersistencePath', () => {
  let dir: string

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'csp-pp-'))
  })
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  it('fromPath produces the expected layout', () => {
    const p = PersistencePath.fromPath(dir)
    expect(p.chunks).toBe(join(dir, 'chunks.json'))
    expect(p.bm25Index).toBe(join(dir, 'bm25_index'))
    expect(p.semanticIndex).toBe(join(dir, 'semantic_index'))
    expect(p.metadata).toBe(join(dir, 'metadata.json'))
  })

  it('nonExisting returns every path when the dir is empty', () => {
    const p = PersistencePath.fromPath(dir)
    expect(p.nonExisting().sort()).toEqual(
      [p.chunks, p.bm25Index, p.semanticIndex, p.metadata].sort(),
    )
  })

  it('nonExisting returns only the truly missing paths', () => {
    const p = PersistencePath.fromPath(dir)
    writeFileSync(p.chunks, '[]')
    mkdirSync(p.bm25Index, { recursive: true })
    expect(p.nonExisting().sort()).toEqual([p.semanticIndex, p.metadata].sort())
  })
})
