// Smoke tests for the public library barrel.
//
// These don't exercise behavior — Unit 12 (CspIndex) and Unit 1 (types) own
// their own deep tests. The point here is to lock down the *shape* of the
// public surface so we'd catch:
//   * an accidental rename of `CspIndex` / `ContentType` / `version`,
//   * a regression of `ContentType` to a type-only export (which would
//     break `import { ContentType } from '@pleaseai/csp'` at runtime).
//
// The wildcard `import * as csp` is deliberate: it also verifies the module
// is *syntactically* a valid ESM barrel (no circular value-time imports).
import { describe, expect, it } from 'bun:test'

import * as csp from './index.ts'

describe('public barrel', () => {
  it('imports without error and exposes the documented names', () => {
    // Use a `Set` so the assertion message is order-independent — easier to
    // diagnose than a positional array diff when a name is missing.
    const exported = new Set(Object.keys(csp))
    for (const name of ['CspIndex', 'ContentType', 'version']) {
      expect(exported.has(name)).toBe(true)
    }
  })

  it('exposes `version` as a string', () => {
    expect(typeof csp.version).toBe('string')
    // Guard against an empty string sneaking in (e.g. failed build-time
    // substitution); a real version is always non-empty.
    expect(csp.version.length).toBeGreaterThan(0)
  })

  it('exposes `CspIndex` as a constructable value', () => {
    // `typeof X === 'function'` covers both `class` and plain functions,
    // which keeps the test resilient if Unit 12 chooses a factory-style
    // implementation instead of a class.
    expect(typeof csp.CspIndex).toBe('function')
  })

  it('exposes `ContentType` as a runtime enum object with `code | docs | config`', () => {
    // The string values are part of the on-disk / CLI contract (`--content code`,
    // persisted indices). They must NOT be tweaked without coordinating with
    // the semble compatibility story documented in CLAUDE.md.
    expect(csp.ContentType.CODE).toBe('code')
    expect(csp.ContentType.DOCS).toBe('docs')
    expect(csp.ContentType.CONFIG).toBe('config')
  })
})
