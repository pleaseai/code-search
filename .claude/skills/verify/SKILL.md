---
name: verify
summary: Exercise csp indexing, persisted-index search, and cache reuse through the built CLI.
---

# Verify csp

1. Build the user-facing binary with `cargo build -p code-search-please --bin csp`.
2. Use a scratch directory outside the repository for all generated indexes and HOME state.
3. Drive persisted-index behavior:
   - `target/debug/csp index crates/csp/src --out <scratch>/index`
   - `target/debug/csp search "load_or_build_index cache" --index <scratch>/index --top-k 3`
4. Drive auto-cache behavior twice with an isolated home:
   - `HOME=<scratch>/home target/debug/csp search "DEFAULT_MODEL_NAME" crates/csp/src/indexing --top-k 1`
   - Repeat the same command to exercise cache reuse.
5. Probe errors with a missing `--index` path; expect a clear `Index not found` message and exit status 1.
6. Do not use tests as runtime verification evidence; run the project quality gate separately.
