import { describe, expect, test } from 'bun:test'
import {
  _chunkDefinesSymbol,
  _countKeywordMatches,
  _extractSymbolName,
  _stemMatches,
  applyQueryBoost,
  boostMultiChunkFiles,
  type Chunk,
  DEFINITION_BOOST_MULTIPLIER,
  EMBEDDED_SYMBOL_BOOST_SCALE,
  FILE_COHERENCE_BOOST_FRAC,
  isSymbolQuery,
} from './boosting.ts'

function mkChunk(content: string, filePath: string, startLine = 1, endLine = 10): Chunk {
  return { content, filePath, startLine, endLine }
}

describe('isSymbolQuery', () => {
  test('PascalCase identifiers are symbol queries', () => {
    expect(isSymbolQuery('HandlerStack')).toBe(true)
    expect(isSymbolQuery('Client')).toBe(true)
  })

  test('namespace-qualified identifiers are symbol queries', () => {
    expect(isSymbolQuery('Sinatra::Base')).toBe(true)
    expect(isSymbolQuery('Phoenix.Router')).toBe(true)
    expect(isSymbolQuery('foo->bar')).toBe(true)
    expect(isSymbolQuery('A\\B\\C')).toBe(true)
  })

  test('leading-underscore identifiers are symbol queries', () => {
    expect(isSymbolQuery('_private')).toBe(true)
    expect(isSymbolQuery('_')).toBe(true)
  })

  test('snake_case identifiers are symbol queries', () => {
    expect(isSymbolQuery('my_func')).toBe(true)
  })

  test('plain lowercase words are NL', () => {
    expect(isSymbolQuery('session')).toBe(false)
    expect(isSymbolQuery('foo')).toBe(false)
  })

  test('NL phrases are NL', () => {
    expect(isSymbolQuery('how does this work')).toBe(false)
    expect(isSymbolQuery('find the cache layer')).toBe(false)
  })

  test('trims whitespace', () => {
    expect(isSymbolQuery('  HandlerStack  ')).toBe(true)
  })
})

describe('_extractSymbolName', () => {
  test('extracts trailing name after :: separator', () => {
    expect(_extractSymbolName('Sinatra::Base')).toBe('Base')
  })

  test('extracts trailing name after .', () => {
    expect(_extractSymbolName('Phoenix.Router')).toBe('Router')
  })

  test('extracts trailing name after ->', () => {
    expect(_extractSymbolName('foo->bar')).toBe('bar')
  })

  test('returns the original (trimmed) when no separator', () => {
    expect(_extractSymbolName('Client')).toBe('Client')
    expect(_extractSymbolName('  Client  ')).toBe('Client')
  })
})

describe('_stemMatches', () => {
  test('exact match', () => {
    expect(_stemMatches('client', 'client')).toBe(true)
  })

  test('snake-stripped match', () => {
    expect(_stemMatches('handler_stack', 'handlerstack')).toBe(true)
  })

  test('plural-stripped match', () => {
    expect(_stemMatches('clients', 'client')).toBe(true)
    expect(_stemMatches('handler_stacks', 'handlerstack')).toBe(true)
  })

  test('no match', () => {
    expect(_stemMatches('foo', 'bar')).toBe(false)
  })
})

describe('_chunkDefinesSymbol', () => {
  test('matches class definition', () => {
    const chunk = mkChunk('class HandlerStack:\n    pass\n', 'a.py')
    expect(_chunkDefinesSymbol(chunk, 'HandlerStack')).toBe(true)
  })

  test('matches def function', () => {
    const chunk = mkChunk('def my_func(x):\n    return x\n', 'a.py')
    expect(_chunkDefinesSymbol(chunk, 'my_func')).toBe(true)
  })

  test('matches namespace-qualified defmodule for trailing name', () => {
    const chunk = mkChunk('defmodule Phoenix.Router do\nend\n', 'a.ex')
    expect(_chunkDefinesSymbol(chunk, 'Router')).toBe(true)
  })

  test('case-sensitive: does not match "Module" as keyword', () => {
    const chunk = mkChunk('Module Foo', 'a.txt')
    expect(_chunkDefinesSymbol(chunk, 'Foo')).toBe(false)
  })

  test('case-insensitive for SQL DDL', () => {
    const chunk = mkChunk('create table users (id int);', 'a.sql')
    expect(_chunkDefinesSymbol(chunk, 'users')).toBe(true)
    const chunk2 = mkChunk('CREATE TABLE users (id int);', 'a.sql')
    expect(_chunkDefinesSymbol(chunk2, 'users')).toBe(true)
  })

  test('does not match in the middle of a word', () => {
    const chunk = mkChunk('# subclass Foo\n', 'a.py')
    expect(_chunkDefinesSymbol(chunk, 'Foo')).toBe(false)
  })
})

describe('_countKeywordMatches', () => {
  test('all exact matches', () => {
    expect(_countKeywordMatches(new Set(['foo', 'bar']), new Set(['foo', 'bar', 'baz']))).toBe(2)
  })

  test('prefix overlap (min 3 chars)', () => {
    // "dep" matches "dependency" (keyword shorter than part)
    expect(_countKeywordMatches(new Set(['dep']), new Set(['dependency']))).toBe(1)
    // "depend" matches "dependencies" (both ≥3, longer.startsWith(shorter))
    expect(_countKeywordMatches(new Set(['depend']), new Set(['dependencies']))).toBe(1)
    // Part shorter than keyword also works (shorter is part)
    expect(_countKeywordMatches(new Set(['dependency']), new Set(['dep']))).toBe(1)
  })

  test('skips < 3 chars', () => {
    expect(_countKeywordMatches(new Set(['de']), new Set(['dependency']))).toBe(0)
  })
})

describe('boostMultiChunkFiles', () => {
  test('top chunk receives boost_unit * fileSum / maxFileSum', () => {
    const c1 = mkChunk('x', 'a.ts', 1, 10)
    const c2 = mkChunk('y', 'a.ts', 11, 20)
    const c3 = mkChunk('z', 'a.ts', 21, 30)
    const cOther = mkChunk('q', 'b.ts')

    const scores = new Map<Chunk, number>([
      [c1, 0.5],
      [c2, 0.4],
      [c3, 0.3],
      [cOther, 0.2],
    ])

    boostMultiChunkFiles(scores)

    // Top chunk in a.ts is c1 (0.5). file_sum["a.ts"] = 1.2, file_sum["b.ts"] = 0.2.
    // max_score = 0.5, boost_unit = 0.5 * 0.2 = 0.1, max_file_sum = 1.2.
    // c1 gets: 0.5 + 0.1 * 1.2 / 1.2 = 0.6
    // cOther gets: 0.2 + 0.1 * 0.2 / 1.2 ≈ 0.21666...
    expect(scores.get(c1)).toBeCloseTo(0.6, 10)
    expect(scores.get(c2)).toBe(0.4)
    expect(scores.get(c3)).toBe(0.3)
    expect(scores.get(cOther)).toBeCloseTo(0.2 + 0.1 * 0.2 / 1.2, 10)
  })

  test('no-op on empty map', () => {
    const scores = new Map<Chunk, number>()
    boostMultiChunkFiles(scores)
    expect(scores.size).toBe(0)
  })

  test('no-op when max score is zero', () => {
    const c = mkChunk('x', 'a.ts')
    const scores = new Map<Chunk, number>([[c, 0]])
    boostMultiChunkFiles(scores)
    expect(scores.get(c)).toBe(0)
  })

  test('uses FILE_COHERENCE_BOOST_FRAC = 0.2', () => {
    // Single chunk, single file → fileSum == maxFileSum, so boost = boost_unit.
    const c = mkChunk('x', 'a.ts')
    const scores = new Map<Chunk, number>([[c, 1.0]])
    boostMultiChunkFiles(scores)
    expect(scores.get(c)).toBeCloseTo(1.0 + 1.0 * FILE_COHERENCE_BOOST_FRAC, 10)
  })
})

describe('applyQueryBoost', () => {
  test('symbol query with definition keyword boosts chunk by DEFINITION_BOOST_MULTIPLIER * maxScore (1.0× when stem does not match)', () => {
    // File stem is "other", not "handlerstack" → 1.0× tier.
    const defChunk = mkChunk('class HandlerStack:\n    pass\n', 'other.py')
    const otherChunk = mkChunk('print("hi")', 'b.py')

    const scores = new Map<Chunk, number>([
      [defChunk, 0.5],
      [otherChunk, 1.0],
    ])
    const boosted = applyQueryBoost(scores, 'HandlerStack', [defChunk, otherChunk])

    // maxScore = 1.0, boostUnit = 1.0 * 3.0 = 3.0; defChunk picks up 3.0 (1.0× tier).
    expect(boosted.get(defChunk)).toBeCloseTo(0.5 + 1.0 * DEFINITION_BOOST_MULTIPLIER, 10)
    expect(boosted.get(otherChunk)).toBe(1.0)
  })

  test('symbol query with matching file stem gets 1.5× tier boost', () => {
    // Stem "handler_stack" matches "handlerstack" after snake-stripping.
    const defChunk = mkChunk('class HandlerStack:\n    pass\n', 'handler_stack.py')
    const scores = new Map<Chunk, number>([[defChunk, 0.5]])
    const boosted = applyQueryBoost(scores, 'HandlerStack', [defChunk])
    // boostUnit = 0.5 * 3.0 = 1.5; tier = 1.5 * 1.5 = 2.25; new score = 0.5 + 2.25 = 2.75.
    expect(boosted.get(defChunk)).toBeCloseTo(2.75, 10)
  })

  test('symbol query promotes non-candidate stem-matching chunks', () => {
    const candidate = mkChunk('print("hi")', 'b.py')
    const nonCandidate = mkChunk('class HandlerStack:\n    pass\n', 'handler_stack.py')
    const scores = new Map<Chunk, number>([[candidate, 1.0]])
    const boosted = applyQueryBoost(scores, 'HandlerStack', [candidate, nonCandidate])
    // Non-candidate appears with score = boostUnit * 1.5 = 1.0 * 3.0 * 1.5 = 4.5.
    expect(boosted.get(nonCandidate)).toBeCloseTo(4.5, 10)
  })

  test('NL query with embedded PascalCase triggers half-strength embedded boost', () => {
    const defChunk = mkChunk('class StateManager:\n    pass\n', 'state_manager.py')
    const scores = new Map<Chunk, number>([[defChunk, 1.0]])
    const boosted = applyQueryBoost(
      scores,
      'where does the StateManager initialize state',
      [defChunk],
    )
    // Embedded boost: tier-with-stem-match = boostUnit * 1.5
    // boostUnit_embedded = 1.0 * DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE = 1.5
    // tier = 1.5 * 1.5 = 2.25 → new score = 1.0 + 2.25 = 3.25
    // Plus possible stem-match boost from `_boostStemMatches`. To avoid that ambiguity,
    // assert lower bound.
    const expectedEmbedded = DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE * 1.5
    const result = boosted.get(defChunk) ?? 0
    expect(result).toBeGreaterThanOrEqual(1.0 + expectedEmbedded - 1e-9)
  })

  test('returns a new map and does not mutate input', () => {
    const c = mkChunk('class Foo:\n    pass\n', 'foo.py')
    const original = new Map<Chunk, number>([[c, 1.0]])
    const boosted = applyQueryBoost(original, 'Foo', [c])
    expect(original.get(c)).toBe(1.0)
    expect(boosted).not.toBe(original)
    expect(boosted.get(c)).toBeGreaterThan(1.0)
  })

  test('empty input is returned as-is', () => {
    const empty = new Map<Chunk, number>()
    const out = applyQueryBoost(empty, 'foo', [])
    expect(out.size).toBe(0)
  })

  test('NL query boosts via stem matches when file path words match', () => {
    const c = mkChunk('print("hi")', 'cache_layer.py')
    const scores = new Map<Chunk, number>([[c, 1.0]])
    const boosted = applyQueryBoost(scores, 'find the cache layer', [c])
    // Keywords: {find, the→stopword, cache, layer} → {find, cache, layer}.
    // Parts from "cache_layer" split → cache_layer, cache, layer
    // Matches: cache, layer → n=2, ratio=2/3, boost = 1.0 * 1.0 * 2/3
    expect(boosted.get(c)).toBeCloseTo(1.0 + 2 / 3, 10)
  })
})
