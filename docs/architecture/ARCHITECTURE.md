# statik - Technical Architecture

## Overview

statik is a CLI tool for **code dependency analysis** with a two-tier architecture. It answers questions that existing tools (LSP, IDE features) do not easily answer: What are the dependency chains between files? Which exports are dead? Where are the circular dependencies? What is the blast radius of changing a module?

statik is designed for two primary consumers:
1. **AI coding assistants** (Claude, etc.) -- JSON output optimized for LLM context windows.
2. **Developers** -- human-readable dependency visualization and dead code reports.

### Two-Tier Analysis Model

statik provides two tiers of analysis, selectable via `--deep` flag or auto-detected:

**Tier 1 -- General mode (always available, zero config):**
- Powered by tree-sitter. Works on any project without setup.
- File-level dependency graphs, dead exports/files, circular dependencies.
- Syntactic analysis only. Honest about its accuracy ceiling.
- This is what v1 ships. It is the default.

**Tier 2 -- Deep mode (optional, requires language backend):**
- Powered by language-specific tools (TypeScript compiler API, rust-analyzer, JDT, etc.) when available.
- Type-resolved call graphs, precise symbol references, method-level dead code.
- Compiler-grade precision but requires the language tool to be installed.
- Auto-detected: if `tsserver` is on PATH, deep mode is available for TypeScript. If not, falls back to general mode gracefully.

This resolves the fundamental tension between breadth and depth. General mode provides useful analysis everywhere. Deep mode delivers compiler-grade results when the infrastructure exists.

### What statik Is NOT

statik is not a replacement for LSP. Claude Code already has native LSP integration with go-to-definition and find-references for 11 languages. statik provides **graph-level analysis** that LSP cannot: dependency chains, dead code detection, circular dependency detection, and refactoring blast radius. These are complementary capabilities. In deep mode, statik can leverage the same language servers that power LSP to enrich its analysis -- but its value proposition remains the graph-level queries.

### Prior Art & Influences

statik's architecture is informed by prior art in code analysis tooling (see `docs/research/prior-art.md` for the full survey):

- **Sourcetrail** (closest prior art): Validated the "symbols + relationships + locations in SQLite" database model. Sourcetrail was discontinued partly because maintaining deep per-language compiler frontends was unsustainable -- we avoid this by using tree-sitter as a uniform parser, accepting lower precision.
- **Semgrep**: Validates tree-sitter as a multi-language parsing foundation at production scale (30+ languages).
- **SCIP (Sourcegraph)**: Inspires our symbol naming scheme. Human-readable, structured symbol IDs are superior to opaque numeric IDs for debugging and testing.
- **rust-analyzer**: Inspires our layered architecture (parsing -> extraction -> resolution -> analysis). Its demand-driven computation is over-engineering for a batch CLI tool, but the layer separation applies.
- **CodeQL**: Inspires the "code as data" philosophy: extract once into a queryable format, then run multiple analyses.
- **Kythe**: Inspires the separation of anchors (source locations) from semantic nodes (symbols), and the concept of partial graph merging for incremental indexing.

statik spans a range on the analysis depth spectrum depending on the tier:
```
ctags ---- tree-sitter ---- [statik general] ---- [statik deep] ---- CodeQL
(text)     (syntax)         (syntax+               (semantic via       (full
                             resolution)            language tools)     compiler)
```

## Architecture Diagram

```
                          CLI Interface
                      (clap command parser)
                        |            |
                  [--deep]?    [default]
                        |            |
                        v            v
                    +-------------------+
                    |   Command Router  |
                    +-------------------+
                     /        |        \
                    v         v         v
             +---------+ +--------+ +----------+
             | Index   | | Query  | | Analyze  |
             | Command | | Engine | | Engine   |
             +---------+ +--------+ +----------+
                  |           |           |
                  v           v           v
            +--------------------------------------+
            |          Symbol Graph                 |
            |     (in-memory adjacency lists)       |
            +--------------------------------------+
                  ^                    ^
                  |                    |
            +------------+     +--------------+
            |  SQLite    |     |  Graph       |
            |  Storage   |     |  Builder     |
            +------------+     +--------------+
                                      ^
                                      |
                               +--------------+
                               |   Resolver   |  <-- pluggable per-language
                               |   Trait      |
                               +--------------+
                                      ^
                                      |
                         +------------------------+
                         |   Extractor Trait       |  <-- pluggable per-language
                         +------------------------+
                          /                     \
                         v                       v
            +---------------------+   +---------------------+
            | Tier 1: General     |   | Tier 2: Deep        |
            | (tree-sitter)       |   | (language backends)  |
            | - always available  |   | - tsserver (TS)      |
            | - zero config       |   | - rust-analyzer (Rust)|
            | - syntactic         |   | - jdtls (Java)       |
            +---------------------+   +---------------------+
                                              ^
                                              |
                                    (auto-detected or --deep)
```

### Key Architectural Principle: Pluggable Backends (Two Tiers)

The architecture separates two concerns behind traits, enabling the two-tier analysis model:

1. **`Extractor` trait**: Converts source code into symbols and raw import/export declarations.
2. **`Resolver` trait**: Resolves raw import paths to actual files and symbols.

```rust
trait Extractor {
    fn extract(&self, source: &str, file_path: &Path) -> ExtractionResult;
    fn tier(&self) -> AnalysisTier;
}

trait Resolver {
    fn resolve_import(&self, import_path: &str, from_file: &Path, project: &ProjectContext) -> Resolution;
    fn tier(&self) -> AnalysisTier;
}

enum AnalysisTier {
    General,  // tree-sitter: syntactic, always available
    Deep,     // language backend: semantic, optional
}

enum Resolution {
    Resolved(PathBuf),
    ResolvedWithCaveat(PathBuf, ResolutionCaveat),
    Unresolved(UnresolvedReason),
}
```

**Tier 1 (General) implementations:**
- `TreeSitterExtractor` -- extracts symbols from tree-sitter CST
- `SyntacticResolver` -- resolves imports using file paths, tsconfig, barrel files

**Tier 2 (Deep) implementations (future):**
- `TsServerExtractor` -- queries TypeScript compiler API for type-resolved symbols
- `TsServerResolver` -- uses tsserver for precise module resolution including node_modules, path mapping, declaration files
- `RustAnalyzerExtractor` / `RustAnalyzerResolver` -- wraps rust-analyzer
- `JdtExtractor` / `JdtResolver` -- wraps Eclipse JDT language server

The analysis engine and CLI never depend on any specific backend. They operate on `ExtractionResult` and `Resolution` regardless of which tier produced them. The tier is recorded in the output so consumers know the precision level.

### Backend Selection

```
1. User passes --deep:
   -> Look for language backend (tsserver, rust-analyzer, etc.)
   -> If found, use Tier 2 extractor/resolver
   -> If not found, warn and fall back to Tier 1

2. User passes --general (or no flag):
   -> Use Tier 1 (tree-sitter) always

3. Auto-detection (future, default in v2+):
   -> Check if language backend is available on PATH
   -> If available, use Tier 2 automatically
   -> If not, use Tier 1 silently
```

v1 ships Tier 1 only. The `--deep` flag is accepted but prints "deep mode not yet available for TypeScript, using general mode" until Tier 2 backends are implemented. This establishes the CLI contract early.

## Technology Choices

### Implementation Language: Rust

**Rationale:**
- **Performance**: Parallel file parsing with rayon. No GC pauses during graph construction.
- **Tree-sitter bindings**: First-class Rust bindings via the `tree-sitter` crate.
- **Single binary distribution**: `cargo build --release` produces a static binary. No runtime dependencies. Distribute via `cargo install`, `brew`, or direct download.
- **Ecosystem**: `clap` (CLI), `serde`/`serde_json` (output), `rusqlite` (persistence), `ignore` (file walking from ripgrep).

**Alternatives considered:**
- TypeScript/Node: Easier prototyping, but 5-10x slower for large-scale parsing. Distribution requires Node runtime or bundling.
- Go: Good single-binary story, but tree-sitter bindings less mature. Lack of enums/pattern matching makes AST work verbose.
- stack-graphs (as primary engine): Promising but relatively young. Better to wrap it as an alternative backend later than to couple to it now.

### Parsing Strategy: Tree-sitter with Pluggable Backend Trait

Tree-sitter for v1:
- Unified API across languages. One parsing pipeline, many grammars.
- Battle-tested: used by GitHub, Neovim, Helix, Zed, Semgrep.
- No build tools required.
- Fast: can parse 10,000 files in seconds.
- **Tree-sitter query language** for declarative symbol extraction: S-expression patterns with captures, rather than manual tree walking. This is the approach used by Semgrep and GitHub code navigation.

**Known limitations (documented honestly):**
- Tree-sitter provides syntax, not semantics. It cannot resolve types, understand dynamic dispatch, or infer generics.
- Import resolution must be built separately per language.
- `export * from` (wildcard re-exports) require knowing what the target file exports, which requires multi-pass resolution.

**Future backends** (behind the `Extractor`/`Resolver` traits):
- stack-graphs: GitHub's library for deterministic name resolution. Already supports TS, Python, Java. Could replace tree-sitter extraction for languages it supports.
- LSP queries: For projects where an LSP server is available, use it for higher-accuracy resolution.
- SCIP indexes: For projects that already generate SCIP indexes (Sourcegraph users).

### Symbol Database: Streaming Extraction + In-Memory Graph + SQLite Persistence

**Critical design constraint: Do NOT hold ASTs in memory.**

Files are processed one-at-a-time in a streaming fashion:
1. Read file
2. Parse with tree-sitter
3. Extract symbols and raw imports/exports
4. Drop the AST immediately
5. Accumulate only the extracted `Symbol` and `RawImport`/`RawExport` records

After all files are extracted, the resolver runs to connect raw imports to files, and the graph is built from the resolved data.

**SQLite persistence** for incremental updates:
- On re-index, check file mtimes, only re-parse changed files.
- SQLite is zero-config, single-file.
- Stored at `.statik/index.db`.

**In-memory graph** for analysis queries:
- Loaded from SQLite on query.
- Adjacency lists for fast BFS/DFS traversal.
- File-level graph is compact: even 100k files produce a manageable graph (files + edges, not all symbols).

### Language Support Strategy

**v1: TypeScript/JavaScript, Java, Rust.**

TypeScript was implemented first because it has the most complex module system
(tsconfig paths, barrel exports, `export *`, package.json exports, conditional
exports, CJS interop). Solving TS first meant other languages were easier. Java
and Rust followed using the same `LanguageParser`/`Resolver` trait architecture.

**v1 TypeScript resolver handles:**
- Relative imports (`./foo`, `../bar`)
- tsconfig `paths` aliases (`@/components/Button`)
- Index file resolution (`import from "./services"` -> `./services/index.ts`)
- Named re-exports (`export { foo } from "./bar"`)
- Wildcard re-exports (`export * from "./bar"`) -- multi-pass resolution
- Type-only imports (`import type { User }`) -- tracked separately
- Side-effect imports (`import "./polyfill"`) -- tracked as file-level dependency
- Namespace imports (`import * as utils from "./utils"`)

**v1 TypeScript resolver explicitly does NOT handle (documented as limitations):**
- `node_modules` resolution (third-party packages are treated as external, not analyzed)
- Dynamic `import()` with computed paths (flagged as "unresolvable" in output)
- `require()` with computed strings
- Module augmentation (`declare module`)
- Ambient declarations (`.d.ts` files treated as regular TS)
- Conditional exports in package.json

**v2:** Python (leverage existing extractor/resolver trait)
**v3:** Go

Note: Java and Rust are now supported via Tier 1 (tree-sitter) general mode.

## Data Model

**NOTE on code convergence:** The architecture data model below is the canonical reference. The current implementation uses simplified types (`ImportRecord` with boolean flags, `ExportRecord` without wildcard support). These must converge to the architecture model before v1 ships. Specific gaps:
- `ImportRecord` -> `RawImport`: needs `ImportedName` enum (Named/Default/Namespace/Wildcard), `is_type_only`, `is_side_effect` fields.
- `ExportRecord` -> `RawExport`: needs `is_type_only`, wildcard re-export support (`export *`).
- SymbolId generation: current `file_id * 100_000 + counter` must be replaced with content-hash IDs (see below).

### Core Types

```rust
// Unique identifiers -- derived from content, NOT counters.
// FileId = hash of relative path (stable across re-indexes).
// SymbolId = hash of (file_path, qualified_name, kind) (stable across re-indexes).
// This ensures that re-indexing a file does not break cross-file references.
//
// IMPORTANT: The current implementation uses file_id * 100_000 + counter for SymbolId.
// This is fragile (breaks for files with >100k symbols, unstable across re-indexes).
// Must migrate to content-hash before v1 ships.
//
// For v1 file-level analysis (deps, cycles, impact), only FileIds matter for the
// dependency graph. SymbolIds are needed for export-level dead code detection,
// where stability matters for incremental re-indexing.
struct FileId(u64);
struct SymbolId(u64);

// Human-readable symbol identifier (SCIP-inspired)
// Format: "language file_path descriptor"
// Example: "typescript src/utils/parser.ts Parser.parse"
// Used in CLI input/output; SymbolId used internally for graph traversal.
struct SymbolName(String);

// What kind of symbol
enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    EnumVariant,
    Interface,
    TypeAlias,
    Variable,
    Constant,
    Module,
}

// A symbol extracted from source code
struct Symbol {
    id: SymbolId,
    name: String,
    qualified_name: String,    // "module::Class::method"
    kind: SymbolKind,
    file: FileId,
    span: Span,                // byte offsets
    line: u32,                 // 1-indexed line number
    col: u32,                  // 1-indexed column
    parent: Option<SymbolId>,  // enclosing symbol
    visibility: Visibility,    // Public, Private, Internal
    signature: Option<String>, // function signature for display
}

enum Visibility {
    Public,       // exported / pub
    Private,      // not exported
    Internal,     // pub(crate) / module-internal
}

// A raw import as extracted from source (before resolution)
struct RawImport {
    file: FileId,
    import_path: String,       // the string literal in the import
    imported_names: Vec<ImportedName>,
    is_type_only: bool,
    is_side_effect: bool,      // import "./polyfill" (no names)
    line: u32,
}

enum ImportedName {
    Named(String),             // import { foo }
    Default,                   // import foo from
    Namespace(String),         // import * as foo
    Wildcard,                  // export * from
}

// A raw export as extracted from source
struct RawExport {
    file: FileId,
    symbol: Option<SymbolId>,  // None for re-exports
    exported_name: String,
    is_re_export: bool,
    re_export_source: Option<String>, // path for re-exports
    is_type_only: bool,
    line: u32,
}

// Resolution result
enum Resolution {
    Resolved(FileId),
    ResolvedWithCaveat(FileId, ResolutionCaveat),
    External(String),          // third-party package, not analyzed
    Unresolved(UnresolvedReason),
}

enum ResolutionCaveat {
    BarrelFileWildcard,        // resolved through export *, may be imprecise
    AmbiguousIndex,            // multiple possible index files
}

enum UnresolvedReason {
    DynamicPath,               // computed import path
    NodeModules,               // third-party (by design)
    FileNotFound,              // dangling import
    UnsupportedSyntax,         // syntax we don't handle
}

// The resolved file-level dependency graph
struct FileGraph {
    files: Vec<FileInfo>,
    // File A imports from File B
    imports: HashMap<FileId, Vec<FileImport>>,
    // File A is imported by File B
    imported_by: HashMap<FileId, Vec<FileImport>>,
}

struct FileImport {
    from: FileId,
    to: FileId,
    imported_names: Vec<String>,
    is_type_only: bool,
    line: u32,
}

struct FileInfo {
    id: FileId,
    path: PathBuf,
    language: Language,
    symbols: Vec<SymbolId>,
    exports: Vec<RawExport>,
}
```

### Confidence Levels in Output

All analysis results include a confidence field:

```rust
enum Confidence {
    Certain,     // all imports resolved, no wildcards, no dynamic paths
    High,        // resolved with minor caveats (barrel file wildcard)
    Medium,      // some imports unresolved but result likely correct
    Low,         // significant unresolved imports, treat with skepticism
}
```

This directly addresses the concern about false positives. **Zero false-positive tolerance** means: when confidence is low, we say so rather than asserting dead code.

## CLI Design

### Commands (v1 Scope)

```
statik index [<path>]
    Index the project at <path> (default: current directory).
    Creates/updates .statik/index.db
    Streams progress to stderr.

statik deps [<path>] [--transitive] [--direction in|out|both]
    File-level dependency analysis.
    Default: show direct imports/importers of the given file.
    --transitive: follow the chain.
    Without <path>: show the full file dependency graph.

statik exports [<path>]
    List all exported symbols from a file or directory.
    Shows which exports are used and which are unused.

statik dead-code [--scope files|exports|both]
    Find dead code:
    --scope files: files never imported (orphaned files)
    --scope exports: exported symbols never imported anywhere
    --scope both: (default) both of the above

statik cycles
    Detect and report circular dependency chains.
    Output ordered by cycle length (shortest first).

statik impact <file-or-symbol>
    Blast radius analysis: if this file/export changes, what is affected?
    Shows direct and transitive dependents.

statik summary
    Project overview: file count by language, dependency statistics,
    dead code count, circular dependency count.
    Designed to fit in a single LLM context message.
```

### Commands unlocked by Deep Mode (v2+)

These commands require type-resolved analysis and are only available when a Tier 2 backend is active (`--deep`). In general mode they are not offered, avoiding misleading results from syntactic-only analysis:

- `statik callers <symbol>` -- type-resolved call graph (requires knowing what `obj` is in `obj.method()`)
- `statik references <symbol>` -- precise cross-file references with type information
- `statik dead-code --scope methods` -- method-level dead code (requires call graph)

In general mode, these commands print: "This command requires deep mode. Run with --deep or install tsserver."

### Global Flags

```
--format json|text       Output format (default: json if stdout is not a TTY, text otherwise)
--deep                   Use Tier 2 language backend if available (auto-detected in v2+)
--general                Force Tier 1 tree-sitter analysis even if backend available
--no-index               Skip auto-indexing, use existing index only
--include <glob>         Include only matching files
--exclude <glob>         Exclude matching files
--max-depth <n>          Limit transitive depth (prevent runaway output)
```

### Output Format

**JSON (primary, for AI assistants):**
```json
{
  "command": "dead-code",
  "tier": "general",
  "scope": "exports",
  "confidence": "high",
  "limitations": ["2 files had unresolvable dynamic imports"],
  "results": [
    {
      "file": "src/utils/deprecated.ts",
      "line": 15,
      "export_name": "oldHelper",
      "kind": "function",
      "confidence": "certain",
      "signature": "function oldHelper(x: string): void"
    }
  ],
  "summary": {
    "total_exports": 142,
    "dead_exports": 3,
    "files_analyzed": 87,
    "files_with_unresolvable_imports": 2
  }
}
```

Key JSON output properties:
- Top-level `confidence` for the overall analysis
- Per-result `confidence` for individual findings
- `limitations` array documenting what could not be resolved
- `summary` for quick overview without reading all results
- Results are filterable by `--include`/`--exclude` flags

**Text (for developers):**
```
Dead Exports (3 found, confidence: high)

  src/utils/deprecated.ts:15  oldHelper()           [certain]
  src/utils/deprecated.ts:28  LegacyConfig          [certain]
  src/models/internal.ts:5    InternalState          [high - resolved through barrel]

Limitations:
  - 2 files have dynamic imports that could not be resolved

Summary: 3/142 exports unused across 87 files
```

### Auto-indexing Behavior

When a query command is run:
1. If `.statik/index.db` does not exist, run a full index first (progress on stderr).
2. If `.statik/index.db` exists, check file mtimes. Re-parse only changed files.
3. If `--no-index` is passed, use existing index or error.

## Project Structure

```
statik/
  Cargo.toml
  src/
    main.rs                       # Entry point, clap CLI setup
    cli/
      mod.rs                      # Command definitions and routing
      index.rs                    # statik index
      deps.rs                     # statik deps
      exports.rs                  # statik exports
      dead_code.rs                # statik dead-code
      cycles.rs                   # statik cycles
      impact.rs                   # statik impact
      summary.rs                  # statik summary
      output.rs                   # JSON/text output formatting
    extract/
      mod.rs                      # Extractor trait + registry + tier selection
      tree_sitter_backend.rs      # Tier 1: tree-sitter based extraction (generic)
      typescript.rs               # Tier 1: TS/JS-specific tree-sitter queries
    resolve/
      mod.rs                      # Resolver trait + tier selection
      typescript.rs               # Tier 1: TS/JS syntactic import resolution
      tsconfig.rs                 # tsconfig.json paths parsing
    deep/                         # Tier 2 backends (v2+, stubbed in v1)
      mod.rs                      # Deep backend detection + registry
      tsserver.rs                 # Tier 2: TypeScript compiler API backend (v2)
    model/
      mod.rs                      # Symbol, RawImport, RawExport, etc.
      graph.rs                    # FileGraph: in-memory adjacency lists
      confidence.rs               # Confidence levels
    db/
      mod.rs                      # SQLite schema and CRUD
      migrations.rs               # Schema versioning
    analysis/
      mod.rs                      # Analysis engine coordinator
      dead_code.rs                # Dead file/export detection
      cycles.rs                   # Circular dependency detection (Tarjan's)
      impact.rs                   # Blast radius / transitive dependents
    discovery/
      mod.rs                      # File walking, language detection
  tests/
    integration/
      index_test.rs
      dead_code_test.rs
      cycles_test.rs
      deps_test.rs
    fixtures/
      simple_project/             # Happy path: clean imports
      barrel_exports/             # index.ts re-exports, export *
      path_aliases/               # tsconfig paths
      circular_deps/              # Intentional cycles
      dynamic_imports/            # Dynamic import(), require()
      mixed_js_ts/                # .js, .jsx, .tsx mixed
      type_only_imports/          # import type { ... }
      side_effect_imports/        # import "./polyfill"
      namespace_imports/          # import * as utils
      no_dead_code/               # Negative test: zero dead code expected
      syntax_errors/              # Malformed files (parser robustness)
      large_project/              # 500+ files for perf testing
      monorepo/                   # Workspace with multiple packages
  docs/
    architecture/
      ARCHITECTURE.md             # This file
```

## Data Flow

### Indexing Flow

```
1. File Discovery
   - Walk project directory using `ignore` crate (respects .gitignore)
   - Detect language by file extension (.ts, .tsx, .js, .jsx, .java, .rs)
   - Apply --include/--exclude filters

2. Incremental Check (if .statik/index.db exists)
   - Load file records from SQLite
   - Compare mtimes, build set of changed/new/deleted files
   - Remove records for deleted files

3. Streaming Extraction (parallel via rayon)
   - For each new/changed file:
     a. Read file contents
     b. Parse with tree-sitter (language grammar)
     c. Extract symbols, raw imports, raw exports
     d. DROP the AST (do not hold in memory)
     e. Yield extraction result
   - Memory: only symbols + imports + exports retained, not ASTs

4. Import Resolution (sequential, needs full file list)
   - For each raw import:
     a. Run language-specific resolver
     b. Produce Resolution (Resolved, External, Unresolved)
   - For wildcard re-exports (export *):
     Multi-pass: resolve export targets first, then expand wildcards

5. Graph Construction
   - Build FileGraph from resolved imports
   - Build adjacency lists (imports, imported_by)
   - Compute per-file and per-export reachability metadata

6. Persistence
   - Write to SQLite: files, symbols, imports, exports, resolutions
   - Update mtimes
```

### Query Flow

```
1. Auto-index if needed
2. Load FileGraph from SQLite into memory
3. Execute analysis algorithm (dead code, cycles, impact, etc.)
4. Attach confidence levels to results
5. Format output (JSON or text) with limitations documented
```

## Analysis Algorithms

### Dead Code Detection

Two scopes:

**Dead files (orphaned):**
```
1. Identify entry points:
   - Files referenced in package.json "main", "module", "exports"
   - Files matching common entry patterns: index.ts, main.ts, app.ts
   - Test files (*test*, *spec*)
   - User-configurable via --entry-points glob

2. BFS from entry points following file-level import edges.

3. Unreachable files = all files - reachable files - entry points.

4. Confidence: Certain if all imports in the project resolved.
   Reduced if dynamic imports exist.
```

**Dead exports (unused):**
```
1. For each exported symbol across all files:
   Count how many times it is imported by another file.

2. Exports with zero imports = dead exports.

3. Exception: entry point file exports are not dead
   (they may be consumed externally).

4. Exception: re-exported symbols -- trace through the re-export chain.

5. Confidence: Certain if the export is named and all importers resolved.
   Reduced if any file uses `import *` or `export *` from the exporting file.
```

**False positive prevention:**
- Never report entry point exports as dead.
- If a file has unresolvable imports, reduce confidence for all symbols in that file's dependency chain.
- Report "possibly dead" vs "certainly dead" separately.
- If in doubt, do not report. Precision over recall.

**Namespace import handling (`import * as ns from "./mod"`):**

When a file uses a namespace import, we cannot syntactically determine which specific exports are accessed (e.g., `ns.helperA()` vs `ns.helperC`) without type-level analysis. The v1 decision:

- **Conservative approach (v1):** Treat a namespace import as "all exports are used." If any file has `import * as ns from "./mod"`, all of `./mod`'s exports are marked as used. This means we will miss some dead exports (lower recall) but never falsely report a live export as dead (zero false positives).
- **Rationale:** Without type information, resolving `ns.helperA()` to a specific export requires knowing that `ns` is of the namespace type -- tree-sitter sees `ns.helperA` as a member expression with an unknown receiver, which could be any object. The conservative approach preserves our zero-false-positive guarantee.
- **Deep mode (v2+):** With type-resolved analysis, we can trace which namespace members are actually accessed and only mark those as used. This restores recall.
- **`export *` receives the same treatment:** If file A has `export * from "./mod"` and file B imports from A, all of `./mod`'s exports are conservatively marked as used through A.

### Circular Dependency Detection

```
1. Build directed graph: file A -> file B if A imports B.
2. Run Tarjan's strongly connected components algorithm.
3. SCCs with more than one node are circular dependencies.
4. Report each cycle with the files involved and the import chain.
5. Order by cycle length (shortest cycles first -- most actionable).
```

### Impact Analysis (Blast Radius)

```
1. Resolve target to FileId (or SymbolId for export-level).
2. BFS/DFS on the imported_by edges (reverse dependency direction).
3. Collect all transitively affected files.
4. Group by depth (direct dependents, 2-hop, 3-hop, etc.).
5. Optionally limit depth with --max-depth.
```

## Cross-File Resolution: TypeScript/JavaScript

This is the hardest component. The v1 resolver handles these cases in order of priority:

### Resolution Algorithm

```
resolve(import_path, from_file, project_context) -> Resolution:

  1. If import_path starts with "." or "..":
     -> Relative import. Resolve against from_file's directory.
     -> Try extensions: .ts, .tsx, .js, .jsx, /index.ts, /index.tsx, /index.js
     -> Return Resolved or FileNotFound.

  2. If import_path matches a tsconfig "paths" pattern:
     -> Substitute the path alias.
     -> Re-run step 1 with the substituted path.
     -> Return ResolvedWithCaveat if alias is ambiguous.

  3. If import_path is a bare specifier (no "./" prefix):
     -> Treat as external package (node_modules).
     -> Return External(package_name).
     -> Do NOT attempt to resolve into node_modules.

  4. Otherwise:
     -> Return Unresolved(UnsupportedSyntax).
```

### Barrel File / Re-export Resolution

```
When a file has `export * from "./other"`:

  Pass 1: Extract all named exports from all files.
  Pass 2: For files with `export *`, merge in the named exports
           from the target file. Repeat until stable (handles
           chained re-exports).
  Pass 3: When resolving `import { Foo } from "./barrel"`,
           check if Foo is in barrel's merged export set.

  Caveat: If the chain includes `export *` from an unresolvable
  target, mark all names from that barrel as Confidence::Medium.
```

## Performance Targets

- **Indexing**: < 3 seconds for a 5,000-file TypeScript project.
- **Incremental update**: < 500ms for re-indexing 10 changed files.
- **Queries**: < 100ms for any query on an indexed project.
- **Memory**: Peak memory during indexing < 200MB for a 10,000-file project (streaming extraction, no AST retention).

## Storage

- `.statik/` directory at project root.
- `.statik/index.db` -- SQLite database.
- `.statik/` should be added to `.gitignore`.
- Database schema is versioned; migrations run automatically.

## Known Limitations (v1 -- General Mode)

These limitations apply to Tier 1 (general mode) analysis. They are documented in CLI `--help` and in JSON output `limitations` fields. Most are resolved by Tier 2 (deep mode) when available.

| Limitation | General mode | Deep mode (future) |
|---|---|---|
| No type-level analysis | Yes -- cannot determine variable types | Resolved via language backend |
| No dynamic dispatch | Yes -- `obj.method()` unresolvable | Resolved via type information |
| No node_modules analysis | Third-party packages are opaque | Resolved via tsserver module resolution |
| No computed import paths | Flagged as unresolvable | Partially resolved (static analysis of templates) |
| Language coverage gaps | Python, Go not yet supported | Per-backend expansion |
| Barrel file accuracy | Reduced confidence on `export *` | Resolved via compiler resolution |
| No conditional exports | `package.json` conditions ignored | Resolved via tsserver |
| No CJS/ESM interop | Incomplete for mixed projects | Resolved via tsserver |

## Future Roadmap

### v1.0 (current)
- Tier 1 (general mode) for TypeScript/JavaScript
- File-level deps, dead code, cycles, impact, exports, summary
- `--deep` flag accepted but prints "not yet available"

### v2 -- Deep Mode + More Languages
- **Tier 2: TypeScript deep backend** (`tsserver`-based extractor/resolver)
  - Type-resolved call graphs, precise module resolution
  - `callers`, `references`, `dead-code --scope methods` commands unlocked
- **Tier 1: Python** general mode (extractor + resolver)
- Auto-detection of available backends (default to deep when available)
- `statik watch` for incremental re-indexing on file changes

Note: **Tier 1: Java** and **Tier 1: Rust** general modes are now complete and
shipped. Rust support includes `RustParser` (tree-sitter-rust), `RustResolver`
(filesystem-based `crate::`/`super::`/`self::` module resolution with Cargo.toml
dependency detection), and entry point detection for `main.rs`, `lib.rs`,
`src/bin/`, `tests/`, `examples/`, `benches/`, and `build.rs`.

### v3 -- Broader Deep Mode Coverage
- **Tier 2: Rust** deep backend (rust-analyzer)
- **Tier 2: Java** deep backend (JDT language server)
- Tier 1: Go general mode
- Cargo workspace cross-crate resolution for Rust
- stack-graphs as alternative Tier 1.5 backend (better than tree-sitter, lighter than full compiler)
- SCIP index consumption as optional enrichment
- IDE extensions
