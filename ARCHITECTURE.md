# Architecture

This document describes how statik works at a high level. For the full internal architecture, see `docs/architecture/ARCHITECTURE.md`.

## Overview

statik is a Rust CLI tool that performs static code analysis on TypeScript/JavaScript projects. It extracts symbols and their relationships from source code using tree-sitter, stores them in a SQLite database, and runs graph algorithms to answer questions about dependencies, dead code, and refactoring impact.

### What statik does that LSP does not

LSP (Language Server Protocol) provides symbol-level operations: go-to-definition, find-references, rename. statik provides **graph-level analysis** that operates across the entire codebase:

- **Dependency chains** -- What files does this file depend on, transitively?
- **Dead code detection** -- Which files are never imported? Which exports are never used?
- **Circular dependency detection** -- Where are the import cycles?
- **Refactoring impact** -- If I change this file, what else breaks?

These capabilities are complementary to LSP. statik is designed to be used alongside LSP, not instead of it.

## Data Flow

```
Source Files (.ts, .js, .tsx, .jsx)
        |
        v
  File Discovery          -- Walk project, respect .gitignore, detect language
        |
        v
  Tree-sitter Parsing     -- Parse each file into a concrete syntax tree (parallel)
        |
        v
  Symbol Extraction        -- Walk the CST, extract symbols/imports/exports/references
        |                     Drop the AST immediately (streaming, low memory)
        v
  Import Resolution        -- Resolve import paths to files (relative, tsconfig paths, index files)
        |                     Classify unresolvable imports (external packages, dynamic paths)
        v
  SQLite Storage           -- Persist to .statik/index.db (incremental via mtime)
        |
        v
  Graph Construction       -- Build in-memory adjacency lists from stored data
        |
        v
  Analysis Algorithms      -- Dead code (BFS), cycles (Tarjan's SCC), impact (reverse BFS)
        |
        v
  Output                   -- JSON or text, with confidence levels and limitations
```

## Key Design Decisions

### Tree-sitter for parsing

Tree-sitter provides a uniform parsing API across languages. It produces concrete syntax trees (CST), not abstract syntax trees. This means statik works at the syntactic level -- it sees the structure of the code but not the types. This is a deliberate trade-off: tree-sitter works on any project without build tools or configuration, at the cost of not being able to resolve types or dynamic dispatch.

### Two graph layers

1. **SymbolGraph** -- Symbol-level. Maps function-calls-function, class-extends-class relationships. Used for callers/callees analysis.

2. **FileGraph** -- File-level. Maps file-imports-file relationships. Used for dead code detection, cycle detection, and impact analysis. This is the primary graph for v1.

File-level analysis is both faster and more reliable than symbol-level analysis for the queries statik supports. Even with imperfect symbol resolution, file-level dependency chains are accurate because import statements explicitly name their targets.

### Pluggable import resolution

Import resolution is separated behind a `Resolver` trait, allowing different resolution strategies per language. The TypeScript resolver handles relative imports, tsconfig.json path aliases (`baseUrl`, `paths`), index file resolution, and external package detection. Each resolution returns a typed result (`Resolved`, `ResolvedWithCaveat`, `External`, `Unresolved`) so that analysis algorithms can adjust confidence based on resolution quality.

### SQLite for persistence

The index is stored as a single SQLite file at `.statik/index.db`. This enables:

- **Incremental indexing** -- Only re-parse files whose mtime changed
- **Zero configuration** -- No external database to set up
- **Single file** -- Easy to delete and rebuild

WAL mode is enabled for concurrent read performance.

### Streaming extraction

Files are parsed one at a time. After extracting symbols, imports, and exports, the AST is immediately dropped. Only the extracted data is retained in memory. This keeps memory usage low even for large projects (tens of thousands of files).

### Precision over recall

statik is designed to minimize false positives. The analysis includes confidence levels on every result:

- **Certain** -- All imports resolved, no ambiguity
- **High** -- Minor caveats (e.g., resolved through a barrel file)
- **Medium** -- Some imports unresolvable, result likely correct
- **Low** -- Significant gaps, treat with skepticism

When confidence is low, statik says so rather than asserting. This is critical for AI assistant use cases where false positives waste developer time.

### Parallel parsing

File parsing is parallelized using rayon. Tree-sitter parsers are created per-thread (they are not `Sync`). The database write phase is sequential within a transaction.

## Technology Stack

| Component | Technology | Why |
|-----------|-----------|-----|
| Language | Rust | Performance, single binary distribution, tree-sitter bindings |
| Parsing | tree-sitter | Uniform API, battle-tested (GitHub, Neovim, Zed, Semgrep) |
| CLI | clap (derive) | Standard Rust CLI framework |
| Database | rusqlite (bundled) | Zero-config SQLite, single file |
| File walking | ignore crate | .gitignore support (from the ripgrep project) |
| Parallelism | rayon | Data-parallel file processing |
| Serialization | serde + serde_json | JSON output |

## Analysis Algorithms

### Dead code detection (`analysis/dead_code.rs`)

Operates on `FileGraph`. Two scopes:

- **Dead files**: BFS from entry points (index.ts, main.ts, test files, etc.) following import edges. Files not reached are dead. Entry points are never reported as dead.
- **Dead exports**: For each exported symbol, count how many other files import it. Exports with zero importers are dead. Entry point exports are excluded (they may be consumed externally). Re-exports are excluded.

### Circular dependency detection (`analysis/cycles.rs`)

Tarjan's strongly connected components (SCC) algorithm on the file-level import graph. SCCs with more than one node are circular dependencies. Results are sorted by cycle length (shortest first, most actionable).

### Dependency chain analysis (`analysis/dependencies.rs`)

BFS on the file-level graph in either direction (imports or imported-by), with optional depth limit. Returns dependency trees with depth annotations for display.

### Impact analysis (`analysis/impact.rs`)

Reverse BFS on the imported-by edges from a target file. Collects all transitively affected files, grouped by distance from the target. Answers "if I change this file, what else might break?"

### Confidence levels

All analysis results include per-result and overall confidence. Confidence is computed from the ratio of unresolved imports: zero unresolved = Certain, few unresolved = High, many = Medium or Low. This prevents false positives in the presence of analysis gaps.

## Limitations of Syntactic Analysis

Because statik uses tree-sitter (syntax-only), it cannot:

- Resolve types (`let x: SomeType` -- statik knows the name but not what `SomeType` resolves to)
- Follow dynamic dispatch (`obj.method()` -- cannot determine `obj`'s runtime type)
- Analyze `node_modules` (third-party code is opaque)
- Resolve computed import paths (`import(\`./modules/${name}\`)`)
- Understand TypeScript compiler features (conditional types, template literal types, module augmentation)

These are fundamental limitations of syntactic analysis. A future "deep mode" could integrate with language servers (tsserver, rust-analyzer) to provide compiler-grade precision.
