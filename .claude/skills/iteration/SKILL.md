---
name: iteration
description: Spin up a full dev team to plan and deliver the next feature increment
disable-model-invocation: true
argument-hint: "[focus area or 'next' for auto-detect]"
---

# Development Iteration

Spin up a coordinated team to plan, implement, review, test, and ship the next increment. Uses TODO.md and ROADMAP.md to identify work, then delivers it with quality gates.

## Team Roles

Create a team with `TeamCreate`, then spawn these 5 agents:

### 1. Architect (`architect`)
- **Type**: `general-purpose`
- **Job**: Read TODO.md and ROADMAP.md. Identify the next logical increment based on what's completed (checked items) vs remaining (unchecked). Write a plan, then create concrete implementation tasks in the task list with file paths, descriptions, and acceptance criteria.
- **First task**: Claim the planning task, analyze the project state, create implementation tasks, message the team when ready.
- **Ongoing**: Review boyscout findings, confirm completed tasks, answer architecture questions from the coder. Dogfood statik on itself when implementation is done.

### 2. Coder (`coder`)
- **Type**: `general-purpose`
- **Job**: Pick up implementation tasks from the task list and implement them. Run `cargo check` and `cargo test` after each change.
- **First task**: Check task list for available work. If none yet, message architect.
- **Ongoing**: Implement tasks in ID order, address police feedback promptly, mark tasks completed.
- **Context limit risk**: If implementing 4+ tasks, may hit context limits. The lead should spawn a fresh `coder2` if this happens.

### 3. Police (`police`)
- **Type**: `general-purpose`
- **Job**: Review code quality as tasks complete. Give tight, specific feedback to the coder. Ensure tests are substantive, not smoke tests.
- **Review criteria**: Correctness, error handling, edge cases, test coverage, Rust idioms, no `unwrap()` on user-facing paths, exhaustive pattern matching, no dead code.
- **Ongoing**: Watch for completed tasks, review via `git diff`, message coder with specific feedback. Do a final review pass before commit.

### 4. Boyscout (`boyscout`)
- **Type**: `general-purpose`
- **Job**: Explore the codebase for refactoring opportunities. Sync findings with architect for approval before creating tasks. Can also pick up implementation tasks if capacity allows.
- **First task**: Systematic codebase review for duplication, complexity, naming issues, dead code, architecture smells.
- **Ongoing**: After review, pick up approved refactoring tasks or help with implementation tasks. Report "no issues found" if codebase is clean — don't manufacture problems.

### 5. Tester (`tester`)
- **Type**: `general-purpose`
- **Job**: Run the full test suite (`cargo test`) to establish baseline. Monitor for test failures as changes land. Add integration tests for new features. Verify test coverage is adequate.
- **First task**: Run `cargo test`, report baseline count.
- **Ongoing**: Re-run tests after each task completes, add missing integration tests, report failures immediately to coder. Do a final comprehensive test report before commit.

## Workflow

### Phase 1: Setup
1. `TeamCreate` with descriptive name
2. Create initial tasks: "Plan the next increment" and "Identify refactoring opportunities"
3. Spawn all 5 agents in parallel with `run_in_background: true`
4. If `$ARGUMENTS` specifies a focus area, include it in the architect's prompt

### Phase 2: Planning (architect + boyscout work in parallel)
- Architect reads TODO.md/ROADMAP.md, explores codebase, creates implementation tasks
- Boyscout explores codebase for refactoring opportunities, syncs with architect
- Tester establishes test baseline
- Coder and police wait for tasks

### Phase 3: Implementation (all agents active)
- Coder picks up tasks in ID order, implements them
- Police reviews completed tasks, gives feedback
- Tester verifies tests pass after each change, adds integration tests
- Boyscout picks up approved refactoring tasks or helps with implementation
- Architect answers questions, confirms completed work

### Phase 4: Wrap-up
- All implementation tasks complete
- Police does final review pass across all changes
- Tester runs full suite one final time, produces comprehensive report
- Architect dogfoods statik on itself, reports findings
- Lead creates "Update TODO.md" task for remaining agent to handle

### Phase 5: Ship
- Lead verifies all tests pass (`cargo test`)
- Lead commits all changes with descriptive commit message
- Lead shuts down all agents via `shutdown_request`
- Lead runs `TeamDelete` to clean up
- Lead runs `/reflect` to capture session learnings

## Definition of Done
- [ ] All implementation tasks completed and reviewed by police
- [ ] All tests pass (no regressions, no skipped old tests)
- [ ] New features have integration tests
- [ ] TODO.md updated with completed items checked off
- [ ] `cargo clippy` passes clean
- [ ] Changes committed to git
- [ ] Dogfooding report from architect (if applicable)

## Lead Responsibilities (YOU)
- **Do NOT pick up implementation tasks** — stay out of the work to preserve your context for coordination
- Monitor task list progress, nudge idle agents toward available work
- Create follow-up tasks (e.g., "Update TODO.md") as needs emerge
- Handle commits and git operations
- Spawn fresh agents if any hit context limits
- Shut down team and clean up when done

## Tips
- Spawn all agents at once — they'll self-coordinate via the task list
- The architect and boyscout can work in parallel during planning
- If the coder hits context limits after 4+ tasks, spawn `coder2` for remaining work
- Task #16-style bonus refactorings can be deferred if time/context is tight
- The police should review incrementally, not batch at the end
- Message agents directly when they need direction — don't wait for them to ask

## Focus Area

$ARGUMENTS
