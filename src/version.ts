// Port of src/semble/version.py.
//
// The Python upstream stores a triple (`(0, 2, 0)`) and joins it for the
// string form. Here we expose a single literal because:
//   * `package.json#version` is the source of truth for npm publishing.
//   * Bun/tsdown don't read Python-style triples; reconstructing one would
//     just be dead code.
// A future integration PR will keep this in sync with `package.json#version`
// (e.g. via a generated file or a build-time replacement).
export const version = '0.0.0'
