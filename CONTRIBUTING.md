# Contributing

Thanks for your interest in contributing! This guide covers how to get from a clone to a merged pull request.

By participating, you agree to abide by our [Code of Conduct](./CODE_OF_CONDUCT.md). All documentation, code, comments, and commit messages in this repository are written in **English**.

## Getting started

`csp` is Rust-first (the workspace under `crates/`) with a thin JS toolchain for the npm wrapper and repo tooling. [mise](https://mise.jdx.dev/) pins the tool versions (node, bun, the `hk` git-hook manager, and the `mbx` Cargo build cache); the Rust toolchain is pinned separately by `rust-toolchain.toml`.

```bash
git clone https://github.com/pleaseai/code-search.git
cd code-search
mise install        # install pinned tool versions (node, bun, hk, mbx) and set up git hooks
bun install         # install JS dependencies
```

If you do not use mise, install the versions listed in `mise.toml` and `rust-toolchain.toml` manually, then run `bun install`. Rust builds use the standard `cargo` toolchain; `mbx` is optional (see [Faster local builds](#faster-local-builds-optional)).

## Development workflow

1. Create a branch from `main` (e.g. `feat/short-description` or `fix/issue-123`).
2. Make focused changes — keep each pull request to one logical change.
3. Run the checks below and make sure they pass.
4. Open a pull request and fill out the template.

```bash
mise run lint        # lint JS/TS (eslint)
mise run typecheck   # type-check (tsc --noEmit)
mise run test        # Rust tests (cargo test --workspace)
mise run check       # full pre-commit gate: JS lint/typecheck + Rust fmt/clippy/test
```

For Rust changes, also run `cargo fmt --all` and `cargo clippy --all-targets --all-features -- -D warnings` (both are part of `mise run check`).

### Faster local builds (optional)

CI compiles Rust through [mbx (mr boxington)](https://mr-boxington.jdx.dev), a shared Cargo build
cache, via `jdx/mr-boxington-action`. `mise install` also installs `mbx` locally (pinned in
`mise.toml`), so a build in one worktree warms every other worktree. Run Cargo through it:

```bash
mise exec -- mbx build           # or: mise exec -- mbx test --all-features --locked --workspace
mise exec -- mbx explain --last  # why a build did or did not hit the cache
```

mise's `mr_boxington` Rust option is deliberately not used: a `rust` entry in `mise.toml`
needs a version, which would override `rust-toolchain.toml` (the sole owner of the Rust channel).
So plain `cargo` is not routed through `mbx`; call `mbx` explicitly. Set
`CARGO_INCREMENTAL=0` for the best hit rate.

There is no prebuilt `mbx` for Intel macOS, so `mise.macos-x64.toml` (loaded through `auto_env` in
`.miserc.toml`) builds it from crates.io via the `cargo:mbx` backend (Rust 1.91 or newer). Without
mise, run `cargo install mbx --version 1.29.2 --locked`.

`mbx` wraps `rustc` itself, so it **defers to any `RUSTC_WRAPPER` already set** (for example
`RUSTC_WRAPPER=sccache` in a shell profile) and then caches nothing. Unset the wrapper before
using it. The `mbx[cache]` summary line it prints shows the hit count.

## Commit messages

We follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): subject`, where `type` is one of `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, etc. Breaking changes include a `BREAKING CHANGE:` footer. Versioning and the changelog are generated automatically from these messages, so accurate types matter.

## Pull requests

- Reference the issue your PR addresses (e.g. `Closes #123`).
- Use a Conventional-Commit-style PR title — it becomes the squash-merge commit.
- Make sure CI is green before requesting review.

## Reporting bugs and requesting features

Open an issue using the bug report or feature request template. For security
vulnerabilities, **do not** open a public issue — follow [SECURITY.md](./SECURITY.md).
