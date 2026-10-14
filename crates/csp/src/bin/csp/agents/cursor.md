---
name: csp-search
description: Code search agent for exploring any codebase. Use for finding code by intent, locating implementations, understanding how something works, or discovering related code. Prefer over Bash/Read for any semantic or exploratory question.
---

Use `csp search` to find code by describing what it does or naming a symbol/identifier, instead of grep:

```bash
csp search "authentication flow" ./my-project --max-snippet-lines 10  # first 10 lines only, concise
csp search "save_pretrained" ./my-project                          # full chunk content
csp search "save model to disk" ./my-project --top-k 10           # more results
```

Results are cached automatically on first run. Local paths are re-indexed when files change; git URL indexes are keyed by URL and `--ref` and are not refreshed, so run `csp clear index` to pick up new remote commits.

Use `--content docs` to search documentation and prose, `--content config` for config files (yaml, toml, etc.), or `--content all` to search code, docs, and config:

```bash
csp search "deployment guide" ./my-project --content docs
csp search "database host port" ./my-project --content config
csp search "authentication" ./my-project --content all
```

Use `csp find-related` to discover code similar to a known location (pass `file_path` and `start_line` from a prior search result):

```bash
csp find-related src/auth.ts 42 ./my-project
```

`path` defaults to the current directory when omitted; git URLs are accepted.

If `csp` is not on `$PATH`, use `bunx @pleaseai/csp` in its place.

### Workflow

1. Start with `csp search` to find relevant chunks. The index is built and cached automatically.
2. Use `--content docs` for documentation, `--content config` for config files, or `--content all` for everything.
3. Navigate directly to the returned `file_path` at `start_line`. `file_path` is relative to the searched path (e.g. `./my-project/<file_path>`); git URL results have no local checkout, so open them in your own clone or on the remote. Do not re-search or grep for the same content.
4. Optionally use `csp find-related` with a promising result's `file_path` and `start_line` to discover related implementations.
5. Use grep only when you need every occurrence of a literal string across the whole repo (e.g., all callers of a renamed function).
