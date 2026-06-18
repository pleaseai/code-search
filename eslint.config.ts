import pleaseai from '@pleaseai/eslint-config'

export default pleaseai({
  typescript: {
    tsconfigPath: 'tsconfig.json',
  },
  ignores: [
    'dist',
    'node_modules',
    '.csp',
    // Rust build artifacts.
    'target',
    // Rust manifests are governed by `cargo fmt` / the Rust toolchain, not
    // eslint's JS-project TOML style rules (which conflict with Cargo conventions).
    'Cargo.toml',
    'Cargo.lock',
    'crates/**',
    'rust-toolchain.toml',
    'rustfmt.toml',
    // npm distribution wrapper: a hand-written CommonJS Node launcher + a
    // release-time package generator (not part of the linted TS app source).
    'npm',
  ],
}, {
  // Relax a handful of type-aware rules for test files, where common testing
  // patterns legitimately trip them:
  //   - await-thenable: bun's `expect(...).rejects.toThrow()` is typed as
  //     returning a non-Promise, so the (correct) `await` is flagged.
  //   - no-require-imports: tests use typed `require(...) as typeof import(...)`
  //     to pull modules in mid-test (e.g. after filesystem/env setup).
  //   - unbound-method: tests capture `Class.staticMethod` to restore it after
  //     monkey-patching — wrapping in an arrow would break the save/restore.
  //   - no-template-curly-in-string: fixtures embed literal source code (e.g.
  //     `return \`hi ${name}\``) that is data, not a template expression.
  files: ['**/*.test.ts'],
  rules: {
    'ts/await-thenable': 'off',
    'ts/no-require-imports': 'off',
    'ts/unbound-method': 'off',
    'no-template-curly-in-string': 'off',
  },
})
