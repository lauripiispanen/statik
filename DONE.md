# statik — Completed Tasks

Complexity: S=hours, M=days, L=weeks, XL=months

## Phase 1: Core Hardening
- [x] [1.1 Wildcard re-export tracing (M)](TODO/1.1-wildcard-reexport-tracing.md)
- [x] [1.2 Dynamic import support (M)](TODO/1.2-dynamic-import-support.md)
- [x] [1.3 Text output formatting (M)](TODO/1.3-text-output-formatting.md)
- [x] [1.6 Integration tests (M)](TODO/1.6-integration-tests.md) — 2 minor items deferred
- [x] [1.7 Structural diff (L)](TODO/1.7-structural-diff.md)

## Phase 2: Architectural Linting
- [x] [2.1 Configuration file parser (M)](TODO/2.1-config-file-parser.md)
- [x] [2.2 Glob-based file matching (S)](TODO/2.2-glob-file-matching.md) — 1 minor item deferred
- [x] [2.3 Boundary rules (M)](TODO/2.3-boundary-rules.md) — 2 minor items deferred
- [x] [2.4 Layer hierarchy rules (M)](TODO/2.4-layer-hierarchy-rules.md)
- [x] [2.5 Module containment rules (M)](TODO/2.5-module-containment-rules.md)
- [x] [2.6 Import restriction rules (S)](TODO/2.6-import-restriction-rules.md)
- [x] [2.7 Fan-in / fan-out limits (S)](TODO/2.7-fan-in-fan-out-limits.md)
- [x] [2.8 Tag-based dependency rules (M)](TODO/2.8-tag-dependency-rules.md)
- [x] [2.9 `statik lint` command (M)](TODO/2.9-statik-lint-command.md)
- [x] [2.10 Agent integration docs (S)](TODO/2.10-agent-integration-docs.md) — 2 minor items deferred

## Phase 2b: Advanced Lint Rules
- [x] [2b.1 Freeze / baseline mechanism (M)](TODO/2b.1-freeze-baseline.md)
- [x] [2b.2 Cycle size / scope policy (S)](TODO/2b.2-cycle-scope-policy.md) — 1 minor item deferred
- [x] [2b.3 Stability metric rule (S)](TODO/2b.3-stability-metric.md) — 1 minor item deferred
- [x] [2b.4 Naming convention rules (S)](TODO/2b.4-naming-convention-rules.md)
- [x] [2b.5 Restricted consumer rules (S)](TODO/2b.5-restricted-consumer-rules.md)
- [x] [2b.6 Max exports / API surface limit (S)](TODO/2b.6-max-exports-limit.md)
- [x] [2b.7 Dependency weight / coupling detection (M)](TODO/2b.7-coupling-weight.md)
- [x] [2b.8 Directory cohesion rule (M)](TODO/2b.8-directory-cohesion.md)

## Phase 3: Multi-Language Foundation (Java)
- [x] [3.1 Language enum expansion (S)](TODO/3.1-language-enum-expansion.md)
- [x] [3.2 Java file discovery (S)](TODO/3.2-java-file-discovery.md)
- [x] [3.3 Java tree-sitter parser (L)](TODO/3.3-java-tree-sitter-parser.md)
- [x] [3.4 Java import resolver (XL)](TODO/3.4-java-import-resolver.md) — 2 minor items deferred
- [x] [3.5 Mixed-project support (M)](TODO/3.5-mixed-project-support.md) — 1 minor item deferred
- [x] [3.6 Java known limitations](TODO/3.6-java-known-limitations.md) — follow-up items deferred

## Phase 3b: Rust Support (COMPLETE)
- [x] [3b.1 Rust tree-sitter parser (L)](TODO/3b.1-rust-parser.md)
- [x] [3b.2 Rust import resolver (L)](TODO/3b.2-rust-import-resolver.md)
- [x] [3b.3 Rust entry point detection (S)](TODO/3b.3-rust-entry-points.md)
- [x] [3b.4 Integration tests (M)](TODO/3b.4-rust-integration-tests.md)
- [x] [3b.5 Dogfooding findings](TODO/3b.5-rust-dogfooding.md)
- [x] [3b.6 Known limitations](TODO/3b.6-rust-known-limitations.md) — items deferred

## Phase 4: Deep Analysis (COMPLETE)
- [x] [4.1 Reference storage improvements (L)](TODO/4.1-reference-storage.md)
- [x] [4.2 Activate `symbols` command (S)](TODO/4.2-symbols-command.md)
- [x] [4.3 Activate `references` command (M)](TODO/4.3-references-command.md)
- [x] [4.4 Activate `callers` command (S)](TODO/4.4-callers-command.md)
- [x] [4.5 Symbol-level dead code detection (L)](TODO/4.5-symbol-dead-code.md)
- [x] [4.6 Type-only dependency separation (M)](TODO/4.6-type-only-deps.md)
- [x] [4.7 Java inheritance extraction (M)](TODO/4.7-java-inheritance.md)

## Phase 7: Agent-Friendly CLI (COMPLETE)
- [x] [7.1 Output path filtering `--path-filter` (S)](TODO/7.1-path-filter.md)
- [x] [7.2 Count mode `--count` (S)](TODO/7.2-count-mode.md)
- [x] [7.3 Result limiting `--limit` (S)](TODO/7.3-result-limiting.md)
- [x] [7.4 Sort control `--sort` (S)](TODO/7.4-sort-control.md)
- [x] [7.5 Built-in jq filtering `--jq` (M)](TODO/7.5-jq-filtering.md)
- [x] [7.6 Richer JSON schema (S)](TODO/7.6-richer-json-schema.md) — 1 minor item deferred
- [x] [7.7 Cross-module edge filter `--between` (S)](TODO/7.7-cross-module-filter.md)
- [x] [7.8 CSV output format (S)](TODO/7.8-csv-output.md)
- [x] [7.9 Directory-level summary aggregation (M)](TODO/7.9-directory-summary.md)

## Phase 8: Dogfooding Fixes
- [x] [8.1 Source sets: scope classification (L)](TODO/8.1-source-sets.md)
- [x] [8.2 Structural edge propagation for `pub mod` (M)](TODO/8.2-structural-edge-propagation.md)
- [x] [8.3 Relative path output (S)](TODO/8.3-relative-path-output.md)
- [x] [8.4 Inline suppression comments (S)](TODO/8.4-inline-suppression.md)
- [x] [8.6 Output duplication on stderr (S)](TODO/8.6-output-duplication.md)
- [x] [8.7 Remaining known bugs (S)](TODO/8.7-remaining-bugs.md)

## Phase 9: External Project Dogfooding (COMPLETE)
- [x] [9.1 Java source root detection for multi-module (M)](TODO/9.1-java-source-root-detection.md)
- [x] [9.2 `--lang` filter for dead-code (S)](TODO/9.2-lang-filter-dead-code.md)
- [x] [9.3 `--count` exit code fix (S)](TODO/9.3-count-exit-code.md)
- [x] [9.4 Cycles text format fix (S)](TODO/9.4-cycles-text-format.md)
- [x] [9.5 Exclude node_modules by default (S)](TODO/9.5-exclude-node-modules.md) — 3 minor items deferred
- [x] [9.6 Dead code over-reporting fix (S)](TODO/9.6-dead-code-over-reports.md)
- [x] [9.7 Dead file confidence fix (S)](TODO/9.7-dead-file-confidence.md)
- [x] [9.8 Java same-package references (M)](TODO/9.8-java-same-package.md)
- [x] [9.9 Unresolved imports split (S)](TODO/9.9-unresolved-imports-split.md)
- [x] [9.10 Forced re-index (S)](TODO/9.10-forced-reindex.md)
- [x] [9.11 Source set dependency visibility (L)](TODO/9.11-source-set-visibility.md)
- [x] [9.12 Lint crash with no rules (S)](TODO/9.12-lint-no-rules-crash.md)

## Phase 10: Human / Committer Analysis (COMPLETE)
- [x] [10.1 Git history extraction (M)](TODO/10.1-git-history.md)
- [x] [10.2 Ownership model and `statik owners` (M)](TODO/10.2-ownership-model.md) — 1 minor item deferred
- [x] [10.3 `statik who` — reviewer suggestion (M)](TODO/10.3-statik-who.md)
- [x] [10.4 `statik bus-factor` (S)](TODO/10.4-bus-factor.md)
- [x] [10.4b Bus-factor fan_in path fix (S)](TODO/10.4b-bus-factor-fan-in-fix.md)
- [x] [10.4c Per-person bus factor `--by-author` (S)](TODO/10.4c-per-person-bus-factor.md)
- [x] [10.5 `statik churn` (M)](TODO/10.5-churn-analysis.md) — 1 minor item deferred
- [x] [10.6 Team boundary analysis (M)](TODO/10.6-team-boundary.md)
- [x] [10.7 Adaptive ownership half-life (S)](TODO/10.7-adaptive-ownership.md) — 1 minor item deferred
- [x] [10.8 Wildcard import source set boundary (S)](TODO/10.8-wildcard-source-sets.md)
- [x] [10.9 Suppress unknown language warnings (S)](TODO/10.9-suppress-unknown-lang.md) — 1 minor item deferred

## Phase 11: SCIP Ingestion (partial)
- [x] [11.1 SCIP index reader (M)](TODO/11.1-scip-index-reader.md) — benchmark deferred
- [x] [11.2 `statik enrich` command (M)](TODO/11.2-statik-enrich.md) — benchmark deferred
- [x] [11.3 Staleness tracking (S)](TODO/11.3-staleness-tracking.md) — `--precise` flag deferred
- [x] [11.6 Confidence upgrade (S)](TODO/11.6-confidence-upgrade.md)
- [x] [11.7 SCIP symbol deduplication (S)](TODO/11.7-scip-symbol-dedup.md) — two-pass matching, no duplicate symbols
- [x] [11.8 SCIP cross-file refs in FileGraph (M)](TODO/11.8-scip-file-graph-integration.md) — SCIP call refs feed into all file-level analyses
