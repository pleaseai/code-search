// Public library barrel — port of `src/semble/__init__.py`.
//
// External consumers `import { CspIndex, ContentType, ... } from '@pleaseai/csp'`,
// so this file's surface is load-bearing and matches the README.
//
// `ContentType` is intentionally re-exported as a *value* (not via
// `export type`) because Unit 1's port models it as a `const`-object enum:
// the identifier carries both a runtime value and a same-named type alias.
// With `verbatimModuleSyntax`, exporting it via `export {}` carries both
// forms; listing it under `export type {}` would erase the runtime side.

export { CspIndex } from './indexing/index.ts'

export type {
  Chunk,
  IndexStats,
  SearchResult,
} from './types.ts'

export { ContentType } from './types.ts'

export { version } from './version.ts'
