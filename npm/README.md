# npm distribution wrapper

> Status: **live** (since v0.1.4). The published `@pleaseai/csp` package is the
> Rust-binary wrapper generated from this directory by
> `scripts/generate-platform-packages.mjs` and published via npm Trusted
> Publishing in `.github/workflows/release-please.yml`. The generator ships the
> repo-root `README.md` + `LICENSE` inside the wrapper so the npm page renders
> docs. This internal note documents the layout; it is not published.

## Goal

Preserve the existing entrypoint — `bunx @pleaseai/csp` / `npx @pleaseai/csp` —
while shipping the Rust-compiled `csp` binary instead of a bundled JS CLI. The
package layout follows the [Biome](https://github.com/biomejs/biome)
optional-dependency model, and the launch path uses the
[esbuild](https://github.com/evanw/esbuild) **copy-over-shim** optimization:

- `@pleaseai/csp` (this `csp/` dir) is a thin **wrapper** package. Its `bin`
  points at a Node launcher (`bin/csp.js`) that resolves and `exec`s the correct
  platform binary — used as a **fallback** when the copy-over did not run. The
  fallback forwards argv, stdio, exit code, and termination signals
  (SIGINT/SIGTERM/SIGHUP) to the child, so killing the launcher cleanly stops a
  long-running `csp mcp` server instead of orphaning it; on an interactive TTY
  it prints a one-line hint that the native fast path is not active (silence with
  `CSP_NO_FALLBACK_WARNING=1`). Modeled on
  [ast-grep](https://github.com/ast-grep/ast-grep/tree/main/npm)'s launcher.
- A `postinstall` step (`install.js`) copies the resolved platform binary
  **over** `bin/csp.js`, so npm's `.bin/csp` symlink resolves directly to native
  code. After install there is **no Node.js process on the hot path** — this is
  ~10× faster to start than spawning the binary from a Node launcher (see
  "Startup cost" below).
- The shared platform resolver lives in `lib/resolve.js` (required by both the
  launcher and `install.js`); it is never the file overwritten by the
  copy-over, so re-running the postinstall (`npm rebuild`, `npm ci`) is
  idempotent rather than trying to `require()` a native executable.
- Per-platform packages (`@pleaseai/csp-<target>`) each carry one prebuilt
  binary and declare `os` + `cpu` so npm/bun install only the matching one.
- The wrapper lists every platform package under `optionalDependencies`, so a
  failed-to-match platform is skipped rather than failing the whole install.

### Startup cost

Measured on macOS (`csp --version`, via the installed `.bin/csp`):

| Launch path | Median startup |
| --- | --- |
| Spawn shim (Node launcher → `spawnSync` binary) | ~60 ms (Node boot + spawn) |
| Copy-over (`.bin/csp` → native binary directly) | ~5–12 ms |

The delta is the Node.js interpreter boot plus the `spawnSync` of the child —
paid on *every* invocation with the old spawn shim, and eliminated by the
copy-over.

### Package-manager note (bun)

The copy-over runs as a `postinstall` script. **npm** and **pnpm** run it by
default. **bun blocks lifecycle scripts for untrusted dependencies by default**
(`Blocked 1 postinstall`), so under `bun install` the copy-over does not run and
`bin/csp.js` stays the JS launcher — still fully functional, just without the
startup win. bun users who want the fast path add `@pleaseai/csp` to
`trustedDependencies` in their project's `package.json`:

```jsonc
{ "trustedDependencies": ["@pleaseai/csp"] }
```

`bunx @pleaseai/csp` continues to work regardless via the launcher fallback.

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

- `csp/` — the wrapper package:
  - `bin/csp.js` — the runtime launcher (overwritten by the copy-over at install).
  - `install.js` — the `postinstall` copy-over step.
  - `lib/resolve.js` — the shared platform resolver (never overwritten).
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
