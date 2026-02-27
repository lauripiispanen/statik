# Agent Guidelines for statik

## Build & Validate

- Always run `cargo test` before committing — all tests must pass
- Always run `cargo clippy` before committing — no new warnings
- Run `cargo check` as a quick smoke test during development

## Team Sessions

- Agents working on many sequential tasks will hit context limits. Spawn fresh agents for later tasks rather than overloading one agent with 4+ implementation tasks.
- The police/reviewer agent should review each task as it completes, not batch-review at the end. This gives the coder tighter feedback loops.
- When spawning teams, include `cargo clippy` in the definition of done alongside `cargo test`.

## Code Conventions

- Follow existing patterns in the codebase — don't introduce new dependencies or patterns without discussing with the architect
- Use proper error handling on user-facing paths (no `unwrap()`)
- Keep `commands.rs` focused on orchestration; formatting goes in `output.rs`, graph building in `graph_builder.rs`
- Tests should be substantive — verify behavior, not just absence of crashes
