# Contributing to statik

## Development Setup

### Prerequisites

- Rust toolchain (install via [rustup](https://rustup.rs/))
- Cargo (included with Rust)

### Build

```
cargo build
```

### Run tests

```
cargo test
```

### Run the CLI

```
cargo run -- index /path/to/project
cargo run -- dead-code --format json
cargo run -- deps src/utils/helpers.ts --transitive
cargo run -- cycles
cargo run -- impact src/models/user.ts
cargo run -- summary
```

## Project Structure

```
statik/
  Cargo.toml                          # Dependencies and project metadata
  src/
    main.rs                           # Entry point, CLI parsing, command dispatch
    cli/
      mod.rs                          # Clap command definitions (Cli, Commands, OutputFormat)
      commands.rs                     # Analysis command implementations (deps, exports, dead-code, cycles, impact, summary)
      index.rs                        # `statik index` implementation
      output.rs                       # Output formatting (text, JSON, compact)
    parser/
      mod.rs                          # LanguageParser trait and ParserRegistry
      typescript.rs                   # TypeScript/JavaScript extractor (tree-sitter)
    model/
      mod.rs                          # Core types: Symbol, Reference, ImportRecord, ExportRecord, etc.
      graph.rs                        # SymbolGraph: in-memory symbol-level graph with adjacency lists
      file_graph.rs                   # FileGraph: file-level dependency graph with import resolution
    db/
      mod.rs                          # SQLite database layer (schema, CRUD, transactions)
    discovery/
      mod.rs                          # File discovery with gitignore support
    resolver/
      mod.rs                          # Resolver trait, Resolution enum, ProjectContext
      typescript.rs                   # TypeScript/JavaScript import resolver
      tsconfig.rs                     # tsconfig.json parser (baseUrl, paths)
    analysis/
      mod.rs                          # Confidence levels, analysis utilities
      dead_code.rs                    # Dead file and dead export detection (BFS from entry points)
      cycles.rs                       # Circular dependency detection (Tarjan's SCC)
      dependencies.rs                 # Dependency chain analysis (BFS, directional, max depth)
      impact.rs                       # Refactoring blast radius analysis (reverse BFS)
  tests/
    integration/                      # Integration tests (placeholder)
    fixtures/                         # Test fixtures for integration tests
  test-fixtures/                      # Project-like test fixtures for validation
    basic-project/                    # Clean TS project with imports/exports
    barrel-exports/                   # Re-export patterns (barrel files)
    circular-deps/                    # Circular dependency patterns
    dynamic-imports/                  # Dynamic import() usage
    edge-cases/                       # Edge cases and unusual patterns
    monorepo/                         # Multi-package workspace
    perf-100/                         # 100-file performance test
    perf-500/                         # 500-file performance test
  docs/
    architecture/ARCHITECTURE.md      # Detailed architecture document
    research/prior-art.md             # Prior art survey
    review/devils-advocate.md         # Design review and critique
    testing/test-report.md            # Test validation report
```

## Key Abstractions

### LanguageParser trait (`src/parser/mod.rs`)

Each supported language implements this trait to convert source code into symbols, references, imports, and exports.

```rust
pub trait LanguageParser: Send + Sync {
    fn parse(&self, file_id: FileId, source: &str, path: &Path) -> Result<ParseResult>;
    fn supported_languages(&self) -> &[Language];
}
```

The `ParserRegistry` holds all registered parsers and dispatches based on file language. To add a new language, implement `LanguageParser` and register it in `ParserRegistry::with_defaults()`.

### Resolver trait (`src/resolver/mod.rs`)

Each language implements this trait to resolve import paths to actual files.

```rust
pub trait Resolver: Send + Sync {
    fn resolve(&self, import_source: &str, from_file: &Path) -> Resolution;
}
```

`Resolution` is an enum: `Resolved(PathBuf)`, `ResolvedWithCaveat(PathBuf, ResolutionCaveat)`, `External(String)`, or `Unresolved(UnresolvedReason)`. This allows analysis to track confidence levels based on resolution quality.

### Core Data Model (`src/model/mod.rs`)

- **Symbol** -- A named entity in source code (function, class, variable, etc.)
- **Reference** -- A relationship between two symbols (call, import, inheritance)
- **ImportRecord** -- An import statement extracted from source
- **ExportRecord** -- An export statement extracted from source
- **ParseResult** -- The output of parsing a single file

### Two Graph Layers

1. **SymbolGraph** (`src/model/graph.rs`) -- Symbol-level adjacency lists. Maps symbol-to-symbol references for callers/callees analysis.

2. **FileGraph** (`src/model/file_graph.rs`) -- File-level dependency graph. Built from resolved imports. Used for dead code detection, cycle detection, and impact analysis.

### Database (`src/db/mod.rs`)

SQLite persistence for incremental indexing. Tables: `files`, `symbols`, `refs`, `imports`, `exports`. Uses WAL mode and transactions for performance.

## Adding Support for a New Language

1. **Add tree-sitter grammar dependency** to `Cargo.toml`:
   ```toml
   tree-sitter-python = "0.23"
   ```

2. **Create a new parser file** at `src/parser/<language>.rs`. Implement the `LanguageParser` trait. The TypeScript parser (`src/parser/typescript.rs`) is the reference implementation.

3. **Register the parser** in `ParserRegistry::with_defaults()` in `src/parser/mod.rs`:
   ```rust
   pub fn with_defaults() -> Self {
       let mut registry = Self::new();
       registry.register(Box::new(typescript::TypeScriptParser::new()));
       registry.register(Box::new(python::PythonParser::new()));
       registry
   }
   ```

4. **Add language variants** to the `Language` enum in `src/model/mod.rs` (Python and Rust are already defined but have no parser).

5. **Implement import resolution** for the language. Create a resolver at `src/resolver/<language>.rs` implementing the `Resolver` trait from `src/resolver/mod.rs`. The TypeScript resolver (`src/resolver/typescript.rs`) is the reference implementation, handling relative imports, extension probing, index file resolution, and tsconfig path aliases.

6. **Add test fixtures** under `test-fixtures/<language>-project/` with representative source files.

7. **Add parser tests** in the new parser file. Test each construct the language supports: function/class/variable declarations, imports, exports, references.

## Adding a New Analysis Feature

1. **Create analysis module** at `src/analysis/<feature>.rs`.

2. **Define result types** with `Serialize`/`Deserialize` derives for JSON output. Include a `confidence` field on analysis results.

3. **Implement the analysis** operating on `SymbolGraph` or `FileGraph` data structures.

4. **Add CLI command** in `src/cli/mod.rs` (add variant to `Commands` enum) and wire it up in `src/main.rs`.

5. **Add output formatting** in `src/cli/output.rs` for text and JSON formats.

## Testing Conventions

- **Unit tests** live in `#[cfg(test)] mod tests` blocks within each source file.
- **Integration tests** go in `tests/integration/`.
- **Test fixtures** are real TypeScript/JavaScript project structures under `test-fixtures/`.
- The database layer has an `in_memory()` constructor for fast, isolated tests.
- Parser tests use helper functions like `parse_ts()` and `parse_js()` that create a parser and parse a source string directly.

## Code Style

- Use `anyhow::Result` for error handling in public APIs.
- Use `thiserror` for custom error types where needed.
- Derive `Serialize`/`Deserialize` on types that appear in output.
- Keep modules focused: one file per major concern.
- Prefer `#[derive(Debug, Clone)]` on data types.
