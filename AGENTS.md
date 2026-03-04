# Agent Guidelines for statik

## Build & Validate

- Run `cargo fmt --check`, `cargo clippy`, `cargo test` before committing — all three, no subset
- Run `cargo check` as a quick smoke test during development
- Team agents: run `cargo fmt` and `cargo clippy` before marking tasks complete

## Team Sessions

- Spawn fresh agents after 4+ tasks to avoid context limits
- Police reviews each task incrementally, not batched at the end
- Assign file ownership when parallel agents touch adjacent code
- `cargo check` must pass after each atomic change, not just at task completion
- Every bug fix must include a regression test that fails if the bug returns
- Complete all wrap-up work before sending shutdown requests
- `.claude/` is gitignored — use `git add -f` for skill/settings files

## Code Conventions

- Follow existing patterns; no new dependencies without architect approval
- No `unwrap()` on user-facing paths
- `commands.rs` = orchestration, `output.rs` = formatting, `graph_builder.rs` = graph building
- Normalize path formats when combining subsystems (file graph = absolute, git history = relative)
- Never duplicate data rows with new IDs — reuse existing IDs and remap references
- Dogfood with real data, not just synthetic tests

## Planning

- After implementing new data sources or infrastructure, spawn a gap analysis agent to check every command for missing integration
- Create TODO entries for each gap with priority and complexity

## Skills

- `/iteration` → follow `.claude/skills/iteration/SKILL.md` directly — spin up the full team, no solo work

## Team Lead

- Never pick up implementation work — delegate everything, coordination only
- Before committing, run the full `cargo fmt --check && cargo clippy && cargo test` sequence
- Use foreground agents or `mode: "bypassPermissions"` when agents need shell access
