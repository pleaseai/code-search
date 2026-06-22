// Port of src/semble/version.py.
//
// The Python upstream stores a triple (`(0, 2, 0)`) and joins it for the
// string form. Here we expose a single literal because:
//   * `package.json#version` is the source of truth for npm publishing.
//   * Bun/tsdown don't read Python-style triples; reconstructing one would
//     just be dead code.
// Kept in sync with `package.json#version` by release-please via the
// `x-release-please-version` annotation below (see release-please-config.json
// `extra-files`).
export const version = '0.1.4' // x-release-please-version
