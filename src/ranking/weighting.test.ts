import { describe, expect, test } from 'bun:test'
import { ALPHA_NL, ALPHA_SYMBOL, resolveAlpha } from './weighting.ts'

describe('resolveAlpha', () => {
  test('returns ALPHA_NL for plain lowercase queries', () => {
    expect(resolveAlpha('session', null)).toBe(0.5)
    expect(resolveAlpha('session', null)).toBe(ALPHA_NL)
  })

  test('returns ALPHA_SYMBOL for PascalCase symbol queries', () => {
    expect(resolveAlpha('HandlerStack', null)).toBe(0.3)
    expect(resolveAlpha('HandlerStack', null)).toBe(ALPHA_SYMBOL)
  })

  test('returns the provided alpha when set', () => {
    expect(resolveAlpha('foo', 0.7)).toBe(0.7)
    expect(resolveAlpha('HandlerStack', 0.9)).toBe(0.9)
  })

  test('treats undefined like null', () => {
    expect(resolveAlpha('session', undefined)).toBe(0.5)
    expect(resolveAlpha('HandlerStack', undefined)).toBe(0.3)
  })

  test('alpha=0 is honored (not treated as missing)', () => {
    expect(resolveAlpha('HandlerStack', 0)).toBe(0)
  })
})
