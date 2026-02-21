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
- [ ] Define `TagDefinition`: `name: String`, `patterns: Vec<GlobPattern>`
- [ ] Define `TagRule`: `from_tag: String`, `to_tag: String`,
  `allow: bool`
- [ ] Implement evaluation: for each import edge, resolve source and target tags,
  check against tag rules
- [ ] A file may have multiple tags; evaluate all tag combinations
- [ ] Report violations with: tag names, specific files, import details
- [ ] Add tests: allowed inter-tag dependency, forbidden inter-tag dependency,
  multi-tag file

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

### 2b.1 Freeze / baseline mechanism
**Complexity**: M
**Prerequisites**: 2.9
**Files**: `src/linting/freeze.rs`, `src/cli/commands.rs`

Record current violations and only report *new* violations in subsequent runs.
This is the single biggest adoption enabler for architecture rules in existing
codebases. ArchUnit's `FreezingArchRule` is its most praised feature.

Tasks:
- [ ] Add `statik lint --freeze` command that saves current violations to
  `.statik/violations.store` (plain text, one violation per line)
- [ ] Add `statik lint --frozen` mode that compares against the store and only
  reports new violations not in the baseline
- [ ] Store format: `rule_id|source_path|target_path|imported_names` per line
- [ ] When a baseline violation is fixed, automatically remove it from the store
  on next `--freeze`
- [ ] Add `--update-baseline` flag to refresh the store after intentional changes
- [ ] Document workflow: first run creates baseline, CI uses `--frozen` to catch
  regressions

**Acceptance**: A team can add `statik lint` to an existing codebase with 50
violations, freeze the baseline, and CI only fails on *new* violations going
forward.

---

### 2b.2 Cycle size / scope policy
**Complexity**: S
**Prerequisites**: 2.9
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Configurable cycle detection in the lint framework. Current `statik cycles` is
all-or-nothing; this allows teams to gradually tighten cycle tolerance.

Tasks:
- [ ] Add `CyclePolicyRule` config variant with `max_cycle_size: usize` and
  `scope: file|directory`
- [ ] Implement evaluation: run SCC on file graph, report cycles exceeding
  `max_cycle_size`
- [ ] Optional `pattern` field to restrict check to specific paths
- [ ] Support `scope = "directory"` to check cycles at directory level (collapse
  intra-directory edges)
- [ ] Add tests: 2-file cycle passes with max_size=2, 3-file cycle fails

**Acceptance**: A rule `{max_cycle_size = 2, scope = "file"}` passes 2-file
mutual dependencies but flags 3+-file cycles as errors.

---

### 2b.3 Stability metric rule (Stable Dependencies Principle)
**Complexity**: S
**Prerequisites**: 2.7 (fan limit)
**Files**: `src/linting/rules.rs`, `src/analysis/metrics.rs`

Robert C. Martin's instability metric: `I = Ce / (Ca + Ce)` where Ce = fan-out,
Ca = fan-in. Dependencies should flow toward stability (lower I).

Tasks:
- [ ] Compute instability per file or per directory (configurable scope)
- [ ] Add `StabilityRule` config: `scope: file|directory`,
  `direction: toward-stability`
- [ ] Report violations where a file/dir depends on something more unstable
  than itself
- [ ] Optionally expose metrics in `statik summary` output
- [ ] Add tests with known-stable and known-unstable module arrangements

**Acceptance**: A leaf module (high fan-in, low fan-out, I≈0) depending on a
volatile module (low fan-in, high fan-out, I≈1) is NOT flagged. The reverse IS
flagged as a stable-dependencies-principle violation.

---

### 2b.4 Naming convention boundary rules
**Complexity**: S
**Prerequisites**: 2.2, 2.3
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Complement path-based boundaries with file-name-based boundaries. Common in
codebases without strict directory layering.

Tasks:
- [ ] Add `NamingBoundaryRule` config: `from_name: Vec<GlobPattern>`,
  `deny_name: Vec<GlobPattern>`, `except_name: Option<Vec<GlobPattern>>`
- [ ] Match against file names (not full paths) for the name patterns
- [ ] Implement evaluation: for each edge, check if source filename matches
  `from_name` and target filename matches `deny_name`
- [ ] Add tests: `*Controller.ts must not import *Repository.ts`

**Acceptance**: A rule `{from_name = "*Controller.*", deny_name = "*Repository.*"}`
reports when `UserController.ts` directly imports `UserRepository.ts`.

---

### 2b.5 Restricted consumer rules ("only X may use Y")
**Complexity**: S
**Prerequisites**: 2.3
**Files**: `src/linting/config.rs`, `src/linting/rules.rs`

Inverse of boundary rules: instead of "A must not import B", enforce "only A may
import B". Useful for cross-cutting concerns like logging, metrics, DB access.

Tasks:
- [ ] Add `RestrictedConsumerRule` config: `target: Vec<GlobPattern>`,
  `allowed_from: Vec<GlobPattern>`
- [ ] Implement evaluation: for each edge to a matching target, check if the
  source matches `allowed_from`. If not, it's a violation.
- [ ] Add tests: `{target = "src/db/**", allowed_from = ["src/cli/**"]}` flags
  when `src/analysis/foo.rs` imports from `src/db/`

**Acceptance**: A rule restricting DB access to CLI-only correctly flags when
analysis or parser modules import DB types.

---

### 2b.6 Max exports / API surface limit
**Complexity**: S
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`

Enforce that modules don't expose too many symbols. Barrel files with 50+
re-exports are a common antipattern.

Tasks:
- [ ] Add `ExportLimitRule` config: `pattern: Vec<GlobPattern>`,
  `max_exports: usize`
- [ ] Implement evaluation: count exports per file from the DB, flag files
  exceeding the limit
- [ ] Add tests: file with 5 exports passes at limit 10, file with 15 fails

**Acceptance**: A rule `{pattern = "src/**", max_exports = 15}` flags barrel
files with excessive re-exports.

---

### 2b.7 Dependency weight / coupling detection
**Complexity**: M
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`

Flag heavy coupling between two files: not just "does A depend on B" but "how
many distinct symbols does A import from B." NDepend measures this as coupling
weight.

Tasks:
- [ ] Add `CouplingWeightRule` config: `pattern: Vec<GlobPattern>`,
  `max_imports_per_edge: usize`
- [ ] Implement evaluation: for each edge, count imported symbols. Flag edges
  exceeding the threshold.
- [ ] Report: the two files, current import count, threshold
- [ ] Add tests: edge with 3 imports passes at limit 10, edge with 12 fails

**Acceptance**: A rule `{max_imports_per_edge = 10}` flags file pairs where one
imports 10+ symbols from the other, suggesting they should be merged or a shared
abstraction extracted.

---

### 2b.8 Directory cohesion rule
**Complexity**: M
**Prerequisites**: 2.2
**Files**: `src/linting/rules.rs`, `src/analysis/metrics.rs`

Files in the same directory should depend more on each other than on outside
files. Low cohesion suggests files are misplaced.

Tasks:
- [ ] Add `CohesionRule` config: `scope: directory`, `min_internal_ratio: f64`,
  `pattern: Vec<GlobPattern>`, `min_files: usize`
- [ ] Compute per-directory: internal deps / total deps ratio
- [ ] Flag directories below the threshold (e.g., <30% internal deps)
- [ ] Only check directories with `min_files` or more files
- [ ] Add tests with high-cohesion and low-cohesion directory arrangements

**Acceptance**: A directory where 80% of dependencies are internal passes at
threshold 0.3. A directory where 10% of dependencies are internal is flagged.

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

### 3b.5 Dogfooding findings (HIGH PRIORITY)

Issues discovered by running statik on its own codebase:

- [ ] **`use crate_name::` paths not resolved**: `main.rs` does `use statik::cli::commands`
  — importing via the crate name. The resolver only handles `crate::`, `super::`, `self::`
  prefixes. It doesn't know that `statik` = `crate`. This causes most cross-module imports
  to be unresolved (276/616 unresolved) and cascading false-positive dead exports (592).
  **Fix**: Read crate name from `Cargo.toml` `[package].name`, treat `use crate_name::` as
  equivalent to `use crate::`. Complexity: M.
- [ ] **`pub mod` re-export chains not followed**: `cli/mod.rs` does `pub mod commands;`
  making items available via `cli::commands`. But consumers import via `use statik::cli::commands`,
  which hits the crate_name gap above. Fixing crate_name resolution fixes this transitively.
- [ ] **False cycles from `mod.rs` barrel files**: The `graph.rs <-> file_graph.rs <-> mod.rs`
  cycle is caused by `mod.rs` re-exporting both modules. This is standard Rust module
  structure, not a real dependency cycle. **Fix**: Filter cycles where all edges are `@mod:`
  structural containment edges, or distinguish `mod` edges from `use` edges in cycle
  detection. Complexity: S.
- [ ] **Dead export false-positive cascade**: `commands.rs` exports 13 `pub fn`s, all
  consumed by `main.rs` via `commands::run_deps()` etc. But since `main.rs -> commands.rs`
  edge is missing (crate_name gap), all 13 show as unused. Fixing crate_name resolution
  fixes this transitively.

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
- [ ] Extract `extends` and `implements` relationships as references with
  `RefKind::Inheritance`
- [ ] Extract annotation usage (`@Override`, `@Autowired`, etc.) as references
  with `RefKind::TypeUsage`
- [ ] Add `statik deps --direction in` support for Java inheritance (what extends
  this class?)
- [ ] Add tests for Java inheritance hierarchies

**Acceptance**: `statik impact UserService.java` includes files that extend
`UserService` in the affected list.

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

### 7.1 Output path filtering (`--path`)
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

The single highest-impact CLI addition. Agents cannot currently say "show me
dead code in `src/model/` only" without post-processing.

Tasks:
- [ ] Add `--path <glob>` global flag to `Cli` struct
- [ ] Apply as output filter: only include results where file path matches the
  glob pattern
- [ ] Works with all commands: dead-code, deps, cycles, exports, lint, impact,
  symbols, references, callers
- [ ] Composable with `--lang` (both filters apply)
- [ ] Add tests: `statik dead-code --path "src/model/"` only shows model files

**Acceptance**: `statik dead-code --path "src/model/**"` returns only dead code
in the model directory, without requiring any post-processing.

---

### 7.2 Count mode (`--count`)
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Just print the count of results. Eliminates `| jq '. | length'` and `| wc -l`.

Tasks:
- [ ] Add `--count` global flag to `Cli` struct
- [ ] When set, replace normal output with a single number
- [ ] Works with all commands that produce lists
- [ ] Composable with `--path` and `--lang`
- [ ] Exit code 0 if count == 0, 1 if count > 0 (useful for CI checks)

**Acceptance**: `statik dead-code --count` prints `42`. Combined:
`statik dead-code --count --path "src/model/"` prints `7`.

---

### 7.3 Result limiting (`--limit`)
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Limit output to first N results. Reduces context window waste for agents.

Tasks:
- [ ] Add `--limit <N>` global flag to `Cli` struct
- [ ] Truncate result list before serialization
- [ ] Works with all commands that produce lists
- [ ] Composable with `--path`, `--lang`, `--sort`
- [ ] Add a "... and N more" indicator in text output when truncated

**Acceptance**: `statik dead-code --limit 10` shows only the first 10 dead
code results.

---

### 7.4 Sort control (`--sort`)
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `src/cli/commands.rs`

Deterministic, meaningful ordering. Agents benefit from seeing worst offenders
first.

Tasks:
- [ ] Add `--sort <field>` global flag: `path`, `confidence`, `name`, `depth`
- [ ] Default: `path` (alphabetical, deterministic)
- [ ] `confidence`: high-confidence results first
- [ ] `depth`: for impact command, deepest impact first
- [ ] Add `--reverse` flag for descending order
- [ ] Add tests for each sort field

**Acceptance**: `statik dead-code --sort confidence` shows high-confidence dead
code first.

---

### 7.5 Built-in jq filtering (`--jq`)
**Complexity**: M
**Prerequisites**: None
**Files**: `src/cli/mod.rs`, `Cargo.toml`

Like GitHub CLI's `--jq` flag. Eliminates all post-processing for agents.
Use the `jaq-core` crate (pure Rust jq implementation).

Tasks:
- [ ] Add `jaq-core` and `jaq-std` dependencies
- [ ] Add `--jq <expression>` global flag
- [ ] Implicitly sets `--format json`
- [ ] Apply jq filter to the JSON output before printing
- [ ] Add tests: `--jq '.[].path'`, `--jq '[.[] | select(.confidence == "high")]'`
- [ ] Handle jq errors gracefully (print error message, exit 1)

**Acceptance**: `statik dead-code --jq '.[].path'` prints one file path per
line without requiring external jq.

---

### 7.6 Richer JSON schema
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/output.rs`, `src/cli/commands.rs`

Add structured path components to JSON output so agents don't need to parse
file paths.

Tasks:
- [ ] Add `directory`, `filename`, `extension` fields alongside `path` in all
  JSON output
- [ ] Add `module` field (top-level source directory: model, parser, cli, etc.)
- [ ] Add `language` field to all file-level results
- [ ] Ensure backward compatibility (new fields are additive)
- [ ] Add tests verifying JSON schema

**Acceptance**: JSON output for a dead export includes
`{"path": "src/model/foo.rs", "directory": "src/model", "filename": "foo.rs",
"extension": "rs", "module": "model", "language": "rust", ...}`.

---

### 7.7 Cross-module edge filter (`--between`)
**Complexity**: S
**Prerequisites**: 7.1
**Files**: `src/cli/commands.rs`

Show only edges between two path scopes. Very useful for understanding
cross-module coupling.

Tasks:
- [ ] Add `--between <from_glob> <to_glob>` flag to `deps` command
- [ ] Filter edge list: only include edges where source matches `from_glob`
  and target matches `to_glob`
- [ ] Works with `--format json`, `--count`, `--limit`
- [ ] Add tests: `statik deps --between src/parser/ src/model/`

**Acceptance**: `statik deps --between "src/parser/**" "src/model/**"` shows
only edges from parser files to model files.

---

### 7.8 CSV output format
**Complexity**: S
**Prerequisites**: None
**Files**: `src/cli/output.rs`

Trivially parseable without any JSON library. Useful for agents that want
simple text processing.

Tasks:
- [ ] Add `csv` to the `OutputFormat` enum
- [ ] Implement CSV serialization for all commands (header row + data rows)
- [ ] Use comma separator, quote fields containing commas
- [ ] Add tests for each command's CSV output

**Acceptance**: `statik dead-code --format csv` produces parseable CSV with
headers `path,name,kind,confidence`.

---

### 7.9 Directory-level summary aggregation
**Complexity**: M
**Prerequisites**: None
**Files**: `src/cli/commands.rs`

Roll up statistics per directory for high-level architectural overview.

Tasks:
- [ ] Add `--by-directory` flag to `summary` command
- [ ] Aggregate: files, exports, dead exports, avg fan-out, avg fan-in per
  directory
- [ ] Works with `--format json`, `--sort`, `--limit`
- [ ] Add tests with known directory structures

**Acceptance**: `statik summary --by-directory --format json` produces
per-directory rollup showing file counts, export counts, dead export counts,
and coupling metrics.

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
| **Total** | **20** | **25** | **11** | **2** | **60** |

**Priority guidance**: Phase 2b (advanced lint rules) and Phase 7 (agent-friendly
CLI) are the highest-impact next increments. Within Phase 2b, the freeze/baseline
mechanism (2b.1) is the adoption enabler. Within Phase 7, the `--path`, `--count`,
and `--limit` flags (7.1-7.3) are quick wins that eliminate most agent post-processing.
The Rust dogfooding findings (3b.5) — especially crate_name resolution — should be
addressed in parallel as they affect the tool's credibility for Rust projects.
