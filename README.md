# statik

Static code analysis for dependency graphs, dead code detection, and architectural linting in TypeScript/JavaScript, Java, and Rust projects. Makes your codebase's dependency structure queryable -- by developers and AI coding agents alike.

statik fills a gap between simple text search and full Language Server Protocol (LSP) features. Where LSP gives you go-to-definition and find-references for individual symbols, statik provides **graph-level analysis**: dependency chains between files, dead code detection, circular dependency detection, and refactoring blast radius. AI agents benefit from the same analysis -- they are permanent newcomers to a codebase, and statik gives them the architectural context that would otherwise require reading thousands of files. These are complementary capabilities -- statik does not replace LSP.

## Quick Start

### Build from source

```
git clone <repo-url>
cd statik
cargo build --release
```

The binary is at `target/release/statik`.

### Index a project

```
statik index /path/to/your/typescript-project
```

This scans all supported source files (TypeScript, JavaScript, Java, Rust), extracts symbols and import/export relationships, and stores the result in `.statik/index.db` at the project root.

```
Indexed 87 files: 1423 symbols, 312 references (245ms)
```

### Auto-indexing

Analysis commands automatically create the index if `.statik/index.db` does not exist. You can skip this with `--no-index` to require an existing index. To update a stale index, re-run `statik index`.

### Find dead code

```
statik dead-code
```

### Check for circular dependencies

```
statik cycles
```

### See what depends on a file

```
statik impact src/utils/helpers.ts
```

### Check architectural rules

```
statik lint
```

### Get a project overview

```
statik summary --format json
```

### Enrich with compiler-resolved data (optional)

```
statik enrich java-index.scip
```

If you have a SCIP index from `scip-java`, `scip-typescript`, `rust-analyzer`, or `scip-clang`, import it to upgrade analysis precision. See [`statik enrich`](#statik-enrich-scip-file) for details.

## Commands

### `statik index [path]`

Build or update the symbol index for the project at `path` (default: current directory). Creates `.statik/index.db`.

Re-running `index` only re-parses files whose modification time changed. Deleted files are automatically removed.

```
statik index .
statik index /path/to/project --format json
statik index --with-history                          # also index git commit history
statik index --with-history --history-depth 5000     # limit to last 5000 commits
```

| Flag | Description |
|------|-------------|
| `--force` | Force full re-index (ignore cached data) |
| `--with-history` | Also index git commit history (authors, file changes). Required for `owners`, `bus-factor`, and `churn` commands |
| `--history-depth <N>` | Limit history to the last N commits |

### `statik deps <path>`

File-level dependency analysis. Shows what a file imports and what imports it.

```
statik deps src/utils/helpers.ts
statik deps src/utils/helpers.ts --direction out          # only show imports
statik deps src/utils/helpers.ts --direction in           # only show importers
statik deps src/utils/helpers.ts --transitive             # follow chains
statik deps src/utils/helpers.ts --transitive --max-depth 3
statik deps src/utils/helpers.ts --runtime-only           # exclude type-only imports
statik deps --between "src/parser/**" "src/model/**"      # cross-module edges
```

| Flag | Description |
|------|-------------|
| `--transitive` | Follow dependency chains transitively |
| `--direction in\|out\|both` | Direction of analysis (default: `both`) |
| `--max-depth <N>` | Limit transitive depth |
| `--runtime-only` | Exclude type-only imports from results |
| `--between <from_glob> <to_glob>` | Show only edges where source matches `from_glob` and target matches `to_glob` |

### `statik exports <path>`

List all exports from a file with used/unused status. Shows which exports are imported by other files and which are not.

```
statik exports src/components/index.ts
statik exports src/utils/math.ts --format json
```

### `statik dead-code`

Find dead code: orphaned files (never imported from any entry point), unused exports (exported symbols never imported anywhere), and unused symbols (internal symbols with no references).

```
statik dead-code
statik dead-code --scope files       # only orphaned files
statik dead-code --scope exports     # only unused exports
statik dead-code --scope symbols     # unused internal symbols (no references)
statik dead-code --scope both        # files + exports (default)
statik dead-code --runtime-only      # ignore type-only imports
```

| Flag | Description |
|------|-------------|
| `--scope files\|exports\|symbols\|both` | What to check for (default: `both`) |
| `--runtime-only` | Exclude type-only imports from analysis |

The `symbols` scope performs symbol-level dead code detection: it finds non-exported symbols that have no intra-project references. This is more granular than file-level or export-level analysis.

Entry points are never reported as dead. Entry points are detected automatically: files named `index`, `main`, `app`, `server`, `cli`, and test files (`*.test.*`, `*.spec.*`, `*_test.*`, `*_spec.*`). For Java, entry points are detected by file name conventions (JUnit test files `*Test.java`, `*Tests.java`, `*IT.java`, `Test*.java` and Spring Boot `Application.java`) and by annotation-based detection (`@SpringBootApplication`, `@Test`, `@ParameterizedTest`, `@RepeatedTest`, `@Component`, `@Service`, `@Repository`, `@Controller`, `@RestController`, `@Configuration`, `@Bean`, `@Endpoint`, `@WebServlet`). For Rust, entry points include `lib.rs`, `main.rs`, files in `src/bin/`, `tests/`, `examples/`, `benches/`, and `build.rs`. Additionally, files in source sets with `role = "entry_point"` (configured via the `[scope]` section) are treated as entry points.

### `statik cycles`

Detect circular dependencies in the file-level import graph. Reports cycles ordered by length (shortest first, most actionable).

```
statik cycles
statik cycles --format json
statik cycles --runtime-only         # ignore type-only imports
```

| Flag | Description |
|------|-------------|
| `--runtime-only` | Exclude type-only imports from cycle detection |

### `statik impact <path>`

Blast radius analysis: if this file changes, what other files are affected? Performs reverse traversal of the dependency graph to find all direct and transitive dependents.

```
statik impact src/models/user.ts
statik impact src/models/user.ts --max-depth 2
statik impact src/models/user.ts --runtime-only
```

| Flag | Description |
|------|-------------|
| `--max-depth <N>` | Limit how far to follow the dependency chain |
| `--runtime-only` | Exclude type-only imports from impact analysis |

### `statik summary`

Project overview: file counts by language, dependency statistics, dead code count, circular dependency count. Designed to fit in a single LLM context message.

```
statik summary
statik summary --format json
statik summary --by-directory                      # aggregate stats per directory
statik summary --by-directory --format json
```

| Flag | Description |
|------|-------------|
| `--by-directory` | Aggregate statistics per directory (file counts, exports, dead exports, coupling metrics) |

### `statik lint`

Check architectural rules defined in a config file. Reports violations of boundary rules, layer hierarchies, module containment, import restrictions, and fan-in/fan-out limits.

```
statik lint
statik lint --config path/to/rules.toml
statik lint --rule no-ui-to-db                     # evaluate a single rule
statik lint --severity-threshold warning            # only report warnings and errors
statik lint --freeze                                # save current violations as baseline
statik lint --update-baseline                       # refresh baseline after intentional changes
statik lint --format json
```

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to config file (default: `.statik/rules.toml` or `statik.toml`) |
| `--rule <id>` | Only evaluate a specific rule by ID |
| `--severity-threshold error\|warning\|info` | Minimum severity to report (default: `info`) |
| `--freeze` | Save current violations as the baseline (suppresses them in future runs) |
| `--update-baseline` | Refresh the baseline with current violations (alias for `--freeze`) |

The lint command exits with code 1 if any errors are found, and code 0 otherwise (even if warnings are present).

When a baseline exists (`.statik/lint-baseline.json`), only violations not present in the baseline are reported. This allows teams to adopt architectural rules gradually: freeze existing violations, then enforce that no new violations are introduced.

#### Configuration

Create `.statik/rules.toml` (or `statik.toml` in the project root) to define lint rules. Every rule shares these common fields:

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Unique rule identifier |
| `severity` | yes | `error`, `warning`, or `info` |
| `description` | yes | Human-readable description of the rule |
| `rationale` | no | Why this rule exists (included in JSON output) |
| `fix_direction` | no | Suggested fix direction (included in output) |

Each rule also has a type-specific section (`[rules.boundary]`, `[rules.layer]`, etc.) that determines what it checks.

#### Supported Rules

| Rule type | Config key | Purpose |
|-----------|------------|---------|
| Boundary | `[rules.boundary]` | Block imports from one set of files to another |
| Layer hierarchy | `[rules.layer]` | Enforce top-down dependency flow through ordered layers |
| Module containment | `[rules.containment]` | Require external access through a public API file |
| Import restriction | `[rules.import_restriction]` | Enforce type-only imports, forbidden/allowed names |
| Fan-in/fan-out limit | `[rules.fan_limit]` | Detect architectural hotspots by capping dependency counts |
| Cycle policy | `[rules.cycle_policy]` | Configurable cycle detection with size limits |
| Stability limit | `[rules.stability_limit]` | Enforce the Stable Dependencies Principle (instability metric) |
| Naming boundary | `[rules.naming_boundary]` | Enforce file naming conventions within path patterns |
| Restricted consumer | `[rules.restricted_consumer]` | Allow only specific files to import from a target |
| Export limit | `[rules.export_limit]` | Cap the number of exports per file |
| Coupling weight | `[rules.coupling_weight]` | Limit how many symbols one file imports from another |
| Directory cohesion | `[rules.cohesion]` | Ensure files in a directory depend more internally than externally |
| Tag boundary | `[rules.tag_boundary]` | Define tagged file groups and control dependencies between tags |

#### Boundary rules

Block imports between file sets. Use when you need to prevent a specific group of files from importing another group.

```toml
[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI layer must not import from database layer"
rationale = "The UI should go through the service layer"
fix_direction = "Import from src/services/ instead"

[rules.boundary]
from = ["src/ui/**", "src/components/**"]
deny = ["src/db/**"]
except = ["src/db/types.ts"]
```

| Field | Required | Description |
|-------|----------|-------------|
| `from` | yes | Glob patterns for source files |
| `deny` | yes | Glob patterns for forbidden import targets |
| `except` | no | Glob patterns for exceptions to the deny list |

#### Layer hierarchy rules

Enforce top-down dependency flow through an ordered list of layers. A layer can import from layers below it in the list, but not above. Use this to enforce clean architecture or layered patterns across an entire project.

```toml
[[rules]]
id = "clean-layers"
severity = "error"
description = "Dependencies must flow top-down through layers"
rationale = "Enforces clean architecture: presentation -> service -> data"

[rules.layer]
layers = [
  { name = "presentation", patterns = ["src/ui/**"] },
  { name = "service", patterns = ["src/services/**"] },
  { name = "data", patterns = ["src/db/**"] },
]
```

Layers are ordered top-to-bottom. In this example, `presentation` can import from `service` and `data`, `service` can import from `data`, but `data` cannot import from `service` or `presentation`.

| Field | Required | Description |
|-------|----------|-------------|
| `layers` | yes | Ordered list of `{ name, patterns }` objects |
| `layers[].name` | yes | Human-readable layer name (used in violation messages) |
| `layers[].patterns` | yes | Glob patterns matching files in this layer |

#### Module containment rules

Enforce that files inside a module are only imported through designated public API files. Use this when a module should expose a limited surface area (e.g., through an `index.ts` barrel file) and internal files should not be imported directly by outsiders.

```toml
[[rules]]
id = "auth-encapsulation"
severity = "warning"
description = "Auth module must be accessed through its public API"
fix_direction = "Import from src/auth/index.ts instead"

[rules.containment]
module = ["src/auth/**"]
public_api = ["src/auth/index.ts"]
```

Files inside the module can import each other freely. Only imports from outside the module are checked.

| Field | Required | Description |
|-------|----------|-------------|
| `module` | yes | Glob patterns defining the module boundary |
| `public_api` | yes | Glob patterns for files that outsiders are allowed to import |

#### Import restriction rules

Restrict how files matching a target pattern are imported. Supports type-only enforcement and forbidden/allowed import name lists.

```toml
[[rules]]
id = "models-type-only"
severity = "info"
description = "Imports from models/ should be type-only when possible"

[rules.import_restriction]
target = ["src/models/**"]
require_type_only = true
```

```toml
[[rules]]
id = "no-internals"
severity = "error"
description = "Cannot import internal functions from the internal module"

[rules.import_restriction]
target = ["src/internal/**"]
forbidden_names = ["getSecret", "internalHelper"]
```

| Field | Required | Description |
|-------|----------|-------------|
| `target` | yes | Glob patterns for the import target files to restrict |
| `require_type_only` | no | If `true`, all imports from target must use `import type` (default: `false`) |
| `forbidden_names` | no | List of symbol names that cannot be imported from target |
| `allowed_names` | no | If set, only these symbol names can be imported from target |

#### Fan-in/fan-out limit rules

Detect architectural hotspots by capping how many files a single file can depend on (fan-out) or how many files can depend on it (fan-in). Use this to prevent god modules and identify files that may need refactoring.

```toml
[[rules]]
id = "no-god-modules"
severity = "warning"
description = "Files should not have too many dependencies"
fix_direction = "Split this file into smaller, focused modules"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_out = 10
```

You can set `max_fan_in`, `max_fan_out`, or both:

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns for files to check |
| `max_fan_in` | no | Maximum number of files that may import this file |
| `max_fan_out` | no | Maximum number of files this file may import |

#### Cycle policy rules

Configurable cycle detection in the lint framework. Allows teams to gradually tighten cycle tolerance by setting a maximum allowed cycle length, optionally scoped to specific paths.

```toml
[[rules]]
id = "no-large-cycles"
severity = "error"
description = "No cycles longer than 2 files"

[rules.cycle_policy]
max_cycle_length = 2
pattern = ["src/**"]
```

| Field | Required | Description |
|-------|----------|-------------|
| `max_cycle_length` | yes | Maximum allowed cycle length (0 = no cycles allowed) |
| `pattern` | no | Glob patterns to restrict which files are checked |

#### Stability limit rules

Enforce Robert C. Martin's Stable Dependencies Principle. The instability metric `I = fan-out / (fan-in + fan-out)` measures how volatile a file is. Files with high instability (close to 1.0) have many outgoing dependencies but few incoming ones.

```toml
[[rules]]
id = "stable-model"
severity = "warning"
description = "Model layer must be stable"

[rules.stability_limit]
pattern = ["src/model/**"]
max_instability = 0.3
```

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns for files to check |
| `max_instability` | yes | Maximum allowed instability (0.0 = perfectly stable, 1.0 = maximally unstable) |

#### Naming boundary rules

Enforce file naming conventions within specific directories. Uses regex patterns matched against the full relative file path.

```toml
[[rules]]
id = "service-naming"
severity = "warning"
description = "Services must follow naming convention"

[rules.naming_boundary]
pattern = ["src/services/**"]
must_match = ".*Service\\.(ts|rs)$"
```

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns selecting which files to check |
| `must_match` | yes | Regex pattern that file paths must match |

#### Restricted consumer rules

Inverse of boundary rules: instead of "A must not import B", enforce "only A may import B". Useful for cross-cutting concerns like logging, metrics, or database access.

```toml
[[rules]]
id = "db-restricted"
severity = "error"
description = "Only CLI layer may access the database"

[rules.restricted_consumer]
target = ["src/db/**"]
allowed_consumers = ["src/cli/**"]
```

| Field | Required | Description |
|-------|----------|-------------|
| `target` | yes | Glob patterns for the import target to restrict |
| `allowed_consumers` | yes | Glob patterns for files allowed to import from target |

#### Export limit rules

Cap the number of exports per file. Useful for preventing barrel files with excessive re-exports or god modules that expose too many symbols.

```toml
[[rules]]
id = "api-surface-limit"
severity = "warning"
description = "Files should not export too many symbols"

[rules.export_limit]
pattern = ["src/**"]
max_exports = 15
```

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns for files to check |
| `max_exports` | yes | Maximum number of exports allowed per file |

#### Coupling weight rules

Detect heavy coupling between files by limiting how many distinct symbols one file imports from another. High coupling suggests the files should be merged or a shared abstraction extracted.

```toml
[[rules]]
id = "coupling-limit"
severity = "warning"
description = "Too many symbols imported from a single file"
fix_direction = "Consider merging files or extracting shared types"

[rules.coupling_weight]
pattern = ["src/**"]
max_names_per_edge = 10
```

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns for source files to check |
| `max_names_per_edge` | yes | Maximum number of imported symbols per import edge |

#### Directory cohesion rules

Ensure files within a directory depend more on each other than on outside files. Low cohesion suggests files may be misplaced.

```toml
[[rules]]
id = "module-cohesion"
severity = "info"
description = "Modules should have high internal cohesion"

[rules.cohesion]
pattern = ["src/**"]
max_external_ratio = 0.7
```

| Field | Required | Description |
|-------|----------|-------------|
| `pattern` | yes | Glob patterns for files to include in cohesion analysis |
| `max_external_ratio` | yes | Maximum ratio of external to total dependencies (0.0 = all internal, 1.0 = all external) |

#### Tag boundary rules

The most flexible rule type. Define named tags with glob patterns, then control which tags may depend on which. Tags are defined in a top-level `[tags]` section and referenced by tag boundary rules.

```toml
[tags]
api = ["src/api/**"]
internal = ["src/internal/**"]
shared = ["src/shared/**"]

[[rules]]
id = "no-api-to-internal"
severity = "error"
description = "API must not access internal modules"

[rules.tag_boundary]
from_tag = "api"
deny_tags = ["internal"]
except_tags = ["shared"]
```

| Field | Required | Description |
|-------|----------|-------------|
| `from_tag` | yes | Tag name for source files |
| `deny_tags` | yes | List of tag names that source files must not depend on |
| `except_tags` | no | List of tag names that are exceptions to the deny list |

Tags are defined in the config's `[tags]` section as a map of tag name to glob patterns. A file may match multiple tags. Unknown tag names in rules are silently skipped.

#### Freeze / baseline mechanism

The freeze mechanism allows teams to adopt architectural rules in existing codebases without fixing all violations upfront. Run `statik lint --freeze` to save current violations as a baseline. Subsequent runs only report new violations.

```
# First time: create baseline from current violations
statik lint --freeze

# CI/daily runs: only report NEW violations
statik lint

# After intentionally fixing or introducing violations, refresh the baseline
statik lint --update-baseline
```

The baseline is stored at `.statik/lint-baseline.json`. Add it to version control so the team shares the same baseline. Baseline matching uses `(rule_id, source_file, target_file)` only -- line numbers are stored for reference but not used for matching, so adding or removing blank lines does not break suppression.

#### Inline suppression comments

For per-line exceptions, add a `statik-ignore` comment above the import:

```rust
// statik-ignore[model-is-leaf]
use crate::resolver::TypeScriptResolver;
```

The comment format is language-agnostic and works in Rust, TypeScript/JavaScript, and Java:

```typescript
// statik-ignore[no-ui-to-db]
import { query } from '../db/connection';
```

```java
// statik-ignore[layer-violation]
import com.example.internal.Helper;
```

- `// statik-ignore[rule-id]` suppresses a specific rule for the next line
- `// statik-ignore` (no brackets) suppresses all rules for the next line
- `/* statik-ignore[rule-id] */` block comment variant is also supported
- Suppressed violations are counted and shown in the lint summary

Suppression granularity levels from broadest to narrowest:

1. **Source sets** (`[scope]` config) -- scope-level: exclude entire categories of code from linting
2. **Freeze / baseline** (`--freeze`) -- project-level: suppress all existing violations
3. **Inline comments** (`statik-ignore`) -- line-level: suppress specific known exceptions

#### Source sets (`[scope]` config)

Classify files into named source sets with different roles and analysis behavior. Define `[scope.<name>]` sections in `.statik/rules.toml` to control which files are production code, test code, fixtures, generated code, etc.

```toml
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

Each source set supports the following fields:

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `include` | yes | -- | Glob patterns for files in this source set |
| `exclude` | no | `[]` | Glob patterns to exclude from this source set |
| `role` | no | none | Role for files (currently `"entry_point"` is supported) |
| `lint` | no | `true` | Whether files in this source set are subject to lint rules |
| `analysis` | no | `true` | Whether files appear in analysis output (dead-code, deps, etc.) |

**How source sets affect behavior:**

- **`role = "entry_point"`**: Files in this source set are treated as entry points for dead code analysis, equivalent to the built-in entry point detection for `main.rs`, `*Test.java`, `*.test.ts`, etc. This supplements (does not replace) the hardcoded entry point heuristics.
- **`lint = false`**: Lint violations where the source file belongs to this source set are suppressed. This is useful for test code that legitimately imports from any module.
- **`analysis = false`**: Files in this source set are excluded from analysis command output (dead-code, deps, cycles, impact, summary) but remain in the graph for correct import resolution. This is useful for test fixtures that should not appear as dead code.

Files are classified into the first matching source set (checked in alphabetical order by set name). Files that do not match any source set remain unclassified and are treated normally (lint and analysis enabled, no special role).

When no `[scope]` config exists, all behavior is backward compatible -- the built-in entry point heuristics and default lint/analysis settings apply. Additionally, Java files in standard Maven/Gradle test directories (`**/src/test/java/**`) are automatically classified as the "test" source set and treated as entry points, while files in `**/src/main/java/**` are classified as "production". This auto-detection works for multi-module projects and requires no configuration.

Use `--source-set <name>` to restrict any analysis command to files in a specific source set:

```
statik dead-code --source-set production
statik deps src/main.rs --source-set production
statik lint --source-set production
```

#### AI Agent Integration

`statik lint` is designed to be consumed by AI coding agents. Use `--format json` for structured output that agents can parse and act on:

```
statik lint --format json
```

The JSON output includes `rationale` and `fix_direction` fields (when defined in the config) that give agents the context to understand *why* a violation exists and *how* to fix it, without requiring the agent to understand the full architectural intent behind the rule.

Recommended agent workflow:

1. Run `statik lint --format json` and parse `violations`
2. For each violation, read `description`, `rationale`, and `fix_direction` to understand what to fix
3. Apply the fix
4. Re-run `statik lint --format json` to verify the violation is resolved

See the [JSON output example](#json---format-json) above for the full violation schema.

### `statik diff --before <path>`

Compare the current project's export surface against a previous index snapshot. Detects added, removed, and changed exports across all files.

```
statik diff --before old-index.db
statik diff --before old-index.db --format json
```

| Flag | Description |
|------|-------------|
| `--before <path>` | Path to the baseline index database to compare against |

The `--before` database is typically a copy of `.statik/index.db` from a previous point in time. The command compares the baseline against the current (or auto-indexed) project state.

### `statik symbols`

List symbols in the project with optional filters. Shows name, kind, file, line number, and visibility.

```
statik symbols
statik symbols --file src/utils/helpers.ts
statik symbols --kind function
statik symbols --format json
```

| Flag | Description |
|------|-------------|
| `--file <path>` | Show only symbols in a specific file |
| `--kind <kind>` | Filter by symbol kind (`function`, `class`, `method`, `interface`, `type_alias`, `enum`, `variable`, `constant`, `annotation`, `package`) |

### `statik references <symbol>`

Find all references to a symbol by name. Shows source, target, reference kind, file, and line number.

```
statik references MyClass
statik references helper --kind call
statik references MyClass --file src/models/user.ts
statik references MyClass --format json
```

| Flag | Description |
|------|-------------|
| `--kind <kind>` | Filter by reference kind (`call`, `type_usage`, `inheritance`, `import`, `export`, `field_access`, `assignment`) |
| `--file <path>` | Filter references to a specific file |

### `statik callers <symbol>`

Find all call sites of a symbol. This is equivalent to `statik references <symbol> --kind call`, but shows only incoming calls with the calling function name.

```
statik callers helper
statik callers processData --file src/main.ts
statik callers processData --format json
```

| Flag | Description |
|------|-------------|
| `--file <path>` | Filter callers to a specific file |

### `statik owners <glob>`

Show file ownership based on git history. Requires `statik index --with-history` to have been run first. Uses recency-weighted commit scoring to determine who "owns" each file.

```
statik owners "src/**"
statik owners "src/core/**" --top 5
statik owners "src/**" --half-life 365
statik owners "src/**" --half-life-mode fixed
statik owners "src/**" --format json
```

| Flag | Description |
|------|-------------|
| `--top <N>` | Show only top N owners per file (default: 3) |
| `--half-life <days>` | Recency half-life in days (default: 180) |
| `--half-life-mode fixed\|adaptive` | Half-life mode (default: `adaptive`). Adaptive mode scales the half-life with file age so original creators of old files retain meaningful ownership |

### `statik bus-factor [glob]`

Analyze bus factor risk: ownership concentration combined with dependency fan-in. Identifies files where knowledge is concentrated in too few people and that knowledge is critical (many other files depend on them).

```
statik bus-factor
statik bus-factor "src/core/**"
statik bus-factor --threshold 0.2
statik bus-factor --by-author
statik bus-factor --format json
```

| Flag | Description |
|------|-------------|
| `--threshold <0.0-1.0>` | Ownership threshold for counting as a contributor (default: 0.1 = 10%) |
| `--half-life <days>` | Recency half-life in days (default: 180) |
| `--half-life-mode fixed\|adaptive` | Half-life mode (default: `adaptive`). Adaptive mode scales the half-life with file age |
| `--by-author` | Show per-person ownership concentration instead of per-file. Aggregates sole-owned file count, key areas, and total blast radius per author |

### `statik churn [glob]`

Analyze file change frequency and co-change patterns from git history. Identifies hot files (changed frequently) and hidden couplings (files that change together without a direct dependency).

```
statik churn
statik churn "src/**"
statik churn --co-change
statik churn --co-change --min-co-changes 5
statik churn --since 2024-01-01
statik churn --format json
```

| Flag | Description |
|------|-------------|
| `--co-change` | Switch to co-change analysis mode (find files that change together) |
| `--since <YYYY-MM-DD>` | Only include changes after this date |
| `--until <YYYY-MM-DD>` | Only include changes before this date |
| `--min-co-changes <N>` | Minimum co-change count to report (default: 3) |

### `statik who <path>`

Impact-aware reviewer suggestion. Combines blast radius analysis with git ownership data to answer: "if I change this file, who should I talk to?" This is `git blame` meets `statik impact` -- it follows dependencies, not just file history.

Requires `statik index --with-history` to have been run first.

```
statik who src/core/engine.ts
statik who src/core/engine.ts --max-depth 2
statik who src/core/engine.ts --format json
```

The output includes three sections:

- **Direct owners**: who owns the target file itself (from git history)
- **Suggested reviewers**: a minimal set of people covering all affected files (greedy set-cover algorithm)
- **Downstream owners**: all people who own files in the blast radius, ranked by total ownership weight

| Flag | Description |
|------|-------------|
| `--half-life <days>` | Recency half-life in days (default: 180) |
| `--half-life-mode fixed\|adaptive` | Half-life mode (default: `adaptive`). Adaptive mode scales the half-life with file age |
| `--top <N>` | Show top N owners per affected file (default: 3) |
| `--max-depth <N>` | Limit blast radius depth (global flag) |

### `statik team-coupling [glob]`

Analyze cross-team coordination costs. Maps git authors to teams and identifies files that require coordination across team boundaries, plus dependency edges that cross team boundaries.

Requires `statik index --with-history` to have been run first.

Teams are configured via the `[teams]` section in `.statik/rules.toml`:

```toml
[teams]
platform = ["*@platform.example.com", "alice@example.com"]
product = ["*@product.example.com"]
infra = ["*@infra.example.com"]
```

When no `[teams]` config exists, teams are inferred from email domains.

```
statik team-coupling
statik team-coupling "src/core/**"
statik team-coupling --format json
```

Files where more than 2 teams have >10% ownership each are flagged as cross-team coordination hotspots. The output also includes dependency edges that cross team boundaries (source file owned by one team depends on a target file owned by a different team).

| Flag | Description |
|------|-------------|
| `--half-life <days>` | Recency half-life in days (default: 180) |
| `--half-life-mode fixed\|adaptive` | Half-life mode (default: `adaptive`) |
| `--threshold <0.0-1.0>` | Ownership threshold for counting as a contributing team (default: 0.1) |

### `statik enrich <scip-file>...`

Import one or more [SCIP](https://github.com/sourcegraph/scip) (Source Code Intelligence Protocol) index files into the existing statik database, upgrading tree-sitter heuristic references to compiler-resolved precision.

SCIP indexes are generated by external tools -- `scip-typescript` (via tsc), `scip-java` (via Gradle/Maven), `rust-analyzer` (emits SCIP natively), or `scip-clang` (via clang). statik reads the `.scip` protobuf output and merges the resolved symbols and references into its SQLite index.

This is an optional enrichment step. Tree-sitter indexing (`statik index`) is always the fast path and works without a build. SCIP enrichment adds compiler-grade precision for commands that benefit from it (`dead-code`, `impact`).

```
# Enrich with a single SCIP index
statik enrich java-index.scip

# Enrich with multiple SCIP indexes (e.g., Java + C++)
statik enrich java-index.scip cpp-index.scip
```

Output (text mode):
```
Enriched 150 files (3 skipped): 2340 symbols matched, 8912 references added
```

**How it works**:
- For each SCIP document, statik matches the file to an existing entry in the index by path
- Previous SCIP data for matched files is cleared before importing (re-enrichment is idempotent)
- SCIP definitions are matched to existing tree-sitter symbols by name and line proximity -- no duplicate symbol rows are created. SCIP references are remapped to point at the canonical tree-sitter symbol IDs, so enrichment adds reachability edges without fragmenting the graph
- The enrichment timestamp is recorded; files edited after enrichment are considered stale and fall back to tree-sitter resolution
- `statik summary` shows enrichment status: files enriched, stale files, and tree-sitter-only files
- `statik dead-code` and `statik impact` automatically upgrade confidence to Certain for SCIP-enriched files

**Typical workflow** (CI-generated SCIP):
```
# In CI: generate SCIP indexes during the build
scip-java                       # produces java-index.scip
scip-clang compile_commands.json  # produces cpp-index.scip

# Locally: index with tree-sitter, then enrich with CI artifacts
statik index
statik enrich java-index.scip cpp-index.scip
statik dead-code   # now uses precise SCIP data where available
```

## Global Flags

| Flag | Description |
|------|-------------|
| `--format text\|json\|compact\|csv` | Output format (default: `text`) |
| `--no-index` | Skip auto-indexing, use existing index only |
| `--include <glob>` | Include only files matching this glob |
| `--exclude <glob>` | Exclude files matching this glob |
| `--lang <language>` | Filter to a specific language (`typescript`, `javascript`, `java`, `rust`) |
| `--max-depth <N>` | Limit transitive depth for dependency/impact analysis |
| `--runtime-only` | Exclude type-only imports, showing only runtime dependencies (applies to `deps`, `dead-code`, `cycles`, `impact`) |
| `--path-filter <glob>` | Filter results to files matching this glob pattern |
| `--count` | Output only the count of results instead of full output |
| `--limit <N>` | Limit the number of results shown |
| `--sort <field>` | Sort results by field (`path`, `confidence`, `name`, `depth`) |
| `--reverse` | Reverse the sort order |
| `--source-set <name>` | Restrict analysis to files in a named source set (defined in `[scope]` config) |
| `--jq <expression>` | Apply a jq filter to JSON output (implicitly sets `--format json`) |

## How It Works

statik uses [tree-sitter](https://tree-sitter.github.io/) to parse source files into concrete syntax trees, then extracts symbols (functions, classes, interfaces, types, variables, constants, enums, annotations) and their relationships (imports, exports, call references, inheritance).

The data flow is:

1. **File discovery** -- Walk the project directory respecting `.gitignore`, detect language by file extension
2. **Parsing** -- Parse each file with tree-sitter (parallel via rayon)
3. **Extraction** -- Extract symbols, imports, exports, and references from the syntax tree
4. **Import resolution** -- Resolve import paths to actual files (relative paths, tsconfig path aliases, index file resolution)
5. **Storage** -- Persist everything to a SQLite database at `.statik/index.db`
6. **Analysis** -- Query the stored data for dependency graphs, dead code, cycles, etc.

### Import resolution

Each language has a dedicated import resolver.

**TypeScript/JavaScript** imports are resolved using:

- **Relative imports** (`./foo`, `../bar`) with extension probing (.ts, .tsx, .js, .jsx, .mjs, .cjs)
- **Index file resolution** (`./services` resolves to `./services/index.ts`)
- **tsconfig.json `paths` aliases** (e.g., `@/components/Button` mapped via tsconfig paths)
- **tsconfig.json `baseUrl`** for non-relative module resolution
- **External package detection** -- bare specifiers like `react` or `lodash` are classified as external and not followed

**Java** imports are resolved using:

- **Package-to-directory mapping** -- fully-qualified class names are converted to file paths (e.g., `com.example.Foo` resolves to `com/example/Foo.java`)
- **Source root detection** -- automatically detects Maven/Gradle source roots (`src/main/java`, `src/test/java`) and falls back to the project root for flat layouts
- **Static import resolution** -- static imports like `import static com.example.Foo.bar` resolve to the containing class file
- **External package detection** -- imports from `java.*`, `javax.*`, `jakarta.*`, and common third-party packages (Spring, JUnit, etc.) are classified as external

**Rust** imports are resolved using:

- **Module tree resolution** -- `mod foo;` declarations are resolved to `foo.rs` (2018 style) or `foo/mod.rs` (2015 style)
- **Crate-relative paths** -- `use crate::foo::Bar` is resolved by walking the module tree from the crate root (`src/lib.rs` or `src/main.rs`)
- **Relative paths** -- `use super::Bar` and `use self::Bar` are resolved relative to the current module
- **External crate detection** -- imports from `std`, `core`, `alloc`, and crates not found in the project module tree are classified as external
- **Crate root detection** -- automatically detects `src/lib.rs`, `src/main.rs`, and binary targets in `src/bin/`

### What gets extracted

**TypeScript/JavaScript:**

- **Functions** (including async, generators, arrow functions assigned to variables)
- **Classes** (with methods, properties, heritage/extends/implements)
- **Interfaces**
- **Type aliases**
- **Enums** (with variants)
- **Variables and constants**
- **Import statements** (named, default, namespace, re-exports, dynamic `import()`)
- **Export statements** (named, default, re-exports including `export *` chains)
- **Call references** (function calls and `new` expressions within function bodies)
- **Inheritance references** (extends, implements)
- **Intra-file references** (resolved to actual symbol IDs and stored in the database)

**Java:**

- **Classes** (with methods, fields, constructors, nested classes)
- **Interfaces** (with method declarations, constants)
- **Enums** (with constants and methods)
- **Annotations** (`@interface` declarations)
- **Records**
- **Fields** (`static final` fields are classified as constants)
- **Import statements** (regular, wildcard, static)
- **Public top-level types are exported** (public classes, interfaces, enums, and annotations)
- **Public nested type exports** (inner classes/interfaces where all ancestors are also public)
- **Same-package type references** (field types, parameter types, return types, local variable types, generic type arguments, casts, instanceof, throws clauses)
- **Wildcard import resolution** (`import com.example.*` resolves to all `.java` files in the package directory)
- **Annotation-based entry point detection** (`@SpringBootApplication`, `@Test`, `@Component`, `@Service`, `@Repository`, `@Controller`, `@RestController`, `@Configuration`, `@Bean`, `@ParameterizedTest`, `@RepeatedTest`, `@Endpoint`, `@WebServlet`)
- **Call references** (method calls and `new` expressions)
- **Inheritance references** (extends, implements)

**Rust:**

- **Functions** (top-level and methods within `impl` blocks)
- **Structs**
- **Enums** (with variants)
- **Traits** (with method declarations)
- **Type aliases**
- **Constants and statics**
- **Modules** (both inline `mod foo { }` and external `mod foo;`)
- **Macro definitions** (`macro_rules!`)
- **Use declarations** (simple, grouped `{A, B}`, wildcard `*`, aliased `as`, nested)
- **Module declarations** (`mod foo;`) create structural dependency edges to the module file
- **`pub use` re-exports**
- **`extern crate` declarations**
- **Visibility tracking** (`pub` -> Public, `pub(crate)`/`pub(super)` -> Protected, no modifier -> Private)
- **Call references** (function calls, method calls, struct expressions)
- **Inheritance references** (`impl Trait for Type`)
- **Type references** (`type_identifier` nodes)
- **`#[cfg(test)]` scope tagging** -- imports inside `#[cfg(test)]` blocks are tagged and excluded from production dependency edges
- **Intra-file reference resolution**

### Storage

The index is stored at `.statik/index.db` in the project root. Add `.statik/` to your `.gitignore`. The database uses SQLite with WAL mode for fast writes.

## Supported Languages

| Language | Status |
|----------|--------|
| TypeScript (.ts, .tsx) | Supported |
| JavaScript (.js, .jsx, .mjs, .cjs) | Supported |
| Java (.java) | Supported |
| Rust (.rs) | Supported |
| Python (.py, .pyi) | File discovery only (no parser) |

Python files are discovered during indexing but skipped during parsing because no language-specific extractor is implemented yet.

## Limitations

statik uses tree-sitter for syntactic analysis, not semantic analysis. This means:

- **No type-level analysis** -- tree-sitter parses syntax, not types. statik cannot determine the type of a variable or resolve method calls through dynamic dispatch (e.g., `obj.method()` where `obj`'s type is unknown).

- **No `node_modules` analysis** -- third-party packages are treated as external dependencies. Imports from packages like `react` or `lodash` are recorded but not followed into `node_modules/`.

- **Barrel file accuracy** -- `export *` re-export chains are traced through to resolve symbol usage, but deep chains of `export *` through multiple barrel files may have reduced confidence.

- **Dynamic imports with computed paths** -- `import()` with string literal arguments (e.g., `import('./lazy')`) is fully resolved and creates dependency edges. Dynamic imports with computed paths (e.g., `import(\`./modules/${name}\`)`) cannot be resolved statically and are flagged as unresolvable.

- **Side-effect imports tracked but unnamed** -- Imports like `import './polyfill'` are recorded as dependencies (creating file-level edges in the graph), but since they import no named symbols, they do not contribute to export usage counts.

- **Precision over recall** -- statik is designed to avoid false positives. It may miss some dead code, but it should never falsely flag live code as dead. When confidence is low, the output says so.

Java-specific limitations:

- **No classpath resolution** -- statik resolves imports by mapping package names to source directories. It does not read `pom.xml`, `build.gradle`, or classpath configuration. External dependencies are classified as external and not followed.

- **No annotation processing** -- annotations are extracted as symbols but annotation processor behavior (code generation, compile-time effects) is not modeled. Annotation-based entry point detection uses a hardcoded list of known annotations; meta-annotations and custom framework annotations are not followed.

- **Wildcard import overapproximation** -- `import com.example.*` resolves to all `.java` files in the package directory, creating edges to every file regardless of which classes are actually used. This is an overapproximation that may inflate dependency counts.

- **No Spring DI container modeling** -- Spring dependency injection wiring (`@Autowired`, `@Inject`, constructor injection) is not modeled. statik tracks the annotation references but does not infer runtime dependency edges from DI configuration.

- **No Lombok support** -- Lombok-generated code (getters, setters, builders, etc.) is not visible to tree-sitter since it is generated at compile time.

Rust-specific limitations:

- **No proc macro expansion** -- derive macros, attribute macros, and function-like proc macros are not expanded. Code generated by proc macros (e.g., serde derives, thiserror) is invisible to statik. This is analogous to Lombok in Java.

- **No `#[macro_export]` detection** -- `macro_rules!` definitions are always treated as Private visibility. The `#[macro_export]` attribute, which makes macros public at the crate root, is not recognized.

- **Limited `#[cfg]` evaluation** -- `#[cfg(test)]` is recognized: imports inside `#[cfg(test)]` blocks are tagged and excluded from production dependency edges. However, other conditional compilation attributes (`#[cfg(feature = "...")]`, platform-specific `#[cfg(target_os = "...")]`) are not evaluated. All non-test `#[cfg]` branches are parsed unconditionally, which may create dependency edges to platform-specific modules.

- **No `#[path = "..."]` attribute support** -- custom module path attributes are not recognized. Module resolution uses the standard `foo.rs` / `foo/mod.rs` convention only.

- **No Cargo workspace cross-crate resolution** -- imports across workspace crates (e.g., `use other_crate::Foo`) are classified as external. Each crate in a workspace is analyzed independently.

- **No build.rs generated code visibility** -- code generated by build scripts (e.g., protobuf bindings, phf maps) is not visible to statik.

- **No feature flag resolution** -- Cargo feature flags are not read or evaluated. All code is parsed regardless of feature gates.

- **Wildcard use creates single edge** -- `use foo::*` creates a single dependency edge to the module file rather than expanding to individual items, similar to Java's wildcard import behavior.

## Output Formats

### Text (default)

Human-readable output for all commands. Each command produces structured, readable text by default.

### JSON (`--format json`)

Machine-readable JSON output designed for consumption by AI coding assistants and other tools. Pretty-printed with indentation. Most analysis commands include:

- **`confidence`**: Overall analysis confidence (`certain`, `high`, `medium`, `low`)
- **`summary`**: Quick overview statistics

Some commands also include:

- **`tier`**: `"general"` in v1 (syntactic analysis via tree-sitter) -- present in `exports` and `summary`
- **`limitations`**: Array of strings describing what could not be resolved -- present in `dead-code`

Example (`statik dead-code --format json`):

```json
{
  "dead_files": [
    {
      "file_id": 5,
      "path": "src/utils/deprecated.ts",
      "confidence": "certain"
    }
  ],
  "dead_exports": [
    {
      "file_id": 3,
      "path": "src/utils/math.ts",
      "export_name": "oldHelper",
      "line": 0,
      "confidence": "certain",
      "kind": "export"
    }
  ],
  "confidence": "high",
  "limitations": [
    "2 imports could not be resolved"
  ],
  "summary": {
    "total_files": 42,
    "dead_files": 1,
    "total_exports": 87,
    "dead_exports": 1,
    "entry_points": 5,
    "files_with_unresolvable_imports": 2
  }
}
```

Example (`statik lint --format json`):

```json
{
  "violations": [
    {
      "rule_id": "no-ui-to-db",
      "severity": "error",
      "description": "UI layer must not import from database layer",
      "rationale": "The UI should go through the service layer",
      "source_file": "src/ui/Button.ts",
      "target_file": "src/db/connection.ts",
      "imported_names": ["getConnection"],
      "line": 5,
      "confidence": "certain",
      "fix_direction": "Import from src/services/ instead"
    }
  ],
  "rules_evaluated": 1,
  "summary": {
    "total_violations": 1,
    "errors": 1,
    "warnings": 0,
    "infos": 0,
    "rules_evaluated": 1
  }
}
```

The `rationale` and `fix_direction` fields are included when defined in the config, providing context for AI assistants and developers to understand and resolve violations.

### Compact (`--format compact`)

Single-line JSON output, suitable for piping to other tools.

### CSV (`--format csv`)

Comma-separated values output with a header row. Each command produces columns appropriate to its data. Fields containing commas are quoted. Useful for agents and scripts that need simple text processing without a JSON parser.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (command failed, file not found in index, no index and `--no-index` used, or `lint` found errors) |

## License

See LICENSE file.
