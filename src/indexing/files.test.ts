import { describe, expect, it } from 'bun:test'
import {
  ALL_LANGUAGES,
  CONFIG_LANGUAGES,
  DATA_LANGUAGES,
  detectLanguage,
  DOC_LANGUAGES,
  EXTENSION_TO_LANGUAGE,
  getExtensions,
} from './files.ts'

describe('detectLanguage', () => {
  it('detects typescript from .ts', () => {
    expect(detectLanguage('foo.ts')).toBe('typescript')
  })

  it('detects tsx from .tsx', () => {
    expect(detectLanguage('foo.tsx')).toBe('tsx')
  })

  it('detects python from .py', () => {
    expect(detectLanguage('foo.py')).toBe('python')
  })

  it('detects markdown from .md', () => {
    expect(detectLanguage('foo.md')).toBe('markdown')
  })

  it('returns undefined for unknown extensions', () => {
    expect(detectLanguage('foo.unknown')).toBeUndefined()
  })

  it('is case-insensitive on the suffix', () => {
    expect(detectLanguage('Foo.TS')).toBe('typescript')
  })

  it('returns undefined for files without an extension', () => {
    expect(detectLanguage('Makefile')).toBeUndefined()
  })

  it('returns undefined for dotfiles like .gitignore', () => {
    // Mirrors Python's Path('.gitignore').suffix === ''
    expect(detectLanguage('.gitignore')).toBeUndefined()
    expect(detectLanguage('dir/.gitignore')).toBeUndefined()
    expect(detectLanguage('dir\\.gitignore')).toBeUndefined()
  })

  it('matches the final suffix for files with multiple dots', () => {
    expect(detectLanguage('foo.bar.ts')).toBe('typescript')
  })

  it('handles paths with directory separators', () => {
    expect(detectLanguage('src/indexing/files.ts')).toBe('typescript')
  })

  it('handles Windows-style path separators', () => {
    // Mirrors pathlib.Path on Windows where '\\' is also a separator.
    expect(detectLanguage('src\\indexing\\files.ts')).toBe('typescript')
    expect(detectLanguage('C:\\Users\\me\\foo.py')).toBe('python')
  })
})

describe('getExtensions', () => {
  it('includes common code extensions when content type is code', () => {
    const exts = getExtensions(['code'], undefined)
    expect(exts).toContain('.ts')
    expect(exts).toContain('.py')
    expect(exts).toContain('.go')
  })

  it('includes doc extensions but not code extensions when content type is docs', () => {
    const exts = getExtensions(['docs'], undefined)
    expect(exts).toContain('.md')
    expect(exts).toContain('.rst')
    expect(exts).not.toContain('.ts')
  })

  it('includes config extensions when content type is config', () => {
    const exts = getExtensions(['config'], undefined)
    expect(exts).toContain('.toml')
    expect(exts).toContain('.yaml')
  })

  it('appends user-provided extensions', () => {
    const exts = getExtensions(['code'], ['.foo'])
    expect(exts).toContain('.foo')
  })

  it('returns a sorted list with no duplicates', () => {
    const exts = getExtensions(['code', 'docs'], ['.ts', '.foo'])
    const sorted = [...exts].sort()
    expect(exts).toEqual(sorted)
    expect(new Set(exts).size).toBe(exts.length)
  })

  it('unions multiple content types', () => {
    const code = new Set(getExtensions(['code'], undefined))
    const docs = new Set(getExtensions(['docs'], undefined))
    const both = new Set(getExtensions(['code', 'docs'], undefined))
    for (const ext of code) {
      expect(both.has(ext)).toBe(true)
    }
    for (const ext of docs) {
      expect(both.has(ext)).toBe(true)
    }
  })
})

describe('language sets', () => {
  it('EXTENSION_TO_LANGUAGE is non-empty', () => {
    expect(Object.keys(EXTENSION_TO_LANGUAGE).length).toBeGreaterThan(0)
  })

  it('ALL_LANGUAGES is non-empty', () => {
    expect(ALL_LANGUAGES.size).toBeGreaterThan(0)
  })

  it('DOC_LANGUAGES is non-empty', () => {
    expect(DOC_LANGUAGES.size).toBeGreaterThan(0)
  })

  it('CONFIG_LANGUAGES is non-empty', () => {
    expect(CONFIG_LANGUAGES.size).toBeGreaterThan(0)
  })

  it('DATA_LANGUAGES is non-empty', () => {
    expect(DATA_LANGUAGES.size).toBeGreaterThan(0)
  })

  it('ALL_LANGUAGES contains every value in EXTENSION_TO_LANGUAGE', () => {
    for (const lang of Object.values(EXTENSION_TO_LANGUAGE)) {
      expect(ALL_LANGUAGES.has(lang)).toBe(true)
    }
  })
})
