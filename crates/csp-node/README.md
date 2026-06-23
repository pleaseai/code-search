# @pleaseai/csp-sdk

In-process native (napi-rs) SDK for [csp](https://github.com/pleaseai/code-search) — fast, accurate hybrid code search for agents.

This package binds the Rust `csp` core (`crates/csp`) **directly** through a Node-API addon, so JS callers run search in-process: no subprocess, no JSON round-trip, native objects back.

> **This is the library channel.** The `csp` **CLI + MCP server** ship separately as [`@pleaseai/csp`](../../npm) — a thin launcher that execs the standalone Rust binary (and the Homebrew formula, which needs no Node runtime). Use this SDK when you want to embed search in a Node/Bun program; use `@pleaseai/csp` to run the CLI or an MCP server. Both are built from the same `crates/csp` core (decision A in the repo `CLAUDE.md`).

## Usage

```ts
import { CspIndex, ContentType } from '@pleaseai/csp-sdk'

const index = CspIndex.fromPath('./my-project', { content: [ContentType.Code] })

for (const { chunk, score } of index.search('parse config file', { topK: 5 })) {
  console.log(score.toFixed(3), chunk.location)
}

// Persist / reload
index.save('./.csp-index')
const reloaded = CspIndex.loadFromDisk('./.csp-index')

// Related chunks
const [top] = reloaded.search('auth middleware')
reloaded.findRelated(top.chunk, { topK: 10 })
```

The full type surface is in [`index.d.ts`](./index.d.ts).

## Develop

This crate (`csp-node`) is a member of the workspace Cargo build. The Rust side compiles like any crate:

```bash
cargo build -p csp-node                    # compile the bindings
cargo clippy -p csp-node --all-targets -- -D warnings
```

The `.node` addon + the JS loader (`index.js`) are produced by `@napi-rs/cli`, which reads `package.json` and the colocated `Cargo.toml`:

```bash
cd crates/csp-node
bun install                # @napi-rs/cli
bun run build              # napi build --platform --release  → csp-sdk.<triple>.node + index.js
```

`index.d.ts` is committed as the authoritative type surface; `napi build` regenerates it with the same shape. The compiled `*.node`, generated `index.js`, and per-platform `npm/` packages are gitignored.

## Publish

Cross-compilation targets mirror the binary release (`release-rust.yml`): darwin arm64/x64, linux x64/arm64-gnu + x64-musl, win32 x64. Publishing follows the standard napi-rs flow (build per-target on CI → `napi artifacts` → `napi prepublish -t npm`), with the version kept in lockstep by release-please (`crates/csp-node/package.json#version` is an `extra-files` entry).

## Follow-ups

- **Async factories.** `fromPath` / `fromGit` are currently synchronous and block the Node event loop during indexing (and, for `fromGit`, a network clone). Move them onto napi `AsyncTask` so they yield.
- **CI job.** Add a napi cross-compile + `napi prepublish` matrix to the release workflow (analogous to `release-rust.yml` for the binary).
- **README cross-link.** The top-level `README.md` / `README.ko.md` library section documents the legacy `@pleaseai/csp` import; once this SDK ships, point the library examples at `@pleaseai/csp-sdk`.
