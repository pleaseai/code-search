// Port of src/semble/ranking/penalties.py
// Inlined Chunk type until src/types.ts lands (Unit 1).
interface Chunk {
  content: string
  filePath: string
  startLine: number
  endLine: number
  language?: string
}

// Patterns that identify test files across common languages.
// Grouped by language for readability; combined into a single regex.
export const TEST_FILE_RE = new RegExp(
  '(?:^|/)'
  + '(?:'
  // Python
  + 'test_[^/]*\\.py' // test_foo.py
  + '|[^/]*_test\\.py' // foo_test.py
  // Go
  + '|[^/]*_test\\.go' // foo_test.go
  // Java
  + '|[^/]*Tests?\\.java' // FooTest.java / FooTests.java
  // PHP
  + '|[^/]*Test\\.php' // FooTest.php
  // Ruby
  + '|[^/]*_spec\\.rb' // foo_spec.rb
  + '|[^/]*_test\\.rb' // foo_test.rb
  // JavaScript / TypeScript
  + '|[^/]*\\.test\\.[jt]sx?' // foo.test.js/ts/jsx/tsx
  + '|[^/]*\\.spec\\.[jt]sx?' // foo.spec.js/ts/jsx/tsx
  // Kotlin
  + '|[^/]*Tests?\\.kt' // FooTest.kt / FooTests.kt
  + '|[^/]*Spec\\.kt' // FooSpec.kt (Kotest)
  // Swift
  + '|[^/]*Tests?\\.swift' // FooTests.swift (XCTest)
  + '|[^/]*Spec\\.swift' // FooSpec.swift (Quick)
  // C#
  + '|[^/]*Tests?\\.cs' // FooTest.cs / FooTests.cs
  // C / C++
  + '|test_[^/]*\\.cpp' // test_foo.cpp (Google Test)
  + '|[^/]*_test\\.cpp' // foo_test.cpp (Google Test)
  + '|test_[^/]*\\.c' // test_foo.c
  + '|[^/]*_test\\.c' // foo_test.c
  // Scala
  + '|[^/]*Spec\\.scala' // FooSpec.scala (ScalaTest)
  + '|[^/]*Suite\\.scala' // FooSuite.scala (MUnit)
  + '|[^/]*Test\\.scala' // FooTest.scala
  // Dart
  + '|[^/]*_test\\.dart' // foo_test.dart
  + '|test_[^/]*\\.dart' // test_foo.dart
  // Lua
  + '|[^/]*_spec\\.lua' // foo_spec.lua (busted)
  + '|[^/]*_test\\.lua' // foo_test.lua
  + '|test_[^/]*\\.lua' // test_foo.lua (luaunit)
  // Shared helper patterns (all languages)
  + '|test_helper[^/]*\\.\\w+' // test_helpers.go, test_helper.rb, etc.
  + ')$',
)

// Test/spec directories.
export const TEST_DIR_RE = /(?:^|\/)(?:tests?|__tests__|spec|testing)(?:\/|$)/

// Compat/legacy path components.
export const COMPAT_DIR_RE = /(?:^|\/)(?:compat|_compat|legacy)(?:\/|$)/

// Examples/docs path components.
export const EXAMPLES_DIR_RE = /(?:^|\/)(?:_?examples?|docs?_src)(?:\/|$)/

// TypeScript declaration files (.d.ts stubs).
export const TYPE_DEFS_RE = /\.d\.ts$/

export const STRONG_PENALTY = 0.3 // test files, compat shims, example/doc code
export const MODERATE_PENALTY = 0.5 // re-export / metadata files
export const MILD_PENALTY = 0.7 // .d.ts declaration stubs (still carry useful type info)

// Filenames that are re-export barrels or package-level metadata.
export const REEXPORT_FILENAMES = new Set(['__init__.py', 'package-info.java'])

// Maximum chunks from the same file before a saturation penalty is applied.
export const FILE_SATURATION_THRESHOLD = 1

// Multiplicative penalty per extra chunk from the same file beyond the threshold.
export const FILE_SATURATION_DECAY = 0.5

/**
 * Select top-k results with optional file-path penalties and file-saturation decay.
 *
 * When `penalisePaths` is true, path penalties are applied before sorting.
 * Saturation decay is applied greedily during the greedy pass; because decay
 * only reduces scores and candidates are pre-sorted descending, early exit is
 * safe once the remaining scores cannot beat the current k-th best.
 */
export function rerankTopK(
  scores: Map<Chunk, number>,
  topK: number,
  options: { penalisePaths?: boolean } = {},
): Array<[Chunk, number]> {
  const penalisePaths = options.penalisePaths ?? true

  if (scores.size === 0 || topK <= 0) {
    return []
  }

  // Apply file-path penalties.
  const penaltyCache = new Map<string, number>()
  const penalised = new Map<Chunk, number>()
  for (const [chunk, score] of scores) {
    if (penalisePaths) {
      let cached = penaltyCache.get(chunk.filePath)
      if (cached === undefined) {
        cached = _filePathPenalty(chunk.filePath)
        penaltyCache.set(chunk.filePath, cached)
      }
      penalised.set(chunk, score * cached)
    }
    else {
      penalised.set(chunk, score)
    }
  }

  // Sort by penalised score (highest first) — single sort.
  const ranked = [...penalised.keys()].sort((a, b) => {
    const sa = penalised.get(a) as number
    const sb = penalised.get(b) as number
    return sb - sa
  })

  const fileSelected = new Map<string, number>()
  const selected: Array<[number, Chunk]> = []
  let minSelected = Number.POSITIVE_INFINITY

  for (const chunk of ranked) {
    const penScore = penalised.get(chunk) as number

    if (selected.length >= topK && penScore <= minSelected) {
      break
    }

    const alreadySelected = fileSelected.get(chunk.filePath) ?? 0
    let effScore = penScore
    if (alreadySelected >= FILE_SATURATION_THRESHOLD) {
      const excess = alreadySelected - FILE_SATURATION_THRESHOLD + 1
      effScore *= FILE_SATURATION_DECAY ** excess
    }

    selected.push([effScore, chunk])
    fileSelected.set(chunk.filePath, alreadySelected + 1)

    if (selected.length >= topK) {
      let m = Number.POSITIVE_INFINITY
      for (const [s] of selected) {
        if (s < m) {
          m = s
        }
      }
      minSelected = m
    }
  }

  selected.sort((a, b) => b[0] - a[0])
  return selected.slice(0, topK).map(([score, chunk]) => [chunk, score])
}

/**
 * Return a combined multiplicative penalty for all applicable path patterns.
 */
export function _filePathPenalty(filePath: string): number {
  const normalised = filePath.replace(/\\/g, '/')
  let penalty = 1.0
  if (TEST_FILE_RE.test(normalised) || TEST_DIR_RE.test(normalised)) {
    penalty *= STRONG_PENALTY
  }
  // Match Python's Path(file_path).name (POSIX semantics): only forward-slash
  // is a separator. Backslashes in the raw path are part of the filename.
  const basename = filePath.slice(filePath.lastIndexOf('/') + 1)
  if (REEXPORT_FILENAMES.has(basename)) {
    penalty *= MODERATE_PENALTY
  }
  if (COMPAT_DIR_RE.test(normalised)) {
    penalty *= STRONG_PENALTY
  }
  if (EXAMPLES_DIR_RE.test(normalised)) {
    penalty *= STRONG_PENALTY
  }
  if (TYPE_DEFS_RE.test(normalised)) {
    penalty *= MILD_PENALTY
  }
  return penalty
}
