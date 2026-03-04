# statik TODO

Remaining tasks by priority. Complexity: S=hours, M=days, L=weeks, XL=months.
Task details in [TODO/](TODO/). Completed tasks in [DONE.md](DONE.md).

## Phase 11: SCIP Ingestion (in progress)

- [ ] [11.9 SCIP-powered symbol-level analysis (L)](TODO/11.9-scip-symbol-level-analysis.md) — symbol-level impact, improved dead-code seeding, cross-file callers; prereq: 11.8
- [ ] [11.4 C++ support via scip-clang (M)](TODO/11.4-cpp-support.md)
- [ ] [11.5 Cross-language dependency edges (L)](TODO/11.5-cross-language-edges.md)

## Phase 8: Dogfooding Fixes

- [ ] [8.5 Java multi-module source roots (M)](TODO/8.5-java-source-roots.md) — config-driven `source_roots` for Maven/Gradle

## Phase 1: Core Hardening

- [ ] [1.4 Lazy loading / streaming queries (L)](TODO/1.4-lazy-loading.md) — needed for 10K+ file projects
- [ ] [1.5 Graph caching (M)](TODO/1.5-graph-caching.md) — prereq: 1.4

## Phase 5: Refactoring Intelligence (not started)

- [ ] [5.1 Dual-index comparison engine (L)](TODO/5.1-dual-index-comparison.md) — prereq: 1.7
- [ ] [5.2 Breaking change detection (M)](TODO/5.2-breaking-change-detection.md) — prereq: 5.1
- [ ] [5.3 Git integration for diff (L)](TODO/5.3-git-integration-diff.md) — prereq: 5.1, 5.2
- [ ] [5.4 CI integration mode (M)](TODO/5.4-ci-integration-mode.md) — prereq: 5.3
- [ ] [5.5 Cycle introduction/resolution tracking (M)](TODO/5.5-cycle-tracking.md) — prereq: 5.1

## Phase 6: Ecosystem & Integrations (not started)

- [ ] [6.1 JSON output schema stabilization (M)](TODO/6.1-json-schema-stabilization.md)
- [ ] [6.2 Dependency graph visualization (M)](TODO/6.2-graph-visualization.md) — prereq: 6.1
- [ ] [6.3 VS Code extension (L)](TODO/6.3-vscode-extension.md) — prereq: 6.1
- [ ] [6.4 GitHub Action (M)](TODO/6.4-github-action.md) — prereq: 5.4, 6.1
- [ ] [6.5 Watch mode (L)](TODO/6.5-watch-mode.md) — prereq: 1.5
- [ ] [6.6 Language Server Protocol (XL)](TODO/6.6-lsp.md) — exploratory; prereq: 6.5

## Strategic priorities

1. **11.9** — SCIP symbol-level analysis (unique differentiator: `statik impact --symbol`)
2. **8.5** — Java multi-module source roots
3. **1.4–1.5** — Lazy loading + graph caching (10K+ file scale)
4. **5.x** — Refactoring intelligence (`statik diff HEAD~1 HEAD`)
5. **6.2** — Graph visualization (`statik graph --format dot`)
