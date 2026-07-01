# Contributing

Thanks for your interest in contributing! This guide covers how to get from a clone to a merged pull request.

By participating, you agree to abide by our [Code of Conduct](./CODE_OF_CONDUCT.md). All documentation, code, comments, and commit messages in this repository are written in **English**.

## Getting started

`csp` is Rust-first (the workspace under `crates/`) with a thin JS toolchain for the npm wrapper and repo tooling. [mise](https://mise.jdx.dev/) pins the tool versions (node, bun, and the `hk` git-hook manager); the Rust toolchain is pinned separately by `rust-toolchain.toml`.

```bash
git clone https://github.com/pleaseai/code-search.git
cd code-search
mise install        # install pinned tool versions (node, bun, hk) and set up git hooks
bun install         # install JS dependencies
```

If you do not use mise, install the versions listed in `mise.toml` and `rust-toolchain.toml` manually, then run `bun install`. Rust builds use the standard `cargo` toolchain.

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

## Commit messages

We follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): subject`, where `type` is one of `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, etc. Breaking changes include a `BREAKING CHANGE:` footer. Versioning and the changelog are generated automatically from these messages, so accurate types matter.

## Pull requests

- Reference the issue your PR addresses (e.g. `Closes #123`).
- Use a Conventional-Commit-style PR title — it becomes the squash-merge commit.
- Make sure CI is green before requesting review.

## Reporting bugs and requesting features

Open an issue using the bug report or feature request template. For security
vulnerabilities, **do not** open a public issue — follow [SECURITY.md](./SECURITY.md).
