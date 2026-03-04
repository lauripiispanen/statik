---
name: iteration
description: Spin up a full dev team to plan and deliver the next feature increment
disable-model-invocation: true
argument-hint: "[focus area or 'next' for auto-detect]"
---

# Development Iteration

Spin up a coordinated team to plan, implement, review, test, and ship the next increment. Uses TODO.md and ROADMAP.md to identify work, then delivers it with quality gates.

## Team Roles

Create a team with `TeamCreate`, then spawn these 6 agents:

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
- **Regression testing**: When fixing a reported bug or issue, you MUST write a substantive test case that specifically reproduces the bug scenario and would fail if the bug regressed. "Substantive" means the test verifies the exact behavior that was broken, not just that the code compiles or doesn't crash.
- **Context limit risk**: If implementing 4+ tasks, may hit context limits. The lead should spawn a fresh `coder2` if this happens.

### 3. Police (`police`)
- **Type**: `general-purpose`
- **Job**: Review code quality as tasks complete. Give tight, specific feedback to the coder. Ensure tests are substantive, not smoke tests.
- **Review criteria**: Correctness, error handling, edge cases, test coverage, Rust idioms, no `unwrap()` on user-facing paths, exhaustive pattern matching, no dead code.
- **Zero tolerance for regressions**: You MUST NOT approve any task that introduces regressions in existing functionality. Run `cargo test` yourself to verify. If any previously-passing test now fails, REJECT the task and message the coder with the specific failure.
- **Zero tolerance for old issues**: If you spot pre-existing bugs, code smells, or leftover issues from previous work in the areas being touched, flag them. Create a new task or message the architect — these must be fixed before the increment ships, not carried forward.
- **Regression test enforcement**: Every bug fix MUST have a substantive test that would catch the bug if it regressed. If a fix lacks such a test, REJECT the task and tell the coder exactly what test scenario is missing.
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
- **Zero tolerance for regressions**: If ANY test that passed at baseline now fails, this is a BLOCKING issue. Message the coder AND the lead immediately. Do not approve the increment until all baseline tests pass again.
- **Zero tolerance for old issues**: If you discover pre-existing test gaps, flaky tests, or tests that were skipped/ignored from previous work, flag them to the architect. These should be fixed in this increment.
- **Regression test enforcement**: For every reported bug or issue being fixed, verify that a substantive test case exists that specifically covers the bug scenario. If a bug fix ships without a regression test, report it as a blocking issue to the lead and the coder. A test is "substantive" if it would FAIL when the bug is reintroduced — not just "it compiles" or "it doesn't panic".
- **Ongoing**: Re-run tests after each task completes, add missing integration tests, report failures immediately to coder. Do a final comprehensive test report before commit.

### 6. Documenter (`documenter`)
- **Type**: `general-purpose`
- **Job**: Ensure documentation matches the current state of the project after the increment. Bridge gaps between ROADMAP.md/TODO.md goals, what was actually implemented, and user-facing docs.
- **First task**: Read ROADMAP.md, TODO.md, and any existing documentation (README.md, --help output, doc comments). Wait for implementation tasks to be created so you know what's changing.
- **Ongoing responsibilities**:
  - Update TODO.md to check off completed items
  - Update ROADMAP.md if the increment changes priorities or completes milestones
  - Update README.md or user-facing docs if new CLI flags, commands, or behaviors were added
  - Ensure --help text and CLI argument descriptions are accurate for any new/changed flags
  - Add or update doc comments on new public APIs
  - Flag any gaps between what the roadmap promises and what was actually delivered
- **Timing**: The documenter works in the wrap-up phase after implementation tasks are mostly complete, but can start reading and planning earlier.

## Workflow

### Phase 1: Setup
1. `TeamCreate` with descriptive name
2. Create initial tasks: "Plan the next increment" and "Identify refactoring opportunities"
3. Spawn all 6 agents in parallel with `run_in_background: true`
4. If `$ARGUMENTS` specifies a focus area, include it in the architect's prompt

### Phase 2: Planning (architect + boyscout work in parallel)
- Architect reads TODO.md/ROADMAP.md, explores codebase, creates implementation tasks
- Boyscout explores codebase for refactoring opportunities, syncs with architect
- Tester establishes test baseline
- Documenter reads existing docs, prepares for updates
- Coder and police wait for tasks

### Phase 3: Implementation (all agents active)
- Coder picks up tasks in ID order, implements them
- Police reviews completed tasks, gives feedback
- Tester verifies tests pass after each change, adds integration tests
- Boyscout picks up approved refactoring tasks or helps with implementation
- Architect answers questions, confirms completed work

### Phase 4: Wrap-up
- All implementation tasks complete
- Police does final review pass across all changes — flags any regressions or old issues
- Tester runs full suite one final time, produces comprehensive report — confirms zero regressions
- Documenter updates TODO.md, ROADMAP.md, README.md, and any other docs
- Architect dogfoods statik on itself, reports findings

### Phase 5: Fix-up (if needed)
- If police or tester report issues after the coder has finished:
  - Lead checks if the existing coder agent is still active
  - If active: message the coder with the issues to fix
  - If shut down or at context limit: spawn a fresh `coder2` (or `coder3`, etc.)
  - The new coder picks up fix tasks, implements them, and the review/test cycle repeats
  - This phase loops until ALL issues are resolved — the increment does NOT ship with known regressions or missing regression tests

### Phase 6: Ship
- Lead verifies all tests pass (`cargo test`)
- Lead commits all changes with descriptive commit message
- Lead shuts down all agents via `shutdown_request`
- Lead runs `TeamDelete` to clean up
- Lead runs `/reflect` to capture session learnings

## Definition of Done
- [ ] All implementation tasks completed and reviewed by police
- [ ] All tests pass (no regressions, no skipped old tests)
- [ ] Zero regressions — every test that passed at baseline still passes
- [ ] Every bug fix has a substantive regression test
- [ ] New features have integration tests
- [ ] No pre-existing issues left unaddressed in touched code areas
- [ ] Documentation updated (TODO.md, ROADMAP.md, README.md, CLI help text)
- [ ] `cargo clippy` passes clean
- [ ] Changes committed to git
- [ ] Dogfooding report from architect (if applicable)

## Lead Responsibilities (YOU)
- **Do NOT enter plan mode** — go straight to team setup. Planning is the architect's job, not yours.
- **Do NOT pick up implementation tasks** — stay out of the work to preserve your context for coordination
- Monitor task list progress, nudge idle agents toward available work
- Create follow-up tasks (e.g., "Update TODO.md") as needs emerge
- Handle commits and git operations
- Spawn fresh agents if any hit context limits
- **Spawn fresh coders when issues are found after coder finishes** — the increment must not ship until definition of done is fully met
- Shut down team and clean up when done

## Tips
- Spawn all agents at once — they'll self-coordinate via the task list
- The architect and boyscout can work in parallel during planning
- If the coder hits context limits after 4+ tasks, spawn `coder2` for remaining work
- Task #16-style bonus refactorings can be deferred if time/context is tight
- The police should review incrementally, not batch at the end
- Message agents directly when they need direction — don't wait for them to ask
- The documenter can start reading docs early but should write changes after implementation stabilizes
- If police or tester find issues post-implementation, always spawn a new coder rather than leaving issues unresolved

## Focus Area

$ARGUMENTS
