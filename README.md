# statik

Static code analysis for dependency graphs, dead code detection, and circular dependency detection in TypeScript/JavaScript projects.

statik fills a gap between simple text search and full Language Server Protocol (LSP) features. Where LSP gives you go-to-definition and find-references for individual symbols, statik provides **graph-level analysis**: dependency chains between files, dead code detection, circular dependency detection, and refactoring blast radius. These are complementary capabilities -- statik does not replace LSP.

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

This scans all TypeScript and JavaScript files, extracts symbols and import/export relationships, and stores the result in `.statik/index.db` at the project root.

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

## Commands

### `statik index [path]`

Build or update the symbol index for the project at `path` (default: current directory). Creates `.statik/index.db`.

Re-running `index` only re-parses files whose modification time changed. Deleted files are automatically removed.

```
statik index .
statik index /path/to/project --format json
```

### `statik deps <path>`

File-level dependency analysis. Shows what a file imports and what imports it.

```
statik deps src/utils/helpers.ts
statik deps src/utils/helpers.ts --direction out          # only show imports
statik deps src/utils/helpers.ts --direction in           # only show importers
statik deps src/utils/helpers.ts --transitive             # follow chains
statik deps src/utils/helpers.ts --transitive --max-depth 3
```

| Flag | Description |
|------|-------------|
| `--transitive` | Follow dependency chains transitively |
| `--direction in\|out\|both` | Direction of analysis (default: `both`) |
| `--max-depth <N>` | Limit transitive depth |

### `statik exports <path>`

List all exports from a file with used/unused status. Shows which exports are imported by other files and which are not.

```
statik exports src/components/index.ts
statik exports src/utils/math.ts --format json
```

### `statik dead-code`

Find dead code: orphaned files (never imported from any entry point) and unused exports (exported symbols never imported anywhere).

```
statik dead-code
statik dead-code --scope files       # only orphaned files
statik dead-code --scope exports     # only unused exports
statik dead-code --scope both        # both (default)
```

| Flag | Description |
|------|-------------|
| `--scope files\|exports\|both` | What to check for (default: `both`) |

Entry points are never reported as dead. Entry points are detected automatically: files named `index`, `main`, `app`, `server`, `cli`, and test files (`*.test.*`, `*.spec.*`, `*_test.*`, `*_spec.*`).

### `statik cycles`

Detect circular dependencies in the file-level import graph. Reports cycles ordered by length (shortest first, most actionable).

```
statik cycles
statik cycles --format json
```

### `statik impact <path>`

Blast radius analysis: if this file changes, what other files are affected? Performs reverse traversal of the dependency graph to find all direct and transitive dependents.

```
statik impact src/models/user.ts
statik impact src/models/user.ts --max-depth 2
```

| Flag | Description |
|------|-------------|
| `--max-depth <N>` | Limit how far to follow the dependency chain |

### `statik summary`

Project overview: file counts by language, dependency statistics, dead code count, circular dependency count. Designed to fit in a single LLM context message.

```
statik summary
statik summary --format json
```

### `statik lint`

Check architectural rules defined in a config file. Reports violations of boundary rules, layer hierarchies, module containment, import restrictions, and fan-in/fan-out limits.

```
statik lint
statik lint --config path/to/rules.toml
statik lint --rule no-ui-to-db                     # evaluate a single rule
statik lint --severity-threshold warning            # only report warnings and errors
statik lint --format json
```

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to config file (default: `.statik/rules.toml` or `statik.toml`) |
| `--rule <id>` | Only evaluate a specific rule by ID |
| `--severity-threshold error\|warning\|info` | Minimum severity to report (default: `info`) |

The lint command exits with code 1 if any errors are found, and code 0 otherwise (even if warnings are present).

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

### Deferred commands (v2)

These commands require type-resolved analysis and are deferred to a future release with deep mode support:

| Command | Reason for deferral |
|---------|-------------------|
| `statik symbols` | LSP provides better symbol listing with type information |
| `statik references <symbol>` | LSP provides better find-references |
| `statik callers <symbol>` | Requires type resolution for accurate call graphs |

## Global Flags

| Flag | Description |
|------|-------------|
| `--format text\|json\|compact` | Output format (default: `text`) |
| `--no-index` | Skip auto-indexing, use existing index only |
| `--include <glob>` | Include only files matching this glob |
| `--exclude <glob>` | Exclude files matching this glob |
| `--lang <language>` | Filter to a specific language (`typescript`, `javascript`) |
| `--max-depth <N>` | Limit transitive depth for dependency/impact analysis |

## How It Works

statik uses [tree-sitter](https://tree-sitter.github.io/) to parse source files into concrete syntax trees, then extracts symbols (functions, classes, interfaces, types, variables, constants, enums) and their relationships (imports, exports, call references, inheritance).

The data flow is:

1. **File discovery** -- Walk the project directory respecting `.gitignore`, detect language by file extension
2. **Parsing** -- Parse each file with tree-sitter (parallel via rayon)
3. **Extraction** -- Extract symbols, imports, exports, and references from the syntax tree
4. **Import resolution** -- Resolve import paths to actual files (relative paths, tsconfig path aliases, index file resolution)
5. **Storage** -- Persist everything to a SQLite database at `.statik/index.db`
6. **Analysis** -- Query the stored data for dependency graphs, dead code, cycles, etc.

### Import resolution

statik resolves TypeScript/JavaScript imports using a dedicated resolver that handles:

- **Relative imports** (`./foo`, `../bar`) with extension probing (.ts, .tsx, .js, .jsx, .mjs, .cjs)
- **Index file resolution** (`./services` resolves to `./services/index.ts`)
- **tsconfig.json `paths` aliases** (e.g., `@/components/Button` mapped via tsconfig paths)
- **tsconfig.json `baseUrl`** for non-relative module resolution
- **External package detection** -- bare specifiers like `react` or `lodash` are classified as external and not followed

### What gets extracted

- **Functions** (including async, generators, arrow functions assigned to variables)
- **Classes** (with methods, properties, heritage/extends/implements)
- **Interfaces**
- **Type aliases**
- **Enums** (with variants)
- **Variables and constants**
- **Import statements** (named, default, namespace, re-exports)
- **Export statements** (named, default, re-exports)
- **Call references** (function calls and `new` expressions within function bodies)
- **Inheritance references** (extends, implements)

### Storage

The index is stored at `.statik/index.db` in the project root. Add `.statik/` to your `.gitignore`. The database uses SQLite with WAL mode for fast writes.

## Supported Languages

| Language | Status |
|----------|--------|
| TypeScript (.ts, .tsx) | Supported |
| JavaScript (.js, .jsx, .mjs, .cjs) | Supported |
| Python (.py, .pyi) | File discovery only (no parser) |
| Rust (.rs) | File discovery only (no parser) |

Python and Rust files are discovered during indexing but skipped during parsing because no language-specific extractor is implemented yet.

## Limitations

statik uses tree-sitter for syntactic analysis, not semantic analysis. This means:

- **No type-level analysis** -- tree-sitter parses syntax, not types. statik cannot determine the type of a variable or resolve method calls through dynamic dispatch (e.g., `obj.method()` where `obj`'s type is unknown).

- **No `node_modules` analysis** -- third-party packages are treated as external dependencies. Imports from packages like `react` or `lodash` are recorded but not followed into `node_modules/`.

- **Barrel file accuracy** -- `export *` re-export chains are resolved but with reduced confidence. When a barrel file re-exports from another file that also uses `export *`, accuracy degrades.

- **No dynamic import resolution** -- `import()` with computed paths (e.g., `import(\`./modules/${name}\`)`) cannot be resolved statically. These are flagged as unresolvable in the output.

- **Side-effect imports tracked but unnamed** -- Imports like `import './polyfill'` are recorded as dependencies (creating file-level edges in the graph), but since they import no named symbols, they do not contribute to export usage counts.

- **Precision over recall** -- statik is designed to avoid false positives. It may miss some dead code, but it should never falsely flag live code as dead. When confidence is low, the output says so.

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

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (command failed, file not found in index, no index and `--no-index` used, or `lint` found errors) |

## License

See LICENSE file.
