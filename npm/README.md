# npm distribution wrapper

> Status: **live** (since v0.1.4). The published `@pleaseai/csp` package is the
> Rust-binary wrapper generated from this directory by
> `scripts/generate-platform-packages.mjs` and published via npm Trusted
> Publishing in `.github/workflows/release-please.yml`. The generator ships the
> repo-root `README.md` + `LICENSE` inside the wrapper so the npm page renders
> docs. This internal note documents the layout; it is not published.

## Goal

Preserve the existing entrypoint — `bunx @pleaseai/csp` / `npx @pleaseai/csp` —
while shipping the Rust-compiled `csp` binary instead of a bundled JS CLI. This
follows the [Biome](https://github.com/biomejs/biome) distribution model:

- `@pleaseai/csp` (this `csp/` dir) is a thin **wrapper** package. Its `bin`
  is a tiny Node launcher that resolves and `exec`s the correct platform binary.
- Per-platform packages (`@pleaseai/csp-<target>`) each carry one prebuilt
  binary and declare `os` + `cpu` so npm/bun install only the matching one.
- The wrapper lists every platform package under `optionalDependencies`, so a
  failed-to-match platform is skipped rather than failing the whole install.

```
@pleaseai/csp                     (wrapper — bin/csp.js launcher)
├── @pleaseai/csp-darwin-arm64    (optionalDependency, os=darwin cpu=arm64)
├── @pleaseai/csp-darwin-x64
├── @pleaseai/csp-linux-x64
├── @pleaseai/csp-linux-arm64
├── @pleaseai/csp-linux-x64-musl
└── @pleaseai/csp-win32-x64       (csp.exe)
```

## Layout

- `csp/` — the wrapper package (`package.json` + `bin/csp.js`).
- `scripts/generate-platform-packages.mjs` — at release time, generates the
  per-platform package directories from the built `csp-<target>` assets and the
  release version, ready to `npm publish --provenance` each one.

## Release flow

1. `release-rust.yml` builds `csp-<target>` binaries + checksums.
2. `node npm/scripts/generate-platform-packages.mjs <version> <assets-dir>`
   materializes `npm/dist/<pkg>/` for each platform (and the wrapper, with the
   repo-root `README.md` + `LICENSE` copied in).
3. Publish each platform package, then the wrapper, with
   `npm publish ./<pkg> --access public` (CI: `id-token: write`). Auth is npm
   Trusted Publishing (OIDC) — no token, and provenance is generated
   automatically, so no `--provenance` flag is needed.
