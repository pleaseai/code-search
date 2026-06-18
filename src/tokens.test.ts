// Port of src/semble/tokens.py tests

import { describe, expect, it } from 'bun:test'
import { splitIdentifier, tokenize } from './tokens.ts'

describe('splitIdentifier', () => {
  it('splits PascalCase identifiers', () => {
    expect(splitIdentifier('HandlerStack')).toEqual([
      'handlerstack',
      'handler',
      'stack',
    ])
  })

  it('preserves runs of capitals as a single sub-token', () => {
    expect(splitIdentifier('getHTTPResponse')).toEqual([
      'gethttpresponse',
      'get',
      'http',
      'response',
    ])
  })

  it('handles leading run of capitals', () => {
    expect(splitIdentifier('XMLParser')).toEqual([
      'xmlparser',
      'xml',
      'parser',
    ])
  })

  it('splits snake_case identifiers', () => {
    expect(splitIdentifier('my_func')).toEqual(['my_func', 'my', 'func'])
  })

  it('returns only the lowered token when there is no boundary', () => {
    expect(splitIdentifier('simple')).toEqual(['simple'])
  })

  it('lowercases an already lower-case token', () => {
    expect(splitIdentifier('Already')).toEqual(['already'])
  })

  it('keeps consecutive underscores from collapsing into duplicate parts', () => {
    // Python `split('_')` produces empty strings between consecutive
    // underscores; the upstream filter drops them.
    expect(splitIdentifier('foo__bar')).toEqual(['foo__bar', 'foo', 'bar'])
  })

  it('treats a leading underscore as snake_case with one effective part', () => {
    // `_foo`.split('_') === ['', 'foo'] -> filtered to ['foo'] -> len < 2
    expect(splitIdentifier('_foo')).toEqual(['_foo'])
  })

  it('splits digit runs as their own camel sub-token', () => {
    expect(splitIdentifier('abc123Def')).toEqual([
      'abc123def',
      'abc',
      '123',
      'def',
    ])
  })

  it('splits kebab-case and dotted path stems on `-`/`.` separators', () => {
    // `splitIdentifier` is also called on raw file-path stems (e.g. in
    // ranking/boosting.ts). The camel regex treats `-`/`.` as separators, so
    // the lowercase fast-path must not short-circuit these.
    expect(splitIdentifier('user-service')).toEqual([
      'user-service',
      'user',
      'service',
    ])
    expect(splitIdentifier('foo.bar')).toEqual(['foo.bar', 'foo', 'bar'])
  })
})

describe('tokenize', () => {
  it('splits plain space-separated words', () => {
    expect(tokenize('foo bar baz')).toEqual(['foo', 'bar', 'baz'])
  })

  it('expands compound identifiers and drops non-identifier digits', () => {
    // Numbers that do not start an identifier (e.g. "123") are not matched by
    // TOKEN_RE, which mirrors the upstream Python behaviour.
    expect(tokenize('camelCase_snake_case 123')).toEqual([
      'camelcase_snake_case',
      'camelcase',
      'snake',
      'case',
    ])
  })

  it('returns an empty array for input with no identifiers', () => {
    expect(tokenize('   !!! 123 ???')).toEqual([])
  })

  it('preserves multiple identifiers and expands each', () => {
    expect(tokenize('HandlerStack my_func')).toEqual([
      'handlerstack',
      'handler',
      'stack',
      'my_func',
      'my',
      'func',
    ])
  })
})
