# Agent Guidelines for statik

## Build & Validate

- Always run `cargo fmt`, `cargo clippy`, and `cargo test` before committing — all must pass clean
- Run `cargo check` as a quick smoke test during development
- Team agents: run `cargo fmt` and `cargo clippy` before marking tasks complete, not just `cargo test`

## Team Sessions

- Agents working on many sequential tasks will hit context limits. Spawn fresh agents for later tasks rather than overloading one agent with 4+ implementation tasks.
- The police/reviewer agent should review each task as it completes, not batch-review at the end. This gives the coder tighter feedback loops.
- When spawning teams, include `cargo clippy` in the definition of done alongside `cargo test`.
- When parallel agents work on adjacent tasks (e.g., Phase B and Phase D that both touch `file_graph.rs`), explicitly assign file ownership in task descriptions to prevent duplicate code and merge conflicts.
- Agents must ensure `cargo check` passes after each atomic change, not just at task completion — partial changes break the build for other parallel agents.
- The `.claude/` directory is gitignored. Use `git add -f` to commit skill or settings files.
- Every bug fix must include a substantive regression test that would fail if the bug were reintroduced. Police/reviewers should reject fixes without such tests.
- Team lead: complete all wrap-up work (coverage checks, final test runs, doc verification) before sending shutdown requests. Don't bulk-shutdown agents until you're sure no more coordination is needed.
- When the architect implements tasks beyond planning, coordinate with the coder to avoid compilation-blocking conflicts (e.g., both touching ParseResult or struct constructors simultaneously).

## Code Conventions

- Follow existing patterns in the codebase — don't introduce new dependencies or patterns without discussing with the architect
- Use proper error handling on user-facing paths (no `unwrap()`)
- Keep `commands.rs` focused on orchestration; formatting goes in `output.rs`, graph building in `graph_builder.rs`
- Tests should be substantive — verify behavior, not just absence of crashes
- When combining data from different subsystems (e.g., file graph paths vs git history paths), normalize path formats. The file graph uses absolute paths; git history uses relative paths. Always convert before comparison and write tests that assert non-trivial values to catch silent mismatches.
- When adding a new data source that overlaps with existing data (e.g., SCIP symbols alongside tree-sitter symbols), never create duplicate rows with new IDs. Reuse existing IDs and remap references. Dogfood with real data (not just synthetic tests) before committing to catch duplication issues at scale.

## Team Lead Rules

- **Never pick up implementation work** — not even running tests. Delegate everything. Spawn fresh agents when needed. The lead's job is coordination only. Previous iterations failed because the lead did work and ran out of context.
- When spawning background agents that need bash access, they may be blocked by permission settings. Use foreground agents for tasks requiring shell access, or set `mode: "bypassPermissions"`.
