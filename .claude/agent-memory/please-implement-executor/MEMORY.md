# Memory Index

- [csp typecheck baseline is red by design](csp-typecheck-baseline-red.md) — TS5097 .ts-import errors are project-wide; gate on "no new type errors", not "green tsc"
- [csp test files are ahead of impl](csp-test-files-ahead-of-impl.md) — some indexing *.test.ts depend on not-yet-existing APIs; a scoped source fix won't make them green
