# statik TODO

Detailed task breakdown organized by roadmap phase. Tasks are listed in suggested
execution order within each phase. Complexity: S (hours), M (days), L (weeks),
XL (months).

---

## Phase 1: Core Hardening

### 1.1 Wildcard re-export tracing
**Complexity**: M
**Prerequisites**: None
**Files**: `src/parser/typescript.rs`, `src/analysis/dead_code.rs`

The parser already extracts re-export records with `is_reexport: true` and
`source_path`. The dead code detector skips re-exports entirely (line 152 of
`dead_code.rs`). The gap: `export * from './module'` creates a wildcard re-export
that the dead code detector does not trace through.

Tasks:
- [x] Ensure the TypeScript parser extracts `export * from '...'` as a re-export
  record with `exported_name: "*"` (or a sentinel value)
- [x] In `detect_dead_code`, when checking if an export is used, follow re-export
  chains: if file B re-exports `*` from file A, and file C imports `foo` from
  file B, then `foo` in file A is considered used
- [x] Update the `has_wildcards` flag in `compute_confidence` (currently hardcoded
  `false` at `dead_code.rs:76`)
- [x] Add tests with barrel files that use `export *`

**Acceptance**: `statik dead-code` on a project with barrel files using `export *`
produces zero false negatives for symbols re-exported through wildcards.

---

### 1.2 Dynamic import support
**Complexity**: M
**Prerequisites**: None
**Files**: `src/parser/typescript.rs`

The parser needs to handle `import('./module')` expressions that appear as call
expressions in the tree-sitter CST.

Tasks:
- [x] Add extraction of `import()` call expressions with string literal arguments
- [x] Create `ImportRecord` entries with a flag or marker indicating dynamic import
- [x] Skip dynamic imports with non-literal arguments (template literals, variables)
  and emit an unresolved import with `DynamicPath` reason
- [x] Ensure the resolver handles dynamic import paths the same as static imports
- [x] Add parser tests for `const mod = await import('./lazy')`

**Acceptance**: `statik deps` shows dynamic imports in the dependency list.
Dynamic imports with computed paths are reported as unresolved with `DynamicPath`.

---

### 1.3 Text output formatting
**Complexity**: M
**Prerequisites**: None
**Files**: `src/cli/commands.rs`, `src/cli/output.rs`

Currently, `format_analysis_output` (line 431 of `commands.rs`) falls back to JSON
for all text output. Each command needs a human-readable formatter.

Tasks:
- [x] `deps`: Tree view showing import chain with indentation by depth
- [x] `dead-code`: Two sections (dead files, dead exports) with file paths and
  confidence indicators
- [x] `cycles`: List of cycles, each showing the file chain (A -> B -> C -> A)
- [x] `impact`: Tree view of affected files grouped by depth
- [x] `exports`: Table with columns: name, default?, re-export?, used?
- [x] `summary`: Dashboard-style output with section headers and counts
- [x] Add `--format text` as the default (already is) and ensure it produces
  readable output for all commands

**Acceptance**: Every command produces readable, non-JSON output when `--format text`
is used. Output is useful without piping through `jq`.

---

### 1.4 Lazy loading / streaming queries
**Complexity**: L
**Prerequisites**: None
**Files**: `src/db/mod.rs`, `src/cli/commands.rs`, `src/model/file_graph.rs`

The current architecture loads all data into memory via `all_files()`,
`all_symbols()`, `all_references()`. For large projects (10K+ files), this is
a bottleneck.

Tasks:
- [ ] Add `file_count()` and `symbol_count()` methods to `Database` that use
  `SELECT COUNT(*)` instead of loading all rows
- [ ] Replace `all_files()` in `build_file_graph()` with a streaming approach that
  processes files in batches
- [ ] Add `get_imports_by_files(file_ids: &[FileId])` batch query to avoid N+1
  queries in graph construction
- [ ] Add `get_exports_by_files(file_ids: &[FileId])` batch query
- [ ] Profile `build_file_graph()` on a large project and identify the actual
  bottleneck (DB queries vs HashMap construction vs resolver)
- [ ] Consider keeping the `FileGraph` approach for small projects (<5K files) and
  switching to a different strategy for large ones

**Acceptance**: `statik summary` on a 10K-file project uses less than 500MB of
peak memory.

---

### 1.5 Graph caching
**Complexity**: M
**Prerequisites**: 1.4 (lazy loading)
**Files**: `src/cli/commands.rs`, new file `src/cache.rs`

`build_file_graph()` is called on every command invocation, rebuilding the graph
from the DB each time.

Tasks:
- [ ] Serialize the `FileGraph` to a cache file (`.statik/graph.cache`) after
  building it
- [ ] On subsequent commands, check if the cache is newer than `index.db`; if so,
  load from cache instead of rebuilding
- [ ] Invalidate the cache when `statik index` runs
- [ ] Add `--no-cache` flag to force rebuild
- [ ] Benchmark the improvement on a large project

**Acceptance**: Second invocation of `statik deps` on an unchanged project is at
least 2x faster than the first.

---

### 1.6 End-to-end integration tests
**Complexity**: M
**Prerequisites**: None
**Files**: New `tests/` directory

The project has good unit tests but lacks tests that exercise the full pipeline.

Tasks:
- [x] Create fixture TypeScript projects in `tests/fixtures/` with known dependency
  structures
- [x] Add integration tests that run `statik index`, then `statik deps`, `statik
  dead-code`, `statik cycles`, `statik impact`, `statik summary` and verify output
- [x] Add a test with circular dependencies
- [x] Add a test with barrel files and re-exports
- [ ] Add a test with tsconfig path aliases
- [ ] Add a test for incremental indexing (modify a file, re-index, verify changes)

**Acceptance**: CI runs the full integration test suite. Tests cover all commands
with at least one happy path and one edge case each.

---

### 1.7 Structural diff (export surface comparison)
**Complexity**: L
**Prerequisites**: 1.6 (integration tests for validation)
**Files**: New `src/analysis/diff.rs`, `src/cli/commands.rs`

Compare the export surface of a project between two indexed snapshots.

Tasks:
- [x] Define `DiffResult` type: added files, removed files, added exports, removed
  exports, changed exports (same name, different properties)
- [x] Implement `compare_snapshots(db_a: &Database, db_b: &Database) -> DiffResult`
- [x] Add `statik diff --before <path-to-old-db>` command
- [x] Classify changes: safe (internal only), breaking (removed exports with
  importers), expanding (new exports), restructuring (moved between files)
- [x] Output breaking changes with confidence levels
- [x] Add text and JSON output formatters for diff results

**Acceptance**: `statik diff old-index.db new-index.db` correctly identifies a
removed export as a breaking change when that export has importers.

---

## Phase 2: Architectural Linting

### 2.1 Configuration file parser
**Complexity**: M
**Prerequisites**: None
**Files**: New `src/linting/config.rs`, new `.statik/rules.toml` example

Define the `.statik/rules.toml` configuration format and implement parsing. This is
the foundation for all rule types.

Tasks:
- [x] Add `toml` crate dependency to `Cargo.toml`
- [x] Define `LintConfig` struct: list of `RuleDefinition` entries
- [x] Define `RuleDefinition` enum with variants: `Boundary` (MVP, others deferred)
- [x] Each rule has: `id: String`, `severity: Severity` (error/warning/info),
  `description: String`, `rationale: Option<String>`, `fix_direction: Option<String>`
- [x] Implement TOML deserialization with clear error messages for invalid configs
- [x] Auto-discover config: look for `.statik/rules.toml`, then `statik.toml` at
  project root
- [x] Add `--config <path>` CLI flag to override config location
- [x] Create an example `.statik/rules.toml` in test fixtures
- [x] Add unit tests for config parsing: valid configs, invalid configs, missing
  fields, unknown rule types

**Acceptance**: A `.statik/rules.toml` file with boundary and layer rules parses
correctly into `LintConfig`. Invalid configs produce actionable error messages.

---

### 2.2 Glob-based file matching engine
**Complexity**: S
**Prerequisites**: 2.1
**Files**: `src/linting/matcher.rs`

Rules match files by glob patterns against their project-relative paths. This is
the shared matching engine used by all rule types.

Tasks:
- [x] Add `globset` crate dependency
- [x] Implement `FileMatcher` that takes a glob pattern and matches against
  `FileInfo.path` (project-relative)
- [x] Support multiple patterns per matcher (e.g., `["src/ui/**", "src/components/**"]`)
- [x] Handle pattern negation (`!src/ui/shared/**`)
- [ ] Cache compiled glob patterns for reuse across rules
- [x] Add unit tests with various path/pattern combinations

**Acceptance**: `FileMatcher::matches("src/ui/**", "src/ui/Button.tsx")` returns true.
Negation patterns work. Compilation is cached.

---

### 2.3 Boundary rules
**Complexity**: M
**Prerequisites**: 2.1, 2.2
**Files**: `src/linting/rules/boundary.rs`

The core rule type: "files matching pattern A must not depend on files matching
pattern B."

Tasks:
- [x] Define `BoundaryRule` config: `from: Vec<GlobPattern>`, `deny: Vec<GlobPattern>`,
  `except: Option<Vec<GlobPattern>>`
- [x] Implement evaluation: iterate all edges in `FileGraph.imports`, check if
  source matches `from` and target matches `deny`
- [x] For each violation, produce `LintViolation` with: rule ID, source file,
  target file, imported names, line number, severity
- [ ] Support inverted rules (`allow: true` = only these files may depend on the
  target pattern; all others are violations)
- [ ] Handle unresolved imports: if an edge involves an unresolved import, lower
  the violation confidence
- [x] Add tests: basic violation, exception patterns, no-violation case,
  summary counts, severity sorting, rationale/fix_direction propagation

**Acceptance**: A rule `{from = "src/ui/**", to = "src/db/**", allow = false}`
reports all direct imports from UI files to DB files, with file paths and line
numbers.

---

### 2.4 Layer hierarchy rules
**Complexity**: M
**Prerequisites**: 2.2, 2.3
**Files**: `src/linting/rules/layers.rs`

Define ordered layers; dependencies must flow in one direction.

Tasks:
- [x] Define `LayerRule` config: ordered list of `Layer` entries, each with
  `name: String` and `pattern: GlobPattern`
- [x] Default direction: top-down (first layer is highest, may depend on lower
  layers, but not vice versa)
- [x] Implement evaluation: for each import edge, determine which layers the
  source and target belong to. If source is in a lower layer than target, it's a
  violation
- [x] Handle files that don't belong to any layer: ignore (not a violation)
- [x] Handle files that belong to multiple layers: use the first match (document
  this behavior)
- [x] Report violations with: layer names, direction of violation, specific files
  and import line
- [x] Add tests: valid top-down dependency, layer violation, cross-layer skip
  (A→C skipping B is valid if A is above C)

**Acceptance**: Layers defined as `[presentation, service, data]` report a
violation when a `data` layer file imports from a `presentation` layer file.

---

### 2.5 Module containment rules
**Complexity**: M
**Prerequisites**: 2.2
**Files**: `src/linting/rules/containment.rs`

Enforce that a module's internal files only communicate with the outside world
through a designated public API file.

Tasks:
- [x] Define `ContainmentRule` config: `module: GlobPattern` (the contained
  directory), `public_api: Vec<String>` (files allowed to be imported externally)
- [x] Implement evaluation: for each import edge where target is inside the module
  and source is outside the module, check if the target is a public API file
- [ ] Also check: imports from inside the module to outside should go through
  the module's own public API imports (optional, configurable)
- [x] Report violations with: the external file, the internal file it imports,
  and which public API file it should import instead
- [x] Add tests: valid import through public API, violation (direct internal
  import), edge case (file at module boundary)

**Acceptance**: A rule `{module = "src/auth/**", public_api = ["src/auth/index.ts"]}`
reports when `src/app.ts` imports directly from `src/auth/utils.ts` instead of
`src/auth/index.ts`.

---

### 2.6 Import restriction rules
**Complexity**: S
**Prerequisites**: 2.2
**Files**: `src/linting/rules/imports.rs`

Constrain how files import from specific targets -- e.g., require type-only imports,
forbid specific symbols.

Tasks:
- [x] Define `ImportRestrictionRule` config: `target: GlobPattern`,
  `require_type_only: bool`, `forbidden_names: Option<Vec<String>>`,
  `allowed_names: Option<Vec<String>>`
- [x] Implement evaluation: for each import edge to a matching target, check
  constraints against `FileImport` metadata (`is_type_only`, `imported_names`)
- [x] Report violations with: the importing file, the specific import, which
  constraint was violated
- [x] Add tests: type-only violation, forbidden name import, allowed-list
  enforcement

**Acceptance**: A rule requiring type-only imports from `src/types/**` correctly
flags `import { User } from './types/user'` but passes `import type { User }
from './types/user'`.

---

### 2.7 Fan-in / fan-out limit rules
**Complexity**: S
**Prerequisites**: 2.2
**Files**: `src/linting/rules/fan.rs`

Alert when files exceed dependency thresholds -- architectural hotspot detection.

Tasks:
- [x] Define `FanLimitRule` config: `pattern: GlobPattern`,
  `max_fan_in: Option<u32>` (max number of files that depend on this file),
  `max_fan_out: Option<u32>` (max number of files this file depends on)
- [x] Implement evaluation: count edges in `FileGraph.imports` and
  `FileGraph.imported_by` for matching files
- [x] Report violations with: the file, current count, threshold, and whether
  it's fan-in or fan-out
- [x] Add tests: within limits, exceeds fan-in, exceeds fan-out

**Acceptance**: A rule `{pattern = "src/**", max_fan_out = 20}` reports files with
more than 20 direct dependencies.

---

### 2.8 Tag-based dependency rules
**Complexity**: M
**Prerequisites**: 2.2, 2.3
**Files**: `src/linting/rules/tags.rs`

The most flexible rule type: assign tags to file groups and define allowed/forbidden
relationships between tags.

Tasks:
- [x] Define `TagBoundaryRuleConfig`: `from_tag: String`, `deny_tags: Vec<String>`,
  `except_tags: Option<Vec<String>>`
- [x] Tags defined in top-level `[tags]` section as `HashMap<String, Vec<String>>`
- [x] Implement evaluation: for each import edge, resolve source and target tags,
  check against tag rules
- [x] A file may match multiple tags; tags are evaluated independently
- [x] Report violations with: tag names, specific files, import details
- [x] Add tests: allowed inter-tag dependency, forbidden inter-tag dependency,
  multi-tag file, except tags, unknown tag handling

**Acceptance**: Tags `{api: "src/api/**", internal: "src/internal/**"}` with rule
`{from = "api", to = "internal", allow = false}` reports violations when API
files import internal modules.

---

### 2.9 `statik lint` command
**Complexity**: M
**Prerequisites**: 2.1-2.8
**Files**: `src/cli/commands.rs`, `src/main.rs`

The CLI entry point for architectural linting.

Tasks:
- [x] Add `Commands::Lint` variant with: `--config`, `--rule <id>`, `--format`,
  `--severity-threshold` flags
- [x] Implement `run_lint()`: load config, build file graph, evaluate all rules,
  collect violations, format output
- [x] Text output: grouped by rule, then by source file. Show rule ID, severity
  icon, source → target with imported names and line number
- [x] JSON output: structured array of violations with full metadata (rule ID,
  description, rationale, severity, files, line numbers, imported names,
  confidence, suggested fix direction)
- [x] Exit code: 0 if no errors (warnings are OK), 1 if any error-severity
  violations
- [x] `--rule <id>` filter: only evaluate the specified rule
- [x] `--severity-threshold`: only fail on violations at or above this severity
- [x] Summary line: "X errors, Y warnings across Z rules"
- [x] Add integration test: create a fixture project with `.statik/rules.toml`,
  index it, run `statik lint`, verify violations

**Acceptance**: `statik lint` on a project with configured rules produces clear,
actionable output. JSON output is structured for AI agent consumption. Exit code
reflects error-severity violations.

---

### 2.10 AI agent integration documentation
**Complexity**: S
**Prerequisites**: 2.9
**Files**: README or docs

Document how AI agents should consume `statik lint` output.

Tasks:
- [x] Document the JSON output schema for lint violations
- [ ] Provide example: agent reads violations, proposes import changes
- [x] Document recommended workflow: agent runs `statik lint --format json`,
  parses violations, applies fixes, re-runs to verify
- [x] Provide example `.statik/rules.toml` for common architectural patterns
  (clean architecture, feature modules, hexagonal architecture)
- [ ] Document how to use `statik lint` in CI alongside AI agent review

**Acceptance**: An AI agent developer can read the documentation and integrate
`statik lint` into their agent's workflow within 30 minutes.

---

## Phase 2b: Advanced Lint Rules

Research-driven additions based on ArchUnit, dependency-cruiser, NDepend, and
real-world architecture enforcement patterns.

### 2b.1 Freeze / baseline mechanism ✅
**Complexity**: M
**Prerequisites**: 2.9
**Files**: `src/linting/baseline.rs`, `src/cli/commands.rs`

Record current violations and only report *new* violations in subsequent runs.
This is the single biggest adoption enabler for architecture rules in existing
codebases. ArchUnit's `FreezingArchRule` is its most praised feature.

Tasks:
- [x] Add `statik lint --freeze` command that saves current violations to
  `.statik/lint-baseline.json` (JSON format with version and timestamp)
- [x] Baseline filtering: when baseline exists, only report violations not in
  the baseline
- [x] Store format: JSON with `rule_id`, `source_file`, `target_file`, `line`
  per entry
- [x] When a baseline violation is fixed, automatically remove it from the store
  on next `--freeze`
- [x] Add `--update-baseline` flag to refresh the store after intentional changes
- [x] Document workflow: first run creates baseline, subsequent runs catch
  regressions

**Acceptance**: A team can add `statik lint` to an existing codebase with 50
violations, freeze the baseline, and CI only fails on *new* violations going
forward.

---

### 2b.2 Cycle size / scope policy ✅
**Complexity**: S
**Prerequisites**: 2.9
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Configurable cycle detection in the lint framework. Current `statik cycles` is
all-or-nothing; this allows teams to gradually tighten cycle tolerance.

Tasks:
- [x] Add `CyclePolicyRule` config variant with `max_cycle_length: usize`
- [x] Implement evaluation: run cycle detection on file graph, report cycles
  exceeding `max_cycle_length`
- [x] Optional `pattern` field to restrict check to specific paths
- [ ] Support `scope = "directory"` to check cycles at directory level (collapse
  intra-directory edges)
- [x] Add tests: cycle length filtering works

**Acceptance**: A rule `{max_cycle_length = 2}` passes 2-file
mutual dependencies but flags 3+-file cycles as errors.

---

### 2b.3 Stability metric rule (Stable Dependencies Principle) ✅
**Complexity**: S
**Prerequisites**: 2.7 (fan limit)
**Files**: `src/linting/rules.rs`

Robert C. Martin's instability metric: `I = Ce / (Ca + Ce)` where Ce = fan-out,
Ca = fan-in. Dependencies should flow toward stability (lower I).

Tasks:
- [x] Compute instability per file: `I = fan_out / (fan_in + fan_out)`
- [x] Add `StabilityLimitRule` config: `pattern`, `max_instability`
- [x] Report violations where a file exceeds the instability threshold
- [ ] Optionally expose metrics in `statik summary` output
- [x] Add tests with known-stable and known-unstable arrangements

**Acceptance**: A rule `{pattern = "src/model/**", max_instability = 0.3}` flags
files with instability exceeding 0.3.

---

### 2b.4 Naming convention boundary rules ✅
**Complexity**: S
**Prerequisites**: 2.2, 2.3
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Enforce file naming conventions within specific directories using regex patterns.

Tasks:
- [x] Add `NamingBoundaryRule` config: `pattern: Vec<GlobPattern>`,
  `must_match: String` (regex)
- [x] Match against full relative file path with regex
- [x] Implement evaluation: for each file matching pattern, check if path
  matches the regex
- [x] Add tests: naming convention enforcement

**Acceptance**: A rule `{pattern = "src/services/**", must_match = ".*Service\\.(ts|rs)$"}`
reports files in the services directory that don't follow the naming convention.

---

### 2b.5 Restricted consumer rules ("only X may use Y") ✅
**Complexity**: S
**Prerequisites**: 2.3
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Inverse of boundary rules: instead of "A must not import B", enforce "only A may
import B". Useful for cross-cutting concerns like logging, metrics, DB access.

Tasks:
- [x] Add `RestrictedConsumerRule` config: `target: Vec<GlobPattern>`,
  `allowed_consumers: Vec<GlobPattern>`
- [x] Implement evaluation: for each edge to a matching target, check if the
  source matches `allowed_consumers`. If not, it's a violation.
- [x] Add tests: restricted consumer rule enforcement

**Acceptance**: A rule restricting DB access to CLI-only correctly flags when
analysis or parser modules import DB types.

---

### 2b.6 Max exports / API surface limit ✅
**Complexity**: S
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`

Enforce that modules don't expose too many symbols. Barrel files with 50+
re-exports are a common antipattern.

Tasks:
- [x] Add `ExportLimitRule` config: `pattern: Vec<GlobPattern>`,
  `max_exports: u32`
- [x] Implement evaluation: count exports per file from the graph, flag files
  exceeding the limit
- [x] Add tests: export limit enforcement

**Acceptance**: A rule `{pattern = "src/**", max_exports = 15}` flags barrel
files with excessive re-exports.

---

### 2b.7 Dependency weight / coupling detection ✅
**Complexity**: M
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`

Flag heavy coupling between two files: not just "does A depend on B" but "how
many distinct symbols does A import from B." NDepend measures this as coupling
weight.

Tasks:
- [x] Add `CouplingWeightRule` config: `pattern: Vec<GlobPattern>`,
  `max_names_per_edge: u32`
- [x] Implement evaluation: for each edge, count imported symbols. Flag edges
  exceeding the threshold.
- [x] Report: the two files, current import count, threshold
- [x] Add tests: coupling weight enforcement

**Acceptance**: A rule `{max_names_per_edge = 10}` flags file pairs where one
imports 10+ symbols from the other, suggesting they should be merged or a shared
abstraction extracted.

---

### 2b.8 Directory cohesion rule ✅
**Complexity**: M
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`

Files in the same directory should depend more on each other than on outside
files. Low cohesion suggests files are misplaced.

Tasks:
- [x] Add `CohesionRule` config: `pattern: Vec<GlobPattern>`,
  `max_external_ratio: f64`
- [x] Compute per-directory: external deps / total deps ratio
- [x] Flag directories exceeding the external ratio threshold
- [x] Add tests with high-cohesion and low-cohesion directory arrangements

**Acceptance**: A directory where 80% of dependencies are internal passes at
max_external_ratio 0.7. A directory where 90% of dependencies are external is flagged.

---

## Phase 3: Multi-Language Foundation (Java v1 COMPLETE)

### 3.1 Language enum and SymbolKind expansion ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/model/mod.rs`

Tasks:
- [x] Add `Language::Java` variant
- [x] Add `Language::from_extension` mapping for `.java`
- [x] Add `Language::from_stored_str` mapping for `"java"`
- [x] Add `SymbolKind::Annotation` variant
- [x] Add `SymbolKind::Package` variant
- [x] Ensure `SymbolKind::from_str` handles new variants
- [x] Ensure DB serialization/deserialization handles new kinds gracefully

**Acceptance**: `Language::Java` round-trips through the DB correctly. Unknown
`SymbolKind` values in existing DBs don't cause crashes.

---

### 3.2 Java file discovery ✅
**Complexity**: S
**Prerequisites**: 3.1
**Files**: `src/discovery/mod.rs`

Tasks:
- [x] Add `.java` to `Language::from_extension`
- [x] Add default exclude patterns for Java projects: `target/`, `build/`,
  `.gradle/`, `.idea/`, `*.class`
- [x] Test discovery on a standard Maven project layout
- [x] Test that `--lang java` filter works

**Acceptance**: `statik index` discovers `.java` files in a Maven project, skipping
`target/` and `build/` directories.

---

### 3.3 Java tree-sitter parser ✅
**Complexity**: L
**Prerequisites**: 3.1, 3.2
**Files**: New `src/parser/java.rs`, `src/parser/mod.rs`, `Cargo.toml`

Implement `LanguageParser` for Java using the `tree-sitter-java` crate.

Tasks:
- [x] Add `tree-sitter-java` dependency to `Cargo.toml`
- [x] Create `JavaParser` struct implementing `LanguageParser`
- [x] Extract symbols: classes, interfaces, enums, annotations, methods, fields,
  constructors
- [x] Extract imports: `import` statements (single and wildcard)
- [x] Extract exports: Java doesn't have exports in the TS sense. Treat all
  `public` top-level declarations as exports. Package-private declarations are
  exports within the package.
- [x] Handle `package` declarations to establish qualified names
- [x] Map Java constructs to `SymbolKind`: class -> Class, interface -> Interface,
  enum -> Enum, method -> Method, field -> Variable, annotation -> Annotation
- [x] Handle inner classes (parent relationship)
- [x] Handle `extends` and `implements` as `RefKind::Inheritance` references
- [x] Register `JavaParser` in `ParserRegistry::with_defaults()`
- [x] Add unit tests with Java source code fixtures

**Acceptance**: `statik index` on a Java project produces correct symbol tables.
Classes, methods, and imports are extracted. `statik exports` on a Java file shows
public declarations.

---

### 3.4 Java import resolver ✅
**Complexity**: XL
**Prerequisites**: 3.3
**Files**: New `src/resolver/java.rs`, `src/resolver/mod.rs`

Tasks:
- [x] Implement `JavaResolver` struct implementing `Resolver`
- [x] Handle single-type imports: `import com.example.UserService` -> resolve
  to `com/example/UserService.java` relative to source root
- [x] Handle wildcard imports: `import com.example.*` -> resolve to all files in
  `com/example/` directory (creates edges to all files in the package)
- [x] Detect source roots: look for standard layouts (`src/main/java/`,
  `src/java/`, `src/`) and use package declarations to verify
- [x] Classify external imports: if the import's package doesn't map to a source
  file, classify as `External`
- [ ] Parse `pom.xml` and `build.gradle` minimally to extract dependency group/
  artifact IDs for better external classification (deferred)
- [x] Handle static imports: `import static com.example.Utils.helper`
- [x] Add unit tests with mock project layouts
- [ ] Add integration test with a real Maven project

**Acceptance**: `statik deps` on a Java file shows correct intra-project imports.
External dependencies (from Maven/Gradle) are classified as `External` with the
package name.

---

### 3.5 Mixed-project support ✅
**Complexity**: M
**Prerequisites**: 3.3, 3.4
**Files**: `src/cli/commands.rs`, `src/model/file_graph.rs`

Tasks:
- [x] Ensure `build_file_graph()` selects the correct resolver per file language
- [x] Ensure `FileGraph` handles files with different languages in the same graph
- [x] `statik summary` shows file counts broken down by language
- [x] `statik dead-code` works correctly when Java and TS files coexist
- [ ] Add integration test with a monorepo containing both TS and Java source

**Acceptance**: `statik summary` on a monorepo with TS and Java files shows correct
per-language counts. Dead code detection does not produce cross-language false
positives.

---

### 3.6 Known limitations for Java v1 (follow-up work)

Items identified during implementation that are acceptable for v1 but should be
addressed in future iterations:

- [x] **Wildcard import resolution**: `import com.example.*` now resolves to all
  `.java` files in the package directory, creating dependency edges. Note: this is
  an overapproximation -- edges are created to all files in the package regardless
  of which classes are actually used.
- [x] **Qualified name separator**: Java qualified names now use `.` consistently
  (e.g., `com.example.Foo.bar`) instead of mixing `::` from the TS parser pattern.
- [x] **Annotation-based entry point detection**: Files annotated with
  @SpringBootApplication, @Test, @ParameterizedTest, @RepeatedTest, @Component,
  @Service, @Repository, @Controller, @RestController, @Configuration, @Bean,
  @Endpoint, @WebServlet are recognized as entry points. Limitation: only a
  hardcoded list of annotations is recognized; meta-annotations and custom
  framework annotations are not followed.
- [ ] **pom.xml/build.gradle parsing**: Minimal parsing of build files for better
  external dependency classification and source root detection.
- [ ] **Package-private visibility**: Mapped to Visibility::Private. Could add
  Visibility::PackagePrivate for more accurate dead code analysis within packages.
- [x] **Same-package implicit dependencies**: Type reference scanner extracts type
  names from field types, parameter types, return types, local variable types,
  generic type arguments, casts, instanceof, and throws clauses, then resolves them
  against same-package class names. No import statement needed.
- [x] **Inner class export tracking**: Public nested types (classes, interfaces,
  enums, annotations) where all ancestor types are also public are now listed as
  exports.
- [x] **User-configurable entry points**: Entry point detection uses hardcoded file
  name patterns and annotation names. User-configurable entry points via
  `[entry_points]` in `.statik/rules.toml` allow projects to define custom patterns
  and annotations.
- [ ] **Spring DI container modeling**: Spring dependency injection wiring
  (@Autowired, @Inject, constructor injection) is not modeled as dependency edges.
- [ ] **Lombok support**: Lombok-generated code is not visible to tree-sitter.

---

## Phase 3b: Rust Support (COMPLETE)

### 3b.1 Rust tree-sitter parser ✅
**Complexity**: L
**Prerequisites**: None
**Files**: `src/parser/rust.rs`, `src/parser/mod.rs`, `Cargo.toml`

Tasks:
- [x] Add `tree-sitter-rust` 0.23 dependency (compatible with tree-sitter 0.24 via tree-sitter-language 0.1)
- [x] Create `RustParser` struct implementing `LanguageParser`
- [x] Extract symbols: functions, structs, enums, traits, type aliases, constants, statics, modules, macro_rules
- [x] Extract imports: `use` declarations (simple, grouped, wildcard, aliased, nested), `mod foo;`, `extern crate`
- [x] Extract exports: `pub` items, `pub use` re-exports
- [x] Visibility tracking: `pub` -> Public, `pub(crate)`/`pub(super)` -> Protected, private -> Private
- [x] Extract call references, inheritance references (impl Trait for Type), type references
- [x] Intra-file reference resolution
- [x] Register `RustParser` in parser registry
- [x] Add unit tests with Rust source code fixtures

### 3b.2 Rust import resolver ✅
**Complexity**: L
**Prerequisites**: 3b.1
**Files**: `src/resolver/rust.rs`, `src/resolver/mod.rs`

Tasks:
- [x] Implement `RustResolver` with `Resolver` trait
- [x] Handle `crate::` paths by walking the module tree from crate root
- [x] Handle `super::` paths (including chained `super::super::`)
- [x] Handle `self::` paths
- [x] Detect external crates from `Cargo.toml` `[dependencies]`
- [x] Classify `std`, `core`, `alloc`, `proc_macro`, `test` as external
- [x] Auto-detect crate roots: `src/lib.rs`, `src/main.rs`, `src/bin/*.rs`
- [x] Detect ambiguous modules (both `foo.rs` and `foo/mod.rs`)
- [x] Add per-language resolver dispatch in `build_file_graph()` (`src/cli/commands.rs`)

### 3b.3 Rust entry point detection ✅
**Complexity**: S
**Prerequisites**: 3b.1
**Files**: `src/analysis/dead_code.rs`

Tasks:
- [x] Recognize `main.rs`, `lib.rs` as entry points
- [x] Recognize files in `src/bin/`, `tests/`, `examples/`, `benches/` as entry points
- [x] Recognize `build.rs` as an entry point

### 3b.4 Integration tests ✅
**Complexity**: M
**Prerequisites**: 3b.1, 3b.2
**Files**: `tests/rust_integration.rs`, `tests/fixtures/rust_project/`

Tasks:
- [x] Create fixture Rust project with known dependency structure
- [x] Add integration tests for index, deps, dead-code, cycles, exports, impact
- [x] Test module resolution (crate::, super::, self::)
- [x] Test visibility and entry point detection

### 3b.5 Dogfooding findings ✅

Issues discovered by running statik on its own codebase:

- [x] **`use crate_name::` paths not resolved**: `main.rs` does `use statik::cli::commands`
  — importing via the crate name. **Fixed**: Resolver reads crate name from `Cargo.toml`
  `[package].name` and treats `use crate_name::` as equivalent to `use crate::`.
- [x] **`pub mod` re-export chains not followed**: Fixed transitively by crate_name
  resolution fix.
- [x] **False cycles from `mod.rs` barrel files**: The `graph.rs <-> file_graph.rs <-> mod.rs`
  cycle was caused by `mod.rs` re-exporting both modules. **Fixed**: `is_mod_declaration`
  flag on `FileImport` distinguishes structural `mod` edges from `use` edges; cycles
  consisting entirely of `mod` edges are filtered out.
- [x] **Dead export false-positive cascade**: Fixed transitively by crate_name
  resolution fix.

### 3b.6 Known limitations for Rust v1 (deferred)

Items deferred to future iterations:
- [ ] **Cargo workspace cross-crate resolution**: Cross-crate imports within a
  workspace are classified as external. Each crate is analyzed independently.
- [ ] **Proc macro expansion**: Derive macros, attribute macros, and function-like
  proc macros are not expanded. Generated code is invisible.
- [ ] **`#[macro_export]` detection**: `macro_rules!` definitions are always treated
  as Private. The `#[macro_export]` attribute is not recognized.
- [ ] **`#[cfg]` evaluation**: All conditional compilation branches are parsed
  unconditionally.
- [ ] **`#[path = "..."]` attribute support**: Custom module path attributes are not
  recognized. Only standard `foo.rs` / `foo/mod.rs` conventions are used.
- [ ] **Build script generated code**: Code generated by `build.rs` is not visible.
- [ ] **Feature flag resolution**: Cargo feature flags are not evaluated.

---

## Phase 4: Deep Analysis

### 4.1 Reference storage improvements
**Complexity**: L
**Prerequisites**: Phase 1 complete
**Files**: `src/parser/typescript.rs`, `src/db/mod.rs`

Tasks:
- [x] Audit the placeholder SymbolId system (`u64::MAX - counter`) in the parser
- [x] For intra-file references where both source and target symbols are defined
  in the same file, resolve to actual SymbolIds during parsing
- [x] Store resolved intra-file references in the DB (currently skipped for
  placeholder targets)
- [x] Add DB indexes for efficient reference queries by target
- [x] Ensure `clear_file_data()` correctly handles the new reference records

**Acceptance**: `SELECT count(*) FROM refs` returns a non-zero count for a project
with intra-file function calls.

---

### 4.2 Activate `symbols` command
**Complexity**: S
**Prerequisites**: 4.1
**Files**: `src/cli/commands.rs`, `src/main.rs`

Tasks:
- [x] Remove the `#[command(hide = true)]` from `Commands::Symbols`
- [x] Implement `run_symbols()` using existing DB queries (`get_symbols_by_file`,
  `find_symbols_by_name`, `find_symbols_by_kind`)
- [x] Add text and JSON output formatters
- [x] Add `--file` and `--kind` filters

**Acceptance**: `statik symbols --file src/utils.ts` lists all symbols in the file
with kind, name, line number, and visibility.

---

### 4.3 Activate `references` command
**Complexity**: M
**Prerequisites**: 4.1, 4.2
**Files**: `src/cli/commands.rs`, `src/main.rs`

Tasks:
- [x] Remove the `#[command(hide = true)]` from `Commands::References`
- [x] Implement `run_references()` that finds a symbol by name, then queries all
  references to it
- [x] Handle ambiguous symbol names (multiple symbols with the same name in
  different files) by showing all matches; `--file` filter narrows scope
- [x] Add `--kind` filter for reference kind (call, type_usage, inheritance, etc.)

**Acceptance**: `statik references MyClass` shows all files and line numbers where
`MyClass` is referenced.

---

### 4.4 Activate `callers` command
**Complexity**: S
**Prerequisites**: 4.3
**Files**: `src/cli/commands.rs`, `src/main.rs`

Tasks:
- [x] Remove the `#[command(hide = true)]` from `Commands::Callers`
- [x] Implement `run_callers()` as `run_references()` filtered to `RefKind::Call`
- [x] Show the calling function name and file for each call site

**Acceptance**: `statik callers helper` shows every function that calls `helper`,
with file and line number.

---

### 4.5 Symbol-level dead code detection
**Complexity**: L
**Prerequisites**: 4.1, 4.2
**Files**: `src/analysis/dead_code.rs`

Tasks:
- [x] Add `DeadCodeScope::Symbols` variant
- [x] For each exported symbol, check if it has any intra-project references
  (excluding self-references and same-file references)
- [x] For non-exported symbols, check if they have any references at all (truly
  dead internal code)
- [x] Report dead symbols with confidence levels (lower confidence for symbols in
  files with unresolved imports)
- [x] Ensure entry point files' symbols are not reported as dead

**Acceptance**: `statik dead-code --scope symbols` identifies unused internal
functions that are not exported and not called by any other function in the project.

---

### 4.6 Type-only dependency separation
**Complexity**: M
**Prerequisites**: None (Phase 1 level, but thematically Phase 4)
**Files**: `src/analysis/dependencies.rs`, `src/analysis/impact.rs`,
`src/cli/commands.rs`

Tasks:
- [x] Add `--runtime-only` flag to `deps` command
- [x] Add `--runtime-only` flag to `impact` command
- [x] When `--runtime-only`, filter out `FileImport` edges where `is_type_only`
  is true
- [x] Update the `FileGraph` construction to propagate `is_type_only` from
  `ImportRecord` to `FileImport` (currently hardcoded `false` at `commands.rs:106`)

**Acceptance**: `statik deps --runtime-only src/types.ts` shows fewer dependencies
than `statik deps src/types.ts` when the file has `import type` statements.

---

### 4.7 Java inheritance and annotation extraction
**Complexity**: M
**Prerequisites**: 3.3 (Java parser)
**Files**: `src/parser/java.rs`

Tasks:
- [x] Extract `extends` and `implements` relationships as references with
  `RefKind::Inheritance`
- [x] Extract annotation usage (`@Override`, `@Autowired`, etc.) as references
  with `RefKind::TypeUsage`
- [x] Add `statik deps --direction in` support for Java inheritance (what extends
  this class?)
- [x] Add tests for Java inheritance hierarchies

**Acceptance**: `statik impact UserService.java` includes files that extend
`UserService` in the affected list.

**Status**: Phase 4 complete. All items (4.1–4.7) implemented and verified.

---

## Phase 5: Refactoring Intelligence

### 5.1 Dual-index comparison engine
**Complexity**: L
**Prerequisites**: 1.7 (structural diff foundation)
**Files**: `src/analysis/diff.rs`

Tasks:
- [ ] Load two `Database` instances (old and new)
- [ ] Build `FileGraph` for each
- [ ] Compare file sets: added, removed, modified (by mtime or content hash)
- [ ] Compare export surfaces per file: added exports, removed exports
- [ ] Compare import edges: added edges, removed edges
- [ ] Detect moved exports: export removed from file A and added to file B with
  the same name and kind

**Acceptance**: Given two indexes, the comparison correctly identifies a function
that was moved from `utils.ts` to `helpers.ts` as a restructuring change.

---

### 5.2 Breaking change detection
**Complexity**: M
**Prerequisites**: 5.1
**Files**: `src/analysis/diff.rs`

Tasks:
- [ ] For each removed export, check if any file in the new index imported it
- [ ] For each renamed export (detected via move heuristic), check if importers
  updated their import statements
- [ ] Classify: breaking (removed export with existing importers), safe (removed
  export with no importers), restructuring (moved with importers updated)
- [ ] Assign confidence: Certain for removals, High for renames, Medium for
  restructuring detection

**Acceptance**: Removing an exported function that is imported by 3 files is
reported as a breaking change with Certain confidence and lists the 3 affected files.

---

### 5.3 Git integration for diff
**Complexity**: L
**Prerequisites**: 5.1, 5.2
**Files**: New `src/git.rs`, `src/cli/commands.rs`

Tasks:
- [ ] Add `statik diff <ref1> <ref2>` command that accepts git refs (commit SHA,
  branch name, `HEAD~1`, etc.)
- [ ] Use `git show <ref>:<path>` or `git archive` to read source files at specific
  revisions
- [ ] Index each revision into a temporary database (or cache indexed revisions in
  `.statik/snapshots/<sha>.db`)
- [ ] Run comparison engine on the two indexes
- [ ] Optimize: use `git diff --name-only <ref1> <ref2>` to identify changed files
  and only re-index those
- [ ] Add `--cached` flag to compare staged changes vs HEAD

**Acceptance**: `statik diff HEAD~1 HEAD` shows structural changes between the
last two commits. Cached snapshots make repeated comparisons fast.

---

### 5.4 CI integration mode
**Complexity**: M
**Prerequisites**: 5.3
**Files**: `src/cli/commands.rs`

Tasks:
- [ ] Add `statik diff --ci` flag that outputs machine-readable JSON
- [ ] Exit code 0 if no breaking changes, 1 if breaking changes detected
- [ ] Add `--allow-breaking` flag to document intentional breaking changes
  (exit 0 but still report)
- [ ] Add `--threshold` flag: only fail if more than N breaking changes
- [ ] Document GitHub Actions / GitLab CI integration in README

**Acceptance**: A GitHub Action running `statik diff --ci origin/main HEAD`
correctly blocks a PR that removes a public export used by other files.

---

### 5.5 Cycle introduction/resolution tracking
**Complexity**: M
**Prerequisites**: 5.1
**Files**: `src/analysis/diff.rs`, `src/analysis/cycles.rs`

Tasks:
- [ ] Run cycle detection on both old and new indexes
- [ ] Report new cycles introduced (exist in new but not old)
- [ ] Report cycles resolved (exist in old but not new)
- [ ] Include in `statik diff` output

**Acceptance**: Adding an import that creates a circular dependency is reported
as "new cycle introduced" in `statik diff`.

---

## Phase 6: Ecosystem & Integrations

### 6.1 JSON output schema stabilization
**Complexity**: M
**Prerequisites**: Phases 1-4
**Files**: `src/cli/commands.rs`, new `schema/` directory

Tasks:
- [ ] Define JSON schema for each command's output using JSON Schema or TypeScript
  types
- [ ] Add schema version field to all JSON output
- [ ] Document backward compatibility policy (additive changes only within a major
  version)
- [ ] Add schema validation tests that verify output matches the declared schema
- [ ] Publish schema files alongside releases

**Acceptance**: External tools can depend on the JSON output format with confidence
that it won't break between minor versions.

---

### 6.2 Dependency graph visualization
**Complexity**: M
**Prerequisites**: 6.1
**Files**: New `src/cli/graph.rs`

Tasks:
- [ ] Add `statik graph` command
- [ ] Output DOT format for use with Graphviz
- [ ] Add `--format svg` that shells out to `dot` if available
- [ ] Add `--format html` that generates a self-contained HTML file with an
  interactive graph (use a JS library like d3-force or vis.js embedded in the HTML)
- [ ] Support `--focus <file>` to show only the neighborhood of a specific file
- [ ] Support `--depth <n>` to limit the graph depth

**Acceptance**: `statik graph --format html --focus src/index.ts --depth 2`
produces an HTML file that shows the 2-hop neighborhood of `index.ts` with
interactive zoom and pan.

---

### 6.3 VS Code extension
**Complexity**: L
**Prerequisites**: 6.1
**Files**: New `vscode-statik/` directory

Tasks:
- [ ] Create VS Code extension project (TypeScript)
- [ ] Run `statik` CLI commands as child processes
- [ ] Show dead code as diagnostic warnings (yellow squiggles)
- [ ] Show impact analysis on hover or via code lens ("12 files affected by
  changes to this export")
- [ ] Show dependency graph in a webview panel
- [ ] Add "Go to importers" and "Go to imports" code actions
- [ ] Publish to VS Code marketplace

**Acceptance**: Opening a TS project in VS Code with the extension installed shows
dead export warnings and impact information without manual configuration.

---

### 6.4 GitHub Action
**Complexity**: M
**Prerequisites**: 5.4 (CI mode), 6.1
**Files**: New `action.yml`, `action/` directory

Tasks:
- [ ] Create GitHub Action that installs the `statik` binary
- [ ] Run `statik diff` comparing the PR branch against the base branch
- [ ] Post a comment on the PR with the diff summary (added/removed exports,
  breaking changes, new cycles)
- [ ] Set check status based on breaking change threshold
- [ ] Support configuration via `.statik/ci.yml`

**Acceptance**: A GitHub repository using the action gets automatic structural
diff comments on every PR.

---

### 6.5 Watch mode
**Complexity**: L
**Prerequisites**: 1.5 (graph caching)
**Files**: New `src/cli/watch.rs`, `Cargo.toml` (add `notify` dependency)

Tasks:
- [ ] Add `statik watch` command using the `notify` crate for filesystem events
- [ ] On file change: re-parse only the changed file, update the DB incrementally
- [ ] Invalidate and rebuild the graph cache for affected edges
- [ ] Optionally re-run a specified analysis on each change (e.g., `statik watch
  --run "dead-code"`)
- [ ] Handle rapid successive changes with debouncing

**Acceptance**: After `statik watch` is running, editing a file and saving triggers
an incremental re-index within 500ms.

---

### 6.6 Language Server Protocol (exploratory)
**Complexity**: XL
**Prerequisites**: Phases 1-5 complete, 6.5 (watch mode)
**Files**: New `src/lsp/` module

Tasks:
- [ ] Evaluate whether LSP provides value beyond the VS Code extension
- [ ] Implement basic LSP server with `textDocument/definition` (find where an
  import resolves to)
- [ ] Implement `textDocument/references` (find all importers of an export)
- [ ] Implement custom LSP methods for impact analysis and dead code
- [ ] Test with multiple editors (VS Code, Neovim, Helix)

**Acceptance**: This is exploratory. Success criteria: a working prototype that
demonstrates cross-language dependency navigation in at least two editors.

---

## Phase 7: Agent-Friendly CLI

AI agents currently need Python/jq/grep to post-process statik's output. These
additions eliminate that friction. Design principles from gh CLI: pre-filter
instead of post-process, composable flags, consistent JSON schema.

### 7.1 Output path filtering (`--path-filter`) ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

The single highest-impact CLI addition. Agents cannot currently say "show me
dead code in `src/model/` only" without post-processing.

Tasks:
- [x] Add `--path-filter <glob>` global flag to `Cli` struct
- [x] Apply as output filter: only include results where file path matches the
  glob pattern
- [x] Works with all commands: dead-code, deps, cycles, exports, lint, impact,
  symbols, references, callers
- [x] Composable with `--lang` (both filters apply)
- [x] Add tests

**Acceptance**: `statik dead-code --path-filter "src/model/**"` returns only dead code
in the model directory, without requiring any post-processing.

---

### 7.2 Count mode (`--count`) ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Just print the count of results. Eliminates `| jq '. | length'` and `| wc -l`.

Tasks:
- [x] Add `--count` global flag to `Cli` struct
- [x] When set, replace normal output with a single number
- [x] Works with all commands that produce lists
- [x] Composable with `--path-filter` and `--lang`
- [x] Exit code based on count

**Acceptance**: `statik dead-code --count` prints `42`. Combined:
`statik dead-code --count --path-filter "src/model/"` prints `7`.

---

### 7.3 Result limiting (`--limit`) ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Limit output to first N results. Reduces context window waste for agents.

Tasks:
- [x] Add `--limit <N>` global flag to `Cli` struct
- [x] Truncate result list before serialization
- [x] Works with all commands that produce lists
- [x] Composable with `--path-filter`, `--lang`, `--sort`

**Acceptance**: `statik dead-code --limit 10` shows only the first 10 dead
code results.

---

### 7.4 Sort control (`--sort`) ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Deterministic, meaningful ordering. Agents benefit from seeing worst offenders
first.

Tasks:
- [x] Add `--sort <field>` global flag: `path`, `confidence`, `name`, `depth`
- [x] Default: `path` (alphabetical, deterministic)
- [x] `confidence`: high-confidence results first
- [x] `depth`: for impact command, deepest impact first
- [x] Add `--reverse` flag for descending order
- [x] Add tests for sort fields

**Acceptance**: `statik dead-code --sort confidence` shows high-confidence dead
code first.

---

### 7.5 Built-in jq filtering (`--jq`) ✅
**Complexity**: M
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/main.rs`, `Cargo.toml`

Like GitHub CLI's `--jq` flag. Eliminates all post-processing for agents.
Uses `jaq-interpret` + `jaq-parse` crates (pure Rust jq implementation) with
a minimal standard library (empty, select, map, not, type, etc.) defined via
core constructs.

Tasks:
- [x] Add `--jq <expression>` global flag
- [x] Implicitly sets `--format json`
- [x] Apply jq filter to the JSON output before printing
- [x] Handle jq errors gracefully

**Acceptance**: `statik dead-code --jq '.[].path'` prints one file path per
line without requiring external jq.

---

### 7.6 Richer JSON schema ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/output.rs`, `src/cli/commands.rs`

Add structured path components to JSON output so agents don't need to parse
file paths.

Tasks:
- [x] Add `directory`, `filename`, `extension` fields alongside `path` in all
  JSON output
- [ ] Add `module` field (top-level source directory: model, parser, cli, etc.)
- [x] Add `language` field to all file-level results
- [x] Ensure backward compatibility (new fields are additive)
- [x] Add tests verifying JSON schema

**Acceptance**: JSON output for a dead export includes
`{"path": "src/model/foo.rs", "directory": "src/model", "filename": "foo.rs",
"extension": "rs", "module": "model", "language": "rust", ...}`.

---

### 7.7 Cross-module edge filter (`--between`) ✅
**Complexity**: S
**Prerequisites**: 7.1
**Files**: `src/cli/commands.rs`

Show only edges between two path scopes. Very useful for understanding
cross-module coupling.

Tasks:
- [x] Add `--between <from_glob> <to_glob>` flag to `deps` command
- [x] Filter edge list: only include edges where source matches `from_glob`
  and target matches `to_glob`
- [x] Works with `--format json`, `--count`, `--limit`

**Acceptance**: `statik deps --between "src/parser/**" "src/model/**"` shows
only edges from parser files to model files.

---

### 7.8 CSV output format ✅
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/output.rs`

Trivially parseable without any JSON library. Useful for agents that want
simple text processing.

Tasks:
- [x] Add `csv` to the `OutputFormat` enum
- [x] Implement CSV serialization for all commands (header row + data rows)
- [x] Use comma separator, quote fields containing commas

**Acceptance**: `statik dead-code --format csv` produces parseable CSV with
headers `path,name,kind,confidence`.

---

### 7.9 Directory-level summary aggregation ✅
**Complexity**: M
**Prerequisites**: None
**Files**: `src/cli/commands.rs`

Roll up statistics per directory for high-level architectural overview.

Tasks:
- [x] Add `--by-directory` flag to `summary` command
- [x] Aggregate: files, exports, dead exports, avg fan-out, avg fan-in per
  directory
- [x] Works with `--format json`, `--sort`, `--limit`

**Acceptance**: `statik summary --by-directory --format json` produces
per-directory rollup showing file counts, export counts, dead export counts,
and coupling metrics.

---

## Phase 10: Human / Committer Analysis (VCS History Intelligence)

statik answers "what does this code touch?" — the graph of files, symbols,
dependencies, blast radius. The missing dimension is "who does this code touch?"
Git history is the richest signal for this: every commit is a record of a human
interacting with a file. Combined with statik's existing `FileGraph`, this
projects the code graph onto a *people graph*.

No other CLI tool does this. `git blame` shows who wrote a line. `git log`
shows who touched a file. Neither follows the dependency graph to answer "whose
code breaks if I change this?" That question today requires tribal knowledge —
statik can make it computable.

### 10.1 Git history extraction and storage ✅
**Complexity**: M
**Prerequisites**: None
**Files**: `src/git.rs`, `src/db/mod.rs`, `src/cli/index.rs`

Extract per-file commit history from `git log` and store it alongside the
existing index. This is the foundation for all ownership and committer analysis.

Tasks:
- [x] Add `git log --numstat --format='%H|%an|%ae|%at' --follow` wrapper to
  `src/git.rs` for extracting commit-level file changes with author, email,
  and timestamp
- [x] Define `CommitRecord` struct: `sha`, `author_name`, `author_email`,
  `timestamp`, `files: Vec<(String, u32, u32)>` (path, lines added, removed)
- [x] Add `commits` table to SQLite schema: `sha TEXT PK`, `author_name TEXT`,
  `author_email TEXT`, `timestamp INTEGER`
- [x] Add `file_commits` junction table: `file_path TEXT`, `commit_sha TEXT`,
  `lines_added INTEGER`, `lines_removed INTEGER`
- [x] Integrate into `statik index` with `--with-history` flag (opt-in, since
  `git log` can be slow on very large repos)
- [x] Incremental: store the last-indexed commit SHA in DB metadata; on
  re-index, only process commits after that SHA
- [x] Add `--history-depth <N>` flag to limit how far back to scan (default:
  all history; useful for repos with 100K+ commits)
- [x] Add tests with a temp git repo fixture

**Acceptance**: `statik index --with-history` populates the commits table.
Re-running on the same repo only processes new commits.

---

### 10.2 Ownership model and `statik owners` command ✅
**Complexity**: M
**Prerequisites**: 10.1
**Files**: new `src/analysis/ownership.rs`, `src/cli/commands.rs`

Compute per-file ownership scores from commit history using a weighted model
that values recency, volume, and frequency.

Tasks:
- [x] Implement ownership scoring model:
  - Recency weight: exponential decay from most recent commit (half-life
    configurable, default ~180 days)
  - Volume weight: lines added + removed per commit
  - Frequency weight: number of commits touching the file
  - Final score per author per file: `sum(recency * volume)` normalized to
    percentage
- [x] Add `statik owners <glob>` command: for each matching file, output
  ranked list of authors with ownership percentage
- [x] Support `--top <N>` to show only the top N owners per file (default: 3)
- [ ] Support directory-level aggregation: `statik owners "src/auth/"` rolls
  up ownership across all files in the directory
- [x] Text output: table with file path, author, percentage
- [x] JSON output: structured with `path`, `owners: [{name, email, score}]`
- [x] Works with `--format`, `--sort`, `--limit`, `--jq`, `--path-filter`
- [x] Add tests with known commit history and expected ownership scores

**Acceptance**: `statik owners "src/auth/**"` shows ranked owners per file.
The primary owner is the person who most recently and most frequently touched
the file, not just whoever made the last commit.

---

### 10.3 `statik who` — impact-aware reviewer suggestion ✅
**Complexity**: M
**Prerequisites**: 10.2, existing `impact` command
**Files**: `src/analysis/who.rs`, `src/cli/commands.rs`, `src/cli/mod.rs`

The highest-value command: combine the dependency graph blast radius with
ownership to answer "if I change this file, who should I talk to?"

Tasks:
- [x] Add `statik who <file>` command that:
  1. Runs blast radius analysis (same as `statik impact`) to get affected files
  2. Computes ownership for all affected files
  3. Aggregates: rank people by total ownership weight across all affected files
  4. Groups output: direct owners (of the target file) vs downstream owners
     (own files affected via dependency graph)
- [x] Output includes "suggested reviewers": minimal set of people covering
  all affected areas (greedy set-cover by ownership weight)
- [x] Support `--max-depth <N>` to limit blast radius depth (via global flag)
- [x] Text output: sections for direct owners, downstream owners, suggested
  reviewers
- [x] JSON output: structured with `direct_owners`, `downstream_owners`,
  `suggested_reviewers`, `affected_files` with per-file owners
- [x] Add tests: file with blast radius spanning multiple owners

**Acceptance**: `statik who src/api/users.ts` shows direct owners and
downstream owners affected via the dependency graph. Suggested reviewers is a
minimal covering set. This is substantially more useful than `git blame`
because it follows dependencies, not just file history.

---

### 10.4 `statik bus-factor` — knowledge concentration risk ✅
**Complexity**: S
**Prerequisites**: 10.2
**Files**: `src/analysis/ownership.rs`, `src/cli/commands.rs`

Identify files and modules where knowledge is concentrated in too few people.
Combined with fan-in from the dependency graph, this surfaces organizational
risk: a bus-factor-1 file that is imported by 50 other files is a critical
liability.

Tasks:
- [x] Add `statik bus-factor [glob]` command
- [x] Compute bus factor per file: count of authors with >10% ownership
  (threshold configurable via `--threshold`)
- [x] Cross-reference with fan-in (number of dependents) to compute a
  composite risk score: `risk = fan_in / bus_factor`
- [x] Sort by risk descending by default
- [x] Text output: table with risk level, file, bus factor, primary owner,
  dependent count
- [x] JSON output: structured with `path`, `bus_factor`, `owners`,
  `fan_in`, `risk_score`
- [x] Add tests with known single-owner and multi-owner files

**Acceptance**: `statik bus-factor --sort risk` shows the most
organizationally risky files first — high fan-in with a single dominant author.

**Known bug (confirmed by external evaluation)**: `fan_in` is still 0 on real
multi-module projects despite suffix matching fix. The file graph stores
absolute paths, commit history stores relative paths, and the suffix match in
`fan_in_suffix_lookup` doesn't handle all cases. Files with 200+ importers
(confirmed by `fan-in-limit` lint rule) report `fan_in: 0` in `bus-factor`.
`risk_score` is therefore always 0.0. See 10.4b for the fix.

---

### 10.5 `statik churn` — change frequency and co-change analysis ✅
**Complexity**: M
**Prerequisites**: 10.1
**Files**: new `src/analysis/churn.rs`, `src/cli/commands.rs`

Change frequency analysis surfaces hot spots and hidden coupling that the
import graph doesn't capture. Files that frequently change together but have
no import relationship may have implicit coupling worth investigating.

Tasks:
- [x] Add `statik churn [glob]` command: for each file, output commit count,
  total lines changed, and change frequency (commits per month)
- [ ] Support `--since <date>` and `--until <date>` for time-windowed analysis
- [x] Add `--co-change` mode: identify file pairs that change together in the
  same commit significantly more often than chance
  - For each pair, compute: co-change count, co-change ratio (co-changes /
    max(changes_a, changes_b)), and whether an import edge exists between them
  - Flag pairs with high co-change ratio but no import edge as "hidden
    coupling"
- [x] Text output: table sorted by change frequency
- [x] JSON output: structured with `path`, `commit_count`, `lines_changed`,
  `frequency`, and for co-change mode: `pairs` with correlation data
- [x] Works with `--format`, `--sort`, `--limit`, `--path-filter`
- [x] Add tests

**Acceptance**: `statik churn --sort frequency` shows the most frequently
changed files. `statik churn --co-change` identifies file pairs that change
together but have no direct dependency relationship.

---

### 10.6 Team boundary analysis (optional, config-driven) ✅
**Complexity**: M
**Prerequisites**: 10.2
**Files**: `src/analysis/teams.rs`, `src/linting/config.rs`, `src/cli/mod.rs`,
`src/cli/commands.rs`

When a people-to-team mapping is available (via email domain patterns or
explicit config), detect misalignment between team boundaries and code
boundaries — Conway's Law violations.

Tasks:
- [x] Add optional `[teams]` config section to `.statik/rules.toml`:
  ```toml
  [teams]
  platform = ["*@platform.example.com", "alice@example.com"]
  product = ["*@product.example.com"]
  infra = ["*@infra.example.com"]
  ```
- [x] Alternatively, infer teams from email domains when no config exists
- [x] Add `statik team-coupling [glob]` command: for each file/directory,
  show how many teams have contributed commits (cross-team coordination cost)
- [x] Flag files that require cross-team coordination (>2 teams with >10%
  ownership each)
- [x] Cross-reference with dependency graph: dependency edges that cross
  team boundaries are higher-friction than intra-team edges
- [x] Text output: table with file, team distribution, cross-team edge count
- [x] JSON output: structured with team breakdowns
- [x] Add tests

**Acceptance**: `statik team-coupling` identifies files and dependency edges
that cross team boundaries, highlighting organizational coordination costs.

---

### 10.4b BUG: Fix bus-factor `fan_in` path matching
**Complexity**: S
**Prerequisites**: 10.4
**Files**: `src/analysis/ownership.rs`

**The bug**: `fan_in` is always 0 on real multi-module projects. The suffix
matching in `fan_in_suffix_lookup` was added to bridge absolute paths (file
graph) vs relative paths (commit history), but it fails on projects where the
file graph paths and git log paths have non-trivial prefix differences (e.g.,
multi-module Gradle/Maven projects with deep module paths).

**Evidence**: A file confirmed to have 200+ importers via the `fan-in-limit`
lint rule reports `fan_in: 0` in `bus-factor`. The lint system uses `FileId`
lookups (correct), but bus-factor uses string path matching (broken).

Tasks:
- [x] Root-cause the path mismatch: compare how `fan_in_by_path` keys look vs
  how `file_stats` paths look in a multi-module project
- [x] Fix: use `FileId`-based lookup instead of string path matching. The
  `FileGraph` already maps paths to `FileId`s — look up fan_in by `FileId`,
  then join with ownership data by path
- [x] Add a test that catches this: create a file graph with absolute paths
  and commit history with relative paths, verify non-zero `fan_in`
- [x] Verify with `statik bus-factor` on a real multi-module project

**Acceptance**: `statik bus-factor` reports non-zero `fan_in` for files that
have importers. `risk_score` is meaningful, not always 0.0.

---

### 10.4c Per-person bus factor view (`--by-author`)
**Complexity**: S
**Prerequisites**: 10.4b (fan_in fix)
**Files**: `src/analysis/ownership.rs`, `src/cli/commands.rs`

**Motivation**: External evaluation showed teams need to know "which *people*
are single points of failure?" — not just which files. Currently this requires
scripting `owners --top 1` output externally.

Tasks:
- [x] Add `--by-author` flag to `statik bus-factor`
- [x] Aggregate per author: count of files where they're sole owner (>80%),
  total files touched, top areas (directories) they solely own
- [x] Include total blast radius of their bus-factor-1 files (sum of fan_in)
- [x] Sort by sole-owned file count descending
- [x] Text/JSON output with author, sole-owned count, total files, key areas

**Acceptance**: `statik bus-factor --by-author` shows per-person ownership
concentration — how many files each person solely owns and which areas.

---

### 10.7 Adaptive ownership half-life
**Complexity**: S
**Prerequisites**: 10.2
**Files**: `src/analysis/ownership.rs`

**The problem**: The fixed 180-day half-life for recency weighting doesn't
scale with file age. For an 8-year-old file, the original creator's
contribution decays through ~16 half-lives (2^16 = 65,536x decay), making it
effectively zero — even though they're the person who actually understands the
code. A developer who made a 2-line tweak last year is reported as 99% owner.

**Evidence**: On a large project, a file created in 2018 (90 lines) had two
trivial edits in 2024 (5 lines total). Statik reports the tweaker as 99.3%
owner because the creator's contribution has decayed to nearly zero.

Possible approaches:
- **Adaptive half-life**: `half_life = max(180, file_age_days * 0.25)`. An
  8-year-old file gets a ~2-year half-life; a 6-month-old file stays at 180d.
- **`git blame`-based metric**: who wrote the lines still in the file today.
  More expensive but directly measures surviving contribution.
- **Hybrid**: commit-based for "who maintains this" + blame-based for "who
  built this" — surface both.

Tasks:
- [x] Implement adaptive half-life: scale with file age
- [x] Add `--half-life-mode` flag: `fixed` (current), `adaptive` (new default)
- [ ] Consider adding `git blame`-based scoring as an alternative
- [x] Add tests comparing fixed vs adaptive on known old/new file scenarios

**Acceptance**: For an 8-year-old file with a recent trivial edit, the original
creator retains meaningful ownership percentage (>10%) rather than decaying to
near-zero.

---

### 10.8 Wildcard import source set boundary enforcement
**Complexity**: S
**Prerequisites**: 8.1 (source sets)
**Files**: `src/resolver/java.rs`

**The bug**: Java wildcard imports (`import package.*`) resolve to files across
source set boundaries. For example, a wildcard import in a production source
set can resolve to test files in a different module's test source set, creating
false dependency edges and false positives in boundary lint rules.

Tasks:
- [x] When resolving wildcard imports, filter candidate files by source set
  visibility — a production source set should not resolve to test source sets
  in other modules
- [x] Add source set dependency declaration (already exists in config as
  `depends_on`): a source set can only resolve imports to its own files or
  files in declared dependencies
- [x] Add test: wildcard import does not resolve across source set boundaries

**Acceptance**: Wildcard imports in a production source set do not create
edges to test files in other source sets.

---

### 10.9 Suppress unknown language warnings during indexing
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/index.rs`

**The problem**: Projects with vendored libraries in unsupported languages
(e.g., Python bindings in a Java project) produce 100+ "no parser for
language" warnings during indexing. These are harmless but noisy.

Tasks:
- [x] Silently skip files in languages without a registered parser (no warning)
- [ ] Add `--warn-unsupported` flag to opt into the current warning behavior
- [x] Support `--exclude "*.py"` on `statik index` to filter
  files before language detection

**Acceptance**: `statik index` on a project with vendored files in unsupported
languages produces no warnings by default.

---

## External Evaluation Findings

Summary of feedback from an external evaluation on a large multi-module project
(~7K files, 130K commits, multi-module Gradle, mixed Java/JS).

**What works well**:
- Source sets are essential and transformative for multi-module projects (7x
  more symbols resolved, 9x faster indexing with source sets configured)
- Dead code detection found genuinely dead files with high confidence
- Architectural linting caught god objects (200+ importers) and confirmed
  clean module boundaries
- `deps --between` correctly analyzed cross-module coupling
- Churn with deep history (50K commits) accurately identified hot spots
- Co-change analysis found hundreds of hidden couplings including
  framework-level coupling invisible to static analysis
- Performance is excellent: 15s full index, 1.6s incremental, ~2s lint on 7K files

**What was fixed**:
- ~~Bus-factor `fan_in` always 0 (path mismatch bug)~~ — 10.4b ✅
- ~~Wildcard imports cross source set boundaries~~ — 10.8 ✅
- ~~Ownership recency weighting overvalues trivial recent edits~~ — 10.7 ✅
- ~~Unknown language warnings are noisy~~ — 10.9 ✅
- ~~Per-person bus factor view needed~~ — 10.4c ✅

---

## Phase 11: SCIP Ingestion (Compiler-Grade Precision)

Optional enrichment layer: ingest SCIP (Source Code Intelligence Protocol)
indexes from external language servers/compilers to get fully type-resolved
references without embedding any compiler. Statik stays fast for tree-sitter
indexing; SCIP data upgrades precision when available.

**Key insight**: Statik's value is graph algorithms (impact, dead code, cycles,
lint, ownership). SCIP ingestion lets purpose-built indexers handle parsing
precision while statik focuses on graph intelligence.

**Benchmarks** (from external testing on a 2.5M-line multi-module codebase):

| Operation | Time | Index size |
|-----------|------|------------|
| `scip-clang` (7K C++ TUs) | 19s | 33MB |
| `scip-java` cold (6.7K Java files) | 49s (35s compile + 14s convert) | 179MB |
| `scip-java` incremental (1 file) | 25s (10s compile + 15s convert) | — |
| Both in parallel | ~49s (bottleneck: Java) | 212MB total |

Note: `scip-clang` is much simpler — no build needed, just reads
`compile_commands.json`. `scip-java` requires instrumenting the Gradle build
with a `semanticdb-javac` plugin and has no incremental conversion mode
(the `index-semanticdb` step always reprocesses all files, ~15s floor).

**Design principle**: Tree-sitter is always the fast path. `statik index` and
`statik lint` never require a build. SCIP is generated in CI (where the build
is already happening) and optionally pulled by developers for local precision.
When a file's mtime is newer than the SCIP index, statik falls back to
tree-sitter for that file — stale SCIP is worse than no SCIP.

### 11.1 SCIP index reader
**Complexity**: M
**Prerequisites**: None
**Files**: new `src/scip/mod.rs`, `Cargo.toml`

The official [`scip` crate](https://crates.io/crates/scip) (v0.6.1) provides
Rust bindings with protobuf types (`Index`, `Document`, `Occurrence`,
`SymbolInformation`) and utility functions. No need to vendor the schema or
use `prost` directly.

`scip-clang` is a standalone C++ binary (links Clang/LLVM, builds with Bazel)
— not embeddable as a library. The integration is: `scip-clang` runs
externally and produces `index.scip`, which statik reads via the `scip` crate.

Tasks:
- [ ] Add `scip` crate dependency to `Cargo.toml`
- [ ] Implement SCIP index reader using the crate's `Index`, `Document`,
  `Occurrence` types
- [ ] Map SCIP symbol roles (definition, reference, import) to statik's
  `SymbolKind`, `RefKind`, `ImportRecord` types
- [ ] Map SCIP symbol names to statik's `SymbolId` scheme
- [ ] Handle multi-language SCIP indexes (different documents may be different
  languages)
- [ ] Benchmark: reading a 179MB `.scip` file should complete in <5s
- [ ] Add tests with a small hand-crafted `.scip` file

**Acceptance**: A `.scip` file can be parsed and its symbols/references
mapped to statik's type system. A 179MB SCIP index is read in <5s.

---

### 11.2 `statik enrich` command
**Complexity**: M
**Prerequisites**: 11.1
**Files**: `src/cli/commands.rs`, `src/main.rs`, `src/db/mod.rs`

Import one or more SCIP indexes into the existing SQLite database, upgrading
heuristic tree-sitter references to precise compiler-resolved references.

Tasks:
- [ ] Add `Commands::Enrich` variant with `<scip-file>...` argument (accepts
  multiple files)
- [ ] Implement merge logic: for files present in both tree-sitter index and
  SCIP index, replace heuristic references with SCIP references
- [ ] Retain tree-sitter data for files not covered by the SCIP index
- [ ] Add `enriched: bool` or `source: enum { TreeSitter, Scip }` to reference
  records in the DB
- [ ] Store the SCIP index generation timestamp for staleness tracking
- [ ] Update confidence system: SCIP-sourced references get Certain confidence
- [ ] Support multiple enrichments (e.g., `enrich java.scip cpp.scip`)
- [ ] Only store resolved edges in SQLite, not full SCIP payload (keep DB
  small even when SCIP indexes are large)
- [ ] Benchmark: ingesting 212MB of SCIP data should complete in <5s
- [ ] Add tests: enrich resolves previously-unresolved imports

**Acceptance**: `statik enrich project.scip` imports SCIP data. `statik
dead-code` after enrichment shows fewer unresolved imports and higher
confidence on more files. Ingestion completes in <5s.

---

### 11.3 Staleness tracking
**Complexity**: S
**Prerequisites**: 11.2
**Files**: `src/cli/commands.rs`, `src/db/mod.rs`

When a file is edited after the SCIP index was generated, its SCIP data is
stale. Statik must fall back to tree-sitter for that file automatically.

Tasks:
- [ ] Store SCIP generation timestamp in DB metadata per enrichment
- [ ] Per-file mtime comparison: if file mtime > SCIP timestamp, mark that
  file's SCIP references as stale
- [ ] Stale files use tree-sitter resolution (same as unenriched files)
- [ ] Surface staleness in `statik summary`: "X files enriched, Y stale,
  Z tree-sitter only"
- [ ] Add `--precise` flag to precision-sensitive commands (`dead-code`,
  `impact`) that warns if >10% of files have stale SCIP data
- [ ] Add tests

**Acceptance**: After enriching then editing a file, `statik dead-code` uses
tree-sitter resolution for the edited file and SCIP for unchanged files.

---

### 11.4 C++ support via scip-clang
**Complexity**: M
**Prerequisites**: 11.2
**Files**: `src/model/mod.rs`, `src/discovery/mod.rs`

With SCIP ingestion working, C++ is automatically supported — no tree-sitter
C++ parser needed. `scip-clang` is fast (19s for 7K TUs) and simple (reads
`compile_commands.json` directly, no build instrumentation needed).

Tasks:
- [ ] Add `Language::Cpp` variant (and `Language::C`, `Language::Header`)
- [ ] Add file discovery for `.cpp`, `.cc`, `.cxx`, `.c`, `.h`, `.hpp`, `.hxx`
- [ ] Add default exclude patterns for C++ projects (`build/`, `cmake-build-*/`,
  `*.o`, `*.a`)
- [ ] Ensure `statik enrich` correctly handles C++ SCIP indexes from
  `scip-clang`
- [ ] Test: index a C++ project via `scip-clang`, enrich, verify deps/impact
  work

**Acceptance**: `statik deps SomeFile.cpp` works after enriching with a
`scip-clang` index. `statik impact SomeFile.cpp` shows the correct blast
radius.

---

### 11.5 Cross-language dependency edges
**Complexity**: L
**Prerequisites**: 11.2
**Files**: `src/model/file_graph.rs`, `src/cli/commands.rs`, new
`src/linting/config.rs` additions

When multiple SCIP indexes are imported (e.g., Java + C++), create cross-
language edges based on configured boundary mappings. This is the highest-
value feature for mixed-language codebases.

Tasks:
- [ ] Add `[cross_language]` config section for mapping symbols across
  language boundaries
- [ ] Auto-detect common cross-language patterns (JNI, gRPC protobuf, shared
  symbol names)
- [ ] Create `FileImport` edges across language boundaries
- [ ] Ensure `statik impact` traces across language boundaries
- [ ] Ensure `statik dead-code` considers cross-language usage
- [ ] Add tests: change in C++ file shows affected Java files via cross-
  language edges

**Acceptance**: `statik impact SomeFile.cpp` traces through the C++ graph,
crosses the language boundary, and shows affected Java files.

---

### 11.6 Confidence upgrade for enriched data
**Complexity**: S
**Prerequisites**: 11.2
**Files**: `src/analysis/dead_code.rs`, `src/analysis/impact.rs`

Tasks:
- [ ] When SCIP data is available for a file, upgrade all references to
  Certain confidence
- [ ] In dead code analysis, skip the "has unresolved imports" confidence
  downgrade for SCIP-enriched files
- [ ] In impact analysis, mark SCIP-resolved edges as precise (vs heuristic)
- [ ] Surface enrichment status in `statik summary`: "X files enriched via
  SCIP, Y files tree-sitter only"
- [ ] Add tests

**Acceptance**: `statik dead-code` after enrichment reports higher confidence
on enriched files. `statik summary` shows enrichment coverage.

---

## Summary of Complexity Estimates

| Phase | S tasks | M tasks | L tasks | XL tasks | Total tasks |
|-------|---------|---------|---------|----------|-------------|
| 1     | 0       | 4       | 2       | 0        | 7           |
| 2     | 3       | 6       | 0       | 0        | 10          |
| 2b    | 5       | 3       | 0       | 0        | 8           |
| 3     | 2       | 1       | 1       | 1        | 5           |
| 3b    | 1       | 0       | 2       | 0        | 3           |
| 4     | 2       | 3       | 2       | 0        | 7           |
| 5     | 0       | 3       | 2       | 0        | 5           |
| 6     | 0       | 3       | 2       | 1        | 6           |
| 7     | 7       | 2       | 0       | 0        | 9           |
| 10    | 1       | 5       | 0       | 0        | 6           |
| 11    | 2       | 3       | 1       | 0        | 6           |
| **Total** | **23** | **33** | **12** | **2** | **72** |

**Priority guidance**: Phase 2b (advanced lint rules), Phase 7 (agent-friendly
CLI), and Phase 10 core (10.1-10.5: git history, owners, bus-factor, churn) are
complete. Phase 3b (Rust support) is complete including dogfooding fixes.
Phase 10 dogfooding fixes (10.4b, 10.4c, 10.7, 10.8, 10.9) are now complete.
Phase 10.3 (`statik who` — impact-aware reviewer suggestion) is now complete.
Phase 10.6 (`statik team-coupling` — team boundary analysis) is now complete.
**Phase 10 is now fully complete.**
Phase 8 dogfooding fixes: 8.2, 8.3, 8.4, 8.6, 8.7 are complete. Remaining:
8.1 (source sets), 8.5 (Java multi-module source roots).
Phase 9 external dogfooding: 9.1-9.5, 9.6, 9.7-9.11 are complete. Phase 9
is now fully complete.
**Highest-priority next work**: 8.1 (source sets) for scope classification.
**Phase 11 (SCIP ingestion)** is a strategic priority — validated by external
evaluation as "the cleanest path to making the graph analysis actually precise."
Can be built alongside any other phase.

---

## Phase 8: Dogfooding-Driven Fixes

Findings from running statik v2 on its own codebase. Organized around generic
solutions rather than language-specific special cases.

### 8.1 Source sets: config-driven scope classification
**Complexity**: L
**Prerequisites**: None
**Files**: `src/linting/config.rs`, `src/cli/commands.rs`, `src/model/file_graph.rs`

**The problem**: Test code, generated code, benchmarks, and fixtures are treated
the same as production code. This causes false positives everywhere:
- `dead-code --scope symbols` reports 3729 dead symbols, mostly `#[test]` fns
- `statik lint` flags test-only imports as architecture violations
- `statik dead-code` reports 297 dead files that are all test fixtures
- Entry point detection is hardcoded per-language (`.test.`, `.spec.`, `*Test.java`,
  `main.rs`, `lib.rs`, `@Test`, `@SpringBootApplication`, etc.)

**The generic solution**: Let the config define **source sets** -- named groups
of files with different roles. The concept maps directly to Gradle source sets,
Maven profiles, and Rust's `#[cfg]` system.

```toml
[scope]
# Files outside all source sets are treated as external (ignored in analysis).
# Default: everything discovered is in scope.

[scope.production]
include = ["src/main/java/**", "src/**/*.rs", "src/**/*.ts"]
exclude = ["src/**/test/**", "src/**/tests/**"]

[scope.test]
include = ["src/test/**", "tests/**", "**/*.test.*", "**/*.spec.*"]
role = "entry_point"      # all test files are entry points
lint = false              # test code is excluded from lint rules

[scope.fixture]
include = ["test-fixtures/**", "tests/fixtures/**"]
role = "entry_point"
analysis = false          # completely excluded from analysis output

[scope.generated]
include = ["src/generated/**", "build/generated-sources/**"]
lint = false              # don't lint generated code

[scope.benchmark]
include = ["benches/**", "benchmarks/**"]
role = "entry_point"
```

**How this replaces hardcoded logic**:

| Current hardcoding | Source set equivalent |
|--------------------|---------------------|
| `is_entry_point()` checking `.test.`, `.spec.`, `*Test.java`, `main.rs` | `role = "entry_point"` on test/benchmark/binary source sets |
| `[entry_points]` config section with patterns + annotations | Subsumed by `role = "entry_point"` + include patterns |
| Rust `#[cfg(test)]` items (currently not handled) | Parser tags items with source set; `scope.test.lint = false` excludes them |
| Java `src/test/java` vs `src/main/java` (currently no distinction) | Separate source sets with different `lint` and `role` settings |
| `--path-filter` / `--exclude-path` CLI workarounds | `scope.fixture.analysis = false` makes fixtures invisible by default |

**Implementation approach**:

1. Add `[scope]` section to `LintConfig` (with sane defaults when absent)
2. During `build_file_graph()`, classify each file into a source set based on
   path matching. Store the source set name on `FileInfo`.
3. `role = "entry_point"` replaces the hardcoded `is_entry_point()` function.
   Keep the hardcoded defaults as a built-in `[scope]` that applies when no
   config exists (backward compatible).
4. `lint = false` causes `evaluate_rules()` to skip edges where the source
   file is in a non-linted source set.
5. `analysis = false` causes analysis commands to exclude files in that source
   set from output (like an implicit `--exclude-path`).
6. The Rust parser detects `#[cfg(test)]` and tags imports/symbols. During
   graph construction, `#[cfg(test)]` items are reclassified into the `test`
   source set even though the file itself is in `production`.

Tasks:
- [ ] Define `ScopeConfig` struct with source set definitions
- [ ] Add `source_set: Option<String>` field to `FileInfo`
- [ ] Replace `is_entry_point()` with source set role lookup
- [ ] Add `lint: bool` and `analysis: bool` per source set
- [ ] Default source sets when no `[scope]` config exists (backward compat)
- [ ] Rust parser: detect `#[cfg(test)]` on mod/fn/impl blocks, tag with scope
- [ ] Java: auto-detect `src/test/java` as test source set when no config
- [ ] Add `--scope <name>` CLI flag to restrict analysis to a specific source set
- [ ] Add tests for each source set role

**Acceptance**: `statik dead-code` on statik itself with default scope config
excludes test fixtures. `statik lint` excludes `#[cfg(test)]` imports.
`statik dead-code --scope symbols` excludes test functions. All without any
CLI flags -- just config.

---

### 8.2 Structural edge propagation for `pub mod` re-exports ✓
**Complexity**: M
**Prerequisites**: 3b.5 (crate_name fix, is_mod_declaration flag)
**Files**: `src/analysis/dead_code.rs`, `src/cli/commands.rs`

**The problem**: 28 false dead exports in statik's own `mod.rs` barrel files.
`analysis/mod.rs` exports `cycles` via `pub mod cycles;`, consumed by other
modules via `analysis::cycles::detect_cycles()`. But nobody imports the name
`cycles` from `analysis/mod.rs` directly.

**The generic solution**: Edges already have `is_mod_declaration: bool`. Extend
this to a general concept of **edge kind** that affects how "used" status
propagates:

- `import` edges: standard data-flow. Imported names must match export names.
- `mod_declaration` edges: structural containment. If the target file has ANY
  importers, the mod re-export in the parent is used. This is analogous to
  how TS barrel files work: `export * from './child'` is used if anything
  in `child` is used.
- `re_export` edges (future): for `pub use` chains, propagate usage backward.

Tasks:
- [x] In dead code analysis, when checking if a `pub mod` export (name matches
  a module file stem) is used, check whether the target module file has any
  importers rather than checking if the name appears in an import statement
- [x] Generalize: for any re-export (TS `export * from`, Rust `pub mod`,
  Rust `pub use`), propagate "used" from the target back to the re-exporter
- [x] Add test: mod.rs re-exports child, external file imports from child,
  verify mod.rs export is marked used

**Acceptance**: 0 false dead exports from mod.rs barrel files on statik's own
codebase. Same logic works for TS barrel files with `export *`.

**Result**: Already implemented. The `detect_dead_code()` function builds a `mod_reexport_targets` map from mod_declaration edges and skips exports when the target module file is reachable (lines 212-256 of dead_code.rs). Added two unit tests: `test_pub_mod_reexport_not_dead_when_child_is_used` and `test_pub_mod_reexport_dead_when_parent_unreachable`. Verified 0 false dead exports on statik's own codebase.

---

### 8.3 Relative path output ✓
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/commands.rs`, `src/cli/output.rs`

All output uses absolute paths (`/Users/foo/project/src/main.rs`). This wastes
agent context window tokens and is hard to read. Output project-relative paths
(`src/main.rs`) by default.

Tasks:
- [x] Strip project root prefix from all paths in text, JSON, and CSV output
- [x] Add `--absolute-paths` flag to opt into full paths if needed
- [x] Ensure `--path-filter` glob matching works against relative paths
- [x] Update tests that assert on path values

**Acceptance**: All output uses project-relative paths by default.

---

### 8.4 Inline suppression comments ✅
**Complexity**: S
**Prerequisites**: 2.9 (lint command)
**Files**: `src/parser/suppression.rs`, `src/linting/rules.rs`, `src/parser/*.rs`,
`src/db/mod.rs`, `src/model/file_graph.rs`, `src/cli/graph_builder.rs`

Per-line suppression for known exceptions. Works alongside freeze/baseline
(project-level) and source sets (scope-level) as the most granular control.

```rust
// statik-ignore[model-is-leaf]
use crate::resolver::TypeScriptResolver;
```

Comment format is language-agnostic: `// statik-ignore[rule-id]` works in
Rust, TS/JS, and Java. The parser extracts these as metadata during parsing.

Tasks:
- [x] During parsing, extract `statik-ignore` comments and store as
  line -> rule_id mapping on the parse result
- [x] In lint evaluation, skip violations where source line has a matching
  suppression
- [x] `// statik-ignore` (no rule-id) suppresses all rules for that line
- [x] Report suppressed count in summary
- [x] Add tests for each language

**Implementation**: Shared extraction logic in `src/parser/suppression.rs`.
Suppressions stored in DB (`suppressions` table) and loaded into `FileInfo`
via `FileGraph`. Lint evaluator checks suppressions per-violation and tracks
a suppressed count in `LintSummary`.

**Acceptance**: `// statik-ignore[model-is-leaf]` above an import suppresses
that specific violation.

---

### 8.5 Java source root and multi-module support
**Complexity**: M
**Prerequisites**: 8.1 (source sets)
**Files**: `src/resolver/java.rs`, `src/linting/config.rs`

**The problem**: Java projects commonly have multiple source roots that the
auto-detection doesn't handle well:
- `src/main/java` + `src/test/java` (Maven standard)
- `src/main/java` + `src/integrationTest/java` (Gradle custom)
- Multi-module: `module-a/src/main/java` + `module-b/src/main/java`
- Generated sources: `build/generated/sources/annotationProcessor`

**The generic solution**: Source sets (8.1) already classify files. Extend the
Java resolver to use source set config for source root discovery instead of
only auto-detecting `src/main/java`:

```toml
[scope.production]
include = ["module-a/src/main/java/**", "module-b/src/main/java/**"]
source_roots = ["module-a/src/main/java", "module-b/src/main/java"]

[scope.test]
include = ["*/src/test/java/**", "*/src/integrationTest/java/**"]
source_roots = ["module-a/src/test/java", "module-b/src/test/java"]
role = "entry_point"
lint = false
```

The `source_roots` field tells the Java resolver where package hierarchies
start. This replaces the current heuristic of scanning for `src/main/java`.

Tasks:
- [ ] Add `source_roots: Vec<String>` to source set config
- [ ] Java resolver uses configured source roots when available, falls back
  to auto-detection when not
- [ ] Cross-module imports within the same project resolve correctly when
  both modules' source roots are configured
- [ ] Add test with multi-module Maven layout

**Acceptance**: `statik deps` on a multi-module Java project correctly resolves
imports across modules when source roots are configured.

---

### 8.6 Output duplication on stderr ✓
**Complexity**: S
**Prerequisites**: None
**Files**: `src/main.rs`

Several commands write output to both stdout and stderr.

Tasks:
- [x] Audit all println!/eprintln! calls in main.rs and commands.rs
- [x] Ensure analysis output goes to stdout only, errors to stderr only
- [x] Add test: capture stdout and stderr separately, verify no duplication

**Acceptance**: `statik lint 2>/dev/null` shows violations once on stdout.

**Result**: Audit found no duplication -- all `eprintln!` calls are correctly used for diagnostic messages (auto-index progress, parse errors, baseline info) and all analysis output goes to stdout via `println!` / `emit_output()`. Added two CLI tests to `tests/cli_tests.rs` to verify proper stream separation.

---

### 8.7 Remaining known bugs ✓
**Complexity**: S each
**Prerequisites**: None

- [x] **`--limit`/`--sort` with `--format text`**: Post-processing applies to JSON
  internally, then renders back to a simplified text format. Documented as
  acceptable for v1 -- text rendering with sort/limit produces reasonable output.
- [x] **NamingBoundary regex compiled per invocation**: Verified not a bug --
  regex is compiled once per rule evaluation, not per file. No fix needed.
- [x] **Baseline line-number sensitivity**: Baseline entries include `line`.
  Adding a blank line shifts the line and breaks suppression. Fixed: matching
  now uses `(rule_id, source_file, target_file)` only, ignoring line numbers.
  Line is kept in the baseline for informational purposes.
- [x] **Unused imports warning**: `src/linting/rules.rs:818` had 7 unused
  config type imports in the test module. Cleaned up.

---

## Phase 9: External Project Dogfooding

Findings from running statik against a large multi-module Java/TS monorepo
(~10K files, ~4300 Java, ~4600 JS, ~1400 TS). This exposed critical gaps
in how statik handles real-world project structures.

**Test project**: `tests/fixtures/java_monorepo/` — a synthetic multi-module
layout modeled after the real project, with fake names.

### 9.1 Java source root detection for multi-module repos (CRITICAL)
**Complexity**: M
**Prerequisites**: None
**Files**: `src/resolver/java.rs`, `src/linting/config.rs`

**The problem**: `detect_source_roots()` only checks `{project_root}/src/main/java`,
`{project_root}/src/test/java`, and `{project_root}/src`. For monorepos with nested
modules, the actual source roots are at paths like:
- `server/core/src/main/java/`
- `server/backend/src/java/`
- `engine/src/main/java/`
- `tools/codegen/src/`

Result: **86.6% of imports unresolved** (48,941 / 56,491). Only same-package
imports resolve (via `package_files` map), but cross-package imports fail
because `resolve_fqn()` tries `{wrong_root}/com/example/Foo.java`.

**The solution**: Two-pronged approach:

1. **Auto-detection**: Scan known Java files for all directories matching
   `**/src/main/java`, `**/src/test/java`, `**/src/java`, `**/src` that
   contain `.java` files. Use package declarations to verify — if a file at
   `server/core/src/main/java/com/example/Foo.java` declares
   `package com.example`, then `server/core/src/main/java` is a source root.

2. **Config-driven**: Add `[java]` section to `.statik/rules.toml`:
   ```toml
   [java]
   source_roots = [
     "server/core/src/main/java",
     "server/backend/src/java",
     "engine/src/main/java",
     "tools/codegen/src",
   ]
   ```
   Config takes precedence when present; auto-detection is the fallback.

Tasks:
- [x] Add `detect_all_source_roots()` that walks known files to find all
  `src/main/java` (etc.) directories, not just at the project root
- [x] Verify detected roots using package declarations from known files
- [x] Add `[java]` config section with `source_roots: Vec<String>`
- [x] Pass configured source roots to `JavaResolver::new()` when available
- [x] Add unit tests for multi-root detection
- [x] Add integration test using `tests/fixtures/java_monorepo/`

**Acceptance**: `statik deps` on a multi-module Java monorepo correctly resolves
cross-module imports. Unresolved import ratio drops from 86% to <5% for
intra-project imports.

---

### 9.2 `--lang` filter not applied to `dead-code`
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/commands.rs`

**The problem**: `statik dead-code --lang java` returns the same count as
`statik dead-code` without any filter. The `--lang` flag is ignored during
dead code analysis.

**Observed**: All three of `--lang java`, `--lang typescript`, `--lang javascript`
returned 8,888 dead files (the global total).

Tasks:
- [x] Trace the `--lang` filter path through `run_dead_code()` and identify
  where it's dropped
- [x] Apply language filter to dead file and dead export results before output
- [x] Add test: `dead-code --lang java` on a mixed project returns only Java files

**Acceptance**: `statik dead-code --lang java` returns only dead Java files.

---

### 9.3 `--count` flag returns exit code 1
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/commands.rs` or `src/main.rs`

**The problem**: `statik dead-code --count` prints the correct number but exits
with code 1. This breaks CI pipelines that check exit codes.

**Expected behavior**: Exit 0 when `--count` is used, since the purpose is to
query, not to assert. If the intent is "fail when count > 0", that should be
a separate flag (e.g., `--fail-on-findings`).

Tasks:
- [x] Audit exit code logic for `--count` across all commands
- [x] Exit 0 when `--count` is used (informational mode)
- [x] Add test: `dead-code --count` exits 0 even when dead code exists

**Acceptance**: `statik dead-code --count` exits 0.

---

### 9.4 `cycles` text format shows no cycle details
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/commands.rs` or `src/cli/output.rs`

**The problem**: Text output for `cycles` prints the summary header
(`cycle_count: 96, files_in_cycles: 407...`) then `cycles:` with nothing
after it. JSON format works correctly and shows all cycle details.

**Observed output**:
```
cycle_count: 96, files_in_cycles: 407, longest_cycle: 59, shortest_cycle: 2, total_files: 10391

cycles:
```

Tasks:
- [x] Debug the text formatter for cycles — likely a rendering bug where
  cycles are present in the data but the text formatter doesn't iterate them
- [x] Ensure text format shows cycle chains (A -> B -> C -> A) for each cycle
- [x] Add test: `cycles --format text` on a project with cycles shows file paths

**Acceptance**: `statik cycles` (text format) shows cycle chains, not just
the summary line.

---

### 9.5 Exclude `node_modules` and vendored dirs by default
**Complexity**: S
**Prerequisites**: None
**Files**: `src/discovery/mod.rs`

**The problem**: `node_modules/` directories inside the project get indexed
and analyzed. This produces noise: fake cycles in vendored dependencies (e.g.,
zod's internal circular imports), inflated file counts, and irrelevant dead
code reports.

The current default excludes for TS/JS don't cover `**/node_modules/**`
when the project root isn't a JS project itself (e.g., a Java monorepo with
a nested JS tool).

Tasks:
- [x] Add `**/node_modules/**` to the global default exclude list (not just
  TS-specific)
- [ ] Consider also excluding `**/vendor/**`, `**/third_party/**`,
  `**/external/**` by default
- [ ] Ensure `--include` can override defaults if someone explicitly wants
  to analyze vendored code
- [ ] Add test: node_modules directory is skipped even in a Java project

**Acceptance**: `statik index` on a monorepo with nested `node_modules/`
directories skips them by default.

---

### 9.6 Dead code massively over-reports when imports are unresolved ✓
**Complexity**: S (documentation / confidence fix)
**Prerequisites**: 9.1 (this is a symptom of unresolved imports)
**Files**: `src/analysis/dead_code.rs`

**The problem**: 8,888 / 10,391 files flagged dead (85.5%). This is a direct
cascade from 9.1 — unresolved imports mean files appear unreachable from
entry points.

The confidence system should flag this more clearly. When the unresolved import
ratio is very high (>50%), the dead code results should be downgraded to `low`
confidence or the summary should include a prominent warning.

Tasks:
- [x] When unresolved imports exceed 50% of total, add a warning to the
  summary output: "High unresolved import ratio (X%) — dead code results
  may be unreliable. Consider configuring source roots."
- [x] Downgrade confidence to `low` for all dead code results when the
  unresolved ratio exceeds 50%
- [x] Add the unresolved ratio to `summary` output for visibility

**Result**: Added `unresolved_ratio` field to `DeadCodeSummary`. When the
ratio exceeds 50%, a prominent limitation warning is emitted with the
percentage and a suggestion to configure source roots. The existing
`compute_confidence` function already handles confidence downgrade based on
the unresolved/total ratio.

**Acceptance**: Running statik on a project with high unresolved imports
shows a clear warning rather than silently reporting thousands of false
positives.

---

### 9.7 Dead file confidence incorrectly downgraded by outgoing imports
**Complexity**: S | **Status**: DONE
**Files**: `src/analysis/dead_code.rs`

Files with unresolved *outgoing* imports (e.g., `import java.util.HashMap`) were
marked `Medium` confidence. This was wrong — outgoing edges don't affect
reachability. Removed the per-file Medium branch; graph-level `High` already
handles the uncertainty correctly. Added 3 tests.

---

### 9.8 Java same-package references without import statements
**Complexity**: M | **Status**: MOSTLY DONE
**Files**: `src/parser/java.rs`, `src/resolver/java.rs`

In Java, same-package classes can be referenced without an import statement.
The parser only creates edges from `import` lines, so these references are
invisible. The `@type-ref:` mechanism handles some cases already.

**Progress**:
- [x] `@type-ref:` now emits synthetic imports for types used in method
  signatures, field declarations, generic bounds, casts, instanceof, and
  `new` expressions (already handled via `type_identifier` nodes)
- [x] `maybe_note_static_class_reference()` added to capture uppercase
  identifiers used as receivers of static method calls (`Helper.doSomething()`)
  and static field access (`Helper.CONSTANT`). These appear as `identifier`
  nodes (not `type_identifier`) in tree-sitter Java.
- [x] Wildcard-aware type-ref resolution: when a file has external wildcard
  imports (`import java.util.*`), unresolved type-refs are classified as
  `External` instead of `Unresolved`. Eliminates ~6,400 false unresolved
  imports on large projects.

**Dogfooding result** (large Java monorepo, ~4,400 Java files):
- Dead Java files dropped from 1,560 → 47 (with entry point config)
- All 10 same-package false positives eliminated after full re-index
- Remaining 47 are true positives (genuinely dead code)

**Known limitation**: Incremental indexing doesn't re-parse unchanged files,
so parser logic changes require a full re-index (`rm .statik/index.db`).
See 9.10 for the forced re-index TODO.

Tasks:
- [x] Audit `resolve_type_ref()` coverage for all Java reference patterns
- [x] Quantify impact: 10 false positives on dogfooding project, all fixed
- [x] Expand `@type-ref:` for method-body static calls and field access

---

### 9.9 `unresolved_imports` count conflates external and truly unresolved ✓
**Complexity**: S
**Files**: `src/cli/commands.rs`

The summary `unresolved_imports` includes both `External` (correctly classified
third-party libs) and `FileNotFound` (genuinely broken). This produces confusing
numbers where unresolved exceeds total. See `DOGFOOD_NOTES.md` for details.

Tasks:
- [x] Split count into `external_imports` and `unresolved_imports`
- [x] Only flag `FileNotFound` as a problem in the summary

**Result**: The summary output already had the split. The real fix was in `dead_code.rs` and `dependencies.rs` where confidence calculations and limitation warnings counted External imports as problems. Added `truly_unresolved_count()` and `external_import_count()` helpers to FileGraph. `has_unresolved_imports()` and `files_with_unresolved_imports()` now exclude External. Dead-code confidence improved from "low" to "high" on dogfooding project (249 external imports no longer penalize confidence).

---

### 9.10 Forced re-index when parser logic changes ✓
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/index.rs`, `src/db/mod.rs`

**The problem**: Incremental indexing skips files whose mtime hasn't changed.
When parser logic changes (e.g., new `@type-ref:` extraction rules), unchanged
files retain stale import data. Users must manually `rm .statik/index.db` to
get correct results after upgrading statik.

**The solution**: Two complementary approaches:

1. **`statik index --force`**: Delete and rebuild the index from scratch.
   Simple, explicit, user-initiated.

2. **Parser version stamp**: Store a hash of the parser version/config in the
   DB metadata. On `statik index`, compare the stored hash with the current
   parser version. If they differ, automatically trigger a full re-index.

Tasks:
- [x] Add `--force` flag to `statik index` that deletes `index.db` before
  re-indexing
- [x] Add `parser_version` metadata table to the DB
- [x] Compute a version hash from parser crate versions or a manually bumped
  constant
- [x] On index, compare stored hash with current — auto-reindex if different
- [x] Add test: changing parser version triggers full re-index

**Acceptance**: After upgrading statik with parser changes, `statik index`
automatically detects the version mismatch and re-parses all files.

---

### 9.11 Source set dependency visibility (module boundaries) ✓
**Complexity**: L
**Prerequisites**: 8.1 (source sets / scope config)
**Files**: `src/linting/config.rs`, `src/cli/commands.rs`, new `src/resolver/source_sets.rs`

**The problem**: Statik treats all source files as one flat namespace. Real
build systems (Gradle, Maven, Cargo workspaces, npm workspaces) have module
boundaries with scoped classpaths. This causes:
- False dependency edges across module boundaries
- Inflated cycle detection (e.g., 1000+ file mega-cycle caused by a single
  test file bridging two Gradle modules the JVM never connects)
- False same-package resolution when two modules have classes in the same
  Java package namespace

**Example**: A Gradle module `app/` depends on `framework` as a binary JAR
dependency. But statik indexes `framework`'s source tree alongside `app/src/`,
creating false edges. A `FooTest` in `app/src/test/` shares the same Java
package as `BarImpl` in `framework` source, creating a false framework→app
cycle.

**The solution**: Extend source sets (8.1) with a `deps` field that defines
which source sets can see each other:

```toml
[[source_sets]]
name = "framework"
roots = ["framework/src/main/java"]

[[source_sets]]
name = "framework-test"
roots = ["framework/src/test/java"]
deps = ["framework"]

[[source_sets]]
name = "app"
roots = ["app/src/main/java", "app/src/java"]
deps = ["framework"]

[[source_sets]]
name = "app-test"
roots = ["app/src/test/java"]
deps = ["app", "framework"]
```

During resolution, a file can only resolve to files in its own source set
or in source sets listed in `deps` (transitively). This prevents:
- `BarImpl` (framework) from resolving to `FooTest` (app-test) — framework
  doesn't depend on app-test
- Same-package resolution across module boundaries

**Design decisions**:
- Transitive deps for v1 (if A deps B deps C, A sees C). Could add
  `private_deps` later for Gradle `api` vs `implementation` distinction.
- Language-agnostic: works for Java modules, Rust workspace crates, npm
  packages — same concept of "these files can see those files"
- Opt-in: no `[[source_sets]]` = current behavior (single flat namespace)
- When source sets are defined, they subsume `[java] source_roots` — the
  roots within source sets serve the same purpose
- Files not in any source set go into an implicit "default" set that sees
  everything (backwards compat)

**Implementation approach (phased)**:

Phase A — Config + index:
- [x] Add `SourceSetConfig` struct: `{ name, roots, deps }`
- [x] Add `[[source_sets]]` deserialization to config loading
- [x] Build `SourceSetIndex` at graph-build time: map each file to its
  source set, compute transitive dependency closure
- [x] Provide `can_see(from_file, to_file) -> bool` query
- [x] Unit tests for `SourceSetConfig` parsing: valid configs, missing
  fields, unknown deps (should error), cyclic deps (should error)
- [x] Unit tests for `SourceSetIndex`: file-to-source-set mapping,
  transitive dep closure, `can_see` with direct deps / transitive deps /
  unrelated sets / default set

Phase B — Post-filter edges:
- [x] After resolving all edges in `build_file_graph()`, drop edges where
  `!source_set_index.can_see(from, to)`
- [x] This is the simplest integration — no resolver API changes needed
- [x] Unit test: edge between files in unrelated source sets is dropped
- [x] Unit test: edge between files in same source set is kept
- [x] Unit test: edge from dependent to dependency source set is kept
- [x] Unit test: edge from dependency to dependent source set is dropped
  (reverse direction)
- [x] Unit test: files not in any source set (default set) can see everything

Phase C — Scoped same-package resolution:
- [x] Java resolver's `package_files` map should be scoped per source set
  (or per visible-set-of-source-sets) to prevent false same-package matches
- [x] Unit test: two files with same Java package in different source sets
  don't resolve to each other unless deps allow it
- [x] Unit test: same-package resolution within a source set still works

Phase D — Cycle analysis scoping:
- [x] Report cycles within vs across source sets separately
- [x] A cross-source-set "cycle" that only exists because statik ignores
  module boundaries should be filtered out (or reported as a config issue)

Phase E — Integration tests and fixtures:
- [x] Add `tests/fixtures/java_source_sets/` fixture project with:
  - `framework/src/main/java/com/example/frame/` — core framework classes
  - `framework/src/test/java/com/example/frame/` — framework test classes
  - `app/src/main/java/com/example/app/` — application code that depends
    on framework
  - `app/src/test/java/com/example/app/` — application test code
  - Duplicate class: same FQN in `framework/src/test/` and `app/src/test/`
    to exercise the cross-module resolution scoping
  - `.statik/rules.toml` with `[[source_sets]]` config defining the four
    source sets and their deps
- [x] Integration test `test_source_set_visibility_filtering`: verify that
  deps from app → framework resolve, but framework cannot see app code
- [x] Integration test `test_source_set_prevents_cross_module_same_package`:
  verify that two files with same Java package in different source sets
  don't create a dependency edge when there's no dep relationship
- [x] Integration test `test_source_set_eliminates_false_cycles`: verify
  that a cycle that only exists due to cross-module leakage is eliminated
  when source sets are configured
- [x] Integration test `test_source_set_dead_code_scoping`: verify that
  dead code analysis respects source set visibility (a file in a leaf
  source set isn't falsely marked dead because nothing in the parent
  source set imports it, if it's used within its own source set)
- [x] Integration test `test_no_source_sets_backwards_compat`: verify that
  behavior is unchanged when no `[[source_sets]]` are configured (single
  flat namespace, all files see all files)

**Acceptance**: `statik cycles` on a multi-module project with source set
config eliminates false mega-cycles caused by cross-module leakage.
Cross-module false edges are eliminated. Same-package resolution is scoped
to visible source sets. All integration tests pass with the fixture project.

**Result**: Implemented in 5 phases across `src/resolver/source_sets.rs` (new),
`src/linting/config.rs`, `src/cli/graph_builder.rs`, `src/model/file_graph.rs`,
and `src/resolver/java.rs`. Source sets are configured via `[[source_sets]]` in
`rules.toml` with `name`, `roots`, and `deps` fields. `SourceSetIndex` builds
at graph time, validates for unknown/cyclic deps, computes transitive closure,
and provides `can_see(from, to)` queries. Edges are post-filtered in
`build_file_graph()`. Java resolver's `resolve_type_ref` scopes same-package
resolution to visible source sets (two-pass: prefer same set, then visible deps).
Cycle analysis inherits source set filtering via the filtered graph. Added 34+
new tests (unit + integration) including a `java_source_sets` fixture project.
Backwards compatible: no `[[source_sets]]` = original flat-namespace behavior.

---

### 9.12 `statik lint` crashes when rules.toml has no `[[rules]]` section ✓
**Complexity**: S
**Prerequisites**: None
**Files**: `src/linting/config.rs`

**The problem**: `load_config()` fails with a confusing TOML parse error when
`rules.toml` exists but has no `[[rules]]` section (e.g., only `[entry_points]`
or `[java]` config). Should gracefully return an empty rules vec instead.

Tasks:
- [x] Make the `rules` field in `LintConfig` default to an empty vec
  (`#[serde(default)]`)
- [x] Add test: `load_config` on a TOML with only `[entry_points]` succeeds
  with empty rules
- [x] `statik lint` with no rules should print "No lint rules configured"
  and exit 0

**Acceptance**: `statik lint` on a project with only `[entry_points]` config
prints a helpful message instead of crashing.

---

## What's Left: Strategic Priorities

### Completed (Phases 1-4, 2b, 3, 3b, 7, 8.2, 8.3, 8.6, 9.9, 9.10, 9.11, 9.12)
- Core TS/JS analysis with barrel files, dynamic imports, re-export tracing
- Java support with source root detection, wildcard imports, annotation entry points
- Rust support with crate_name resolution, mod-edge filtering, module-path imports
- Source set dependency visibility with module boundaries (9.11)
- 12 lint rule types with freeze/baseline
- Agent-friendly CLI: --path-filter, --count, --limit, --sort, --jq, CSV, --between
- Symbol-level dead code, references, callers commands
- Structural diff command
- Structural edge propagation for pub mod re-exports (8.2)
- Relative path output by default with --absolute-paths flag (8.3)
- Output duplication on stderr audit (8.6)
- Split unresolved_imports into external vs truly unresolved (9.9)
- Forced re-index with --force flag and parser version stamp (9.10)
- Graceful lint with no rules configured (9.12)

### Highest-impact next work
1. **Phase 8.4** (inline suppression): Completes the suppression trilogy
   (project baseline + source set scope + per-line ignore).
2. **Phase 9.6** (dead code confidence warning): User-facing warning when
   results are unreliable due to high unresolved import ratio.
3. **Phase 10.1-10.3** (human / committer analysis): The most differentiated
   new feature on the roadmap. No other CLI tool combines dependency-graph
   blast radius with committer history. `statik who <file>` answers "if I
   change this, who should I talk to?" — a question that currently requires
   tribal knowledge. Start with git history extraction (10.1), ownership
   model (10.2), then the impact-aware `who` command (10.3). The remaining
   items (bus-factor, churn, team coupling) build on the same data and can
   follow incrementally.
4. **Phase 1.4-1.5** (lazy loading + graph caching): Needed before targeting
   large projects (10K+ files).
5. **Phase 5** (refactoring intelligence): `statik diff HEAD~1 HEAD` is the
   killer feature for CI integration.
6. **Phase 6.2** (graph visualization): `statik graph --format dot` is
   low-effort, high-value for architecture reviews.
