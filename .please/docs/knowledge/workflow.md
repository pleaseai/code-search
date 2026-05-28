# Project Workflow

> Defines the development workflow conventions for the project.
> Referenced by `/please:implement`.

## Guiding Principles

1. **The Plan is the Source of Truth**: All work is tracked in the track's `plan.md`
2. **The Tech Stack is Deliberate**: Changes to the tech stack must be documented in `tech-stack.md` before implementation
3. **Test-Driven Development**: Write tests before implementing functionality
4. **High Code Coverage**: Aim for >80% code coverage for new code
5. **Non-Interactive & CI-Aware**: Prefer non-interactive commands. Use `CI=true` for watch-mode tools

## Task Workflow

All tasks follow a strict lifecycle within `/please:implement`:

### Standard Task Lifecycle

1. **Select Task**: Choose the next available task from `plan.md`
2. **Mark In Progress**: Update task status from `[ ]` to `[~]`
3. **Write Failing Tests (Red Phase)**:
   - Create test file for the feature or bug fix
   - Write unit tests defining expected behavior
   - Run tests and confirm they fail as expected
4. **Implement to Pass Tests (Green Phase)**:
   - Write minimum code to make failing tests pass
   - Run test suite and confirm all tests pass
5. **Refactor (Optional)**:
   - Improve clarity, remove duplication, enhance performance
   - Rerun tests to ensure they still pass
6. **Verify Coverage**: Run coverage reports. Target: >80% for new code
7. **Document Deviations**: If implementation differs from tech stack, update `tech-stack.md` first
8. **Commit**: Stage and commit with conventional commit message
9. **Update Progress**: Mark the task as completed in `## Progress` with a timestamp

### Phase Completion Protocol

Executed when all tasks in a phase are complete:

1. **Verify Test Coverage**: Identify all files changed in the phase, ensure test coverage
2. **Run Full Test Suite**: Execute all tests, debug failures (max 2 fix attempts)
3. **Manual Verification Plan**: Generate step-by-step verification instructions for the user
4. **User Confirmation**: Wait for explicit user approval before proceeding
5. **Create Checkpoint**: Commit with message `chore(checkpoint): complete phase {name}`
6. **Update Plan**: Mark phase as complete in `plan.md`

## Quality Gates

Before marking any task complete:

- [ ] All tests pass (`bun test`)
- [ ] Code coverage meets requirements (>80%)
- [ ] Code follows project style guidelines (`bun run lint`)
- [ ] No type errors (`bun run typecheck`)
- [ ] No security vulnerabilities introduced
- [ ] Documentation updated if needed (especially README + README.ko)

## Development Commands

### Setup

```bash
bun install              # install dependencies (uses bun.lock)
```

### Daily Development

```bash
bun run dev              # tsdown --watch (rebuild on save)
bun run --bun src/cli.ts # run the CLI directly from sources
```

### Testing

```bash
bun test                          # full test suite
bun test path/to/file.test.ts     # single file
bun test --watch                  # watch mode (use CI=true to disable in CI)
bun test --coverage               # coverage report
```

### Before Committing

```bash
bun run typecheck     # tsc --noEmit
bun run lint          # eslint . --cache
bun run lint:fix      # eslint . --fix --cache
bun run build         # tsdown — verify a clean build
bun test              # full suite
```

A pre-commit ritual is `bun run typecheck && bun run lint && bun test`.

## Testing Requirements

### Unit Testing

- Every module must have corresponding tests under `src/**/*.test.ts` (co-located with the source) or `tests/`
- Mock external dependencies (filesystem, network, `git` subprocess) — use `bun:test`'s `mock` and `spyOn`
- Test both success and failure cases
- Fixture repos for indexing tests live under `tests/fixtures/`

### Integration Testing

- Index → search round-trip tests on small fixture repos
- MCP server tests: spawn the server, send tool calls over stdio, assert responses
- CLI tests: invoke `csp` via `Bun.spawn`, assert stdout/exit code

## Commit Guidelines

Follow the conventional commits convention. See `Skill("standards:commit-convention")` for details.

### Types

- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Formatting changes
- `refactor`: Code change without behavior change
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

### Project-specific guidance

- When porting an upstream `semble` module, the commit subject must name it: `feat(chunking): port src/semble/chunking/core.py to TS`.
- Public API renames (anything in the README's library / CLI / MCP sections) must update both `README.md` and `README.ko.md` in the same commit.

## Stacked PR Workflow

This project uses **graphite-based stacked PRs** (`workflow.stacked_pr.enabled: true` in `config.yml`).

- Track branches follow `tracks/{track_id}` (single-phase) or `tracks/{track_id}/phase-N-{slug}` (multi-phase, auto-split from `plan.md` `### Phase N:` headings).
- `/please:new-track` creates the branch + Draft PR via `gt create` + `gt submit`.
- `/please:implement` extends the stack per phase when `split_phases: true`.
- `/please:finalize` runs `gt submit --stack --publish` + `gt sync` to propagate landed merges down the stack.
- Run `gt init` once locally (with `--trunk main`) before first track work if Graphite is not yet initialized in this repo.

## Definition of Done

A task is complete when:

1. All code implemented to specification
2. Unit tests written and passing
3. Code coverage meets project requirements (>80%)
4. Code passes `bun run typecheck`, `bun run lint`, `bun run build`
5. Progress updated in `plan.md`
6. Changes committed with proper conventional message
7. README + README.ko updated if the public API surface changed
