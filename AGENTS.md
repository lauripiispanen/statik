# Agent Guidelines for statik

## Build & Validate

- Always run `cargo test` before committing — all tests must pass
- Always run `cargo clippy` before committing — no new warnings
- Run `cargo check` as a quick smoke test during development

## Team Sessions

- Agents working on many sequential tasks will hit context limits. Spawn fresh agents for later tasks rather than overloading one agent with 4+ implementation tasks.
- The police/reviewer agent should review each task as it completes, not batch-review at the end. This gives the coder tighter feedback loops.
- When spawning teams, include `cargo clippy` in the definition of done alongside `cargo test`.
- When parallel agents work on adjacent tasks (e.g., Phase B and Phase D that both touch `file_graph.rs`), explicitly assign file ownership in task descriptions to prevent duplicate code and merge conflicts.
- Agents must ensure `cargo check` passes after each atomic change, not just at task completion — partial changes break the build for other parallel agents.
- The `.claude/` directory is gitignored. Use `git add -f` to commit skill or settings files.
- Every bug fix must include a substantive regression test that would fail if the bug were reintroduced. Police/reviewers should reject fixes without such tests.
- Team lead: complete all wrap-up work (coverage checks, final test runs, doc verification) before sending shutdown requests. Don't bulk-shutdown agents until you're sure no more coordination is needed.

## Code Conventions

- Follow existing patterns in the codebase — don't introduce new dependencies or patterns without discussing with the architect
- Use proper error handling on user-facing paths (no `unwrap()`)
- Keep `commands.rs` focused on orchestration; formatting goes in `output.rs`, graph building in `graph_builder.rs`
- Tests should be substantive — verify behavior, not just absence of crashes
- When combining data from different subsystems (e.g., file graph paths vs git history paths), normalize path formats. The file graph uses absolute paths; git history uses relative paths. Always convert before comparison and write tests that assert non-trivial values to catch silent mismatches.
