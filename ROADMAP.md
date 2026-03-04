# statik Roadmap

## Vision

statik aims to be the fastest, most precise CLI-first dependency analysis tool for
typed languages -- starting with TypeScript/JavaScript and expanding to Java and
Rust. It prioritizes correctness over completeness, reporting confidence levels
rather than guessing, and ships as a single binary with zero runtime dependencies.

The north star: a developer runs `statik impact src/UserService.ts`, gets a precise
blast radius in under a second, and trusts the result enough to act on it in CI.

---

## Competitive Positioning

statik occupies a specific niche: **fast, offline, file-level dependency intelligence
from the CLI**. The differentiators worth protecting are:

- **Confidence scores**: Every result carries a confidence level. No tool in the
  space does this -- madge, dependency-cruiser, ts-prune all give binary answers.
- **Re-export awareness**: Barrel file tracing with `is_reexport` tracking is rare
  outside IDEs.
- **Type-only import tracking**: `is_type_only` on imports/exports enables
  distinguishing runtime from compile-time dependencies -- valuable for tree-shaking
  analysis and migration planning.
- **Single binary, SQLite-backed**: No daemon, no node_modules, no JVM. Index once,
  query instantly.
- **Architectural linting**: Configurable rule system for enforcing structural
  patterns -- dependency boundaries, layer hierarchies, module isolation. No other
  CLI-first tool offers this with confidence-aware results and sub-second execution.
  This is the bridge from "analysis tool" to "codebase governance tool" and the
  primary integration surface for AI agents that need to understand and enforce
  codebase style.

The strategy: **deepen before broadening**. Strengthen the TypeScript story until it
is best-in-class, add architectural linting as the high-value governance layer, then
extend to Java and Rust with a clear-eyed view of where the effort actually lives
(resolvers, not parsers).

---

## Phased Plan

### Phase 1: Core Hardening (TS/JS Polish, Performance, Stability)

**Goal**: Make statik the most reliable file-level dependency tool for TypeScript
before adding any new language. Fix known gaps, improve output, and ensure the
architecture scales.

**Deliverables**:

1. **Wildcard re-export tracing** -- The dead code detector has a `TODO` for wildcard
   re-exports (`export * from './module'`). These are common in barrel files and
   cause false negatives in dead code detection. The parser already extracts
   re-exports; the gap is in the analysis layer.

2. **Dynamic import support** -- `import()` expressions are partially parsed but not
   fully tracked. Support string-literal dynamic imports (skip computed paths).

3. **Text output formatting** -- All commands currently fall back to JSON for text
   output (`commands.rs:440`). Add human-readable table/tree formatting for `deps`,
   `dead-code`, `cycles`, `impact`, and `summary`.

4. **Lazy loading for large projects** -- `all_files()`, `all_symbols()`, and
   `all_references()` load everything into memory. Replace with streaming iterators
   for the common case where only a subset is needed. Add pagination or streaming to
   `FileGraph` construction.

5. **Graph caching** -- `build_file_graph()` rebuilds the graph from the DB on every
   command. Cache the serialized graph alongside `index.db` and invalidate on
   index changes.

6. **Test coverage** -- The parser has good unit tests but lacks end-to-end tests
   that run the full pipeline (discover -> parse -> store -> resolve -> analyze).
   Add integration tests with fixture projects.

7. **Structural diff (export surface)** -- Compare the export surface of the project
   between two indexed snapshots (e.g., two git commits). Report added/removed/renamed
   exports at the file level. This is the achievable foundation for Phase 4's
   refactoring intelligence, and it avoids the tree-matching research problem.

**Dependencies**: None (builds on current codebase).

**Complexity**: Medium. Mostly incremental improvements to existing code.

**Success Criteria**:
- Dead code detection handles `export * from` with zero false negatives on a test
  corpus of 10 real-world TS projects.
- `statik summary` on a 10K-file project completes in under 2 seconds.
- All commands have human-readable text output.
- Structural diff command (`statik diff <commit1> <commit2>`) works end-to-end.

**Risks & Mitigations**:
- Lazy loading may require significant refactoring of `FileGraph` construction.
  Mitigate by keeping the eager path as fallback for small projects and only
  switching to lazy mode above a threshold.
- Structural diff requires git integration (reading files at different commits).
  Mitigate by starting with "compare two index.db files" rather than git integration.

---

### Phase 2: Architectural Linting (Configurable Rule Engine)

**Goal**: Turn statik from a passive analysis tool into an active codebase governance
tool. Teams define structural rules in a config file; `statik lint` evaluates them
against the dependency graph and reports violations. This is the highest-value
feature for AI agent integration -- agents consume the machine-readable output to
understand architectural intent and enforce patterns automatically.

**Why now**: The file graph, bidirectional edges, import metadata (including
`is_type_only`, imported names, and line numbers), and confidence system are all
in place. The rule engine is pure graph traversal on existing data structures -- no
new parsing or resolution work is needed. This phase has the best effort-to-value
ratio on the roadmap.

**Deliverables**:

1. **Configuration file format** -- Define rules in `.statik/rules.toml` (or
   `statik.toml` at project root). TOML is chosen for consistency with Rust
   ecosystem conventions. The config supports rule definitions with severity levels
   (error, warning, info) and enforcement modes (enforce, alert).

2. **Boundary rules** -- "Files matching pattern A must not depend on files matching
   pattern B." The foundational rule type. Covers: layer violations, forbidden
   cross-module imports, data access isolation.
   Example: `from = "src/ui/**"`, `to = "src/db/**"`, `allow = false`.

3. **Layer hierarchy rules** -- Define an ordered set of layers; dependencies must
   flow in one direction (typically top-down). A layer is a named group of files
   identified by glob patterns. Violations are reported when a lower layer imports
   from a higher layer.
   Example: layers = ["presentation: src/ui/**", "service: src/services/**",
   "data: src/db/**"] with top-down enforcement.

4. **Module containment rules** -- All imports within a module (directory subtree)
   must stay internal, except through a designated public API file (e.g.,
   `index.ts`). Enforces encapsulation for feature modules.
   Example: `module = "src/auth/**"`, `public_api = "src/auth/index.ts"`.

5. **Import restriction rules** -- Constrain which symbols can be imported from a
   target, or require that imports be type-only.
   Example: "imports from `src/types/**` must be type-only."

6. **Fan-in / fan-out limits** -- Alert when a file exceeds a threshold number of
   dependents (fan-in, architectural bottleneck) or dependencies (fan-out,
   god-module smell). Configurable thresholds per glob pattern.

7. **Tag-based grouping** -- Assign tags to file groups (by glob pattern) and define
   allowed/forbidden dependency relationships between tags. This is the most
   flexible rule type and subsumes boundary rules, but boundary rules remain as
   syntactic sugar for the common case.

8. **`statik lint` command** -- Evaluate all configured rules against the current
   index. Report violations with: rule ID, severity, source file, target file,
   imported names, line number. Exit code 0 for clean, 1 for errors (warnings
   don't fail). Support `--rule <id>` to run a single rule, `--format json` for
   machine consumption.

9. **AI agent integration surface** -- JSON output includes: rule metadata (ID,
   description, rationale), violation details, suggested fix direction (which
   import to remove or redirect), and confidence level. This gives AI agents
   enough context to propose fixes, not just flag violations.

**Dependencies**: Phase 1 items 1.1-1.3 (wildcard re-exports, dynamic imports, text
formatting) are recommended but not strictly required. The lint engine works on
the existing file graph.

**Complexity**: Large. The rule engine itself is Medium; the config parser and
diverse rule types bring it to Large. No individual rule type is complex, but the
surface area is broad.

**Success Criteria**:
- `statik lint` on a project with `.statik/rules.toml` reports boundary violations
  in under 1 second.
- At least 5 rule types are supported: boundary, layer, containment, import
  restriction, fan-in/fan-out.
- JSON output is structured enough for an AI agent to propose a fix for each
  violation.
- A project can adopt architectural linting incrementally -- start with one rule,
  add more over time.

**Risks & Mitigations**:
- **Rule DSL complexity**: Start with simple glob-based patterns. Avoid inventing a
  query language. If glob matching proves insufficient, extend to regex, but resist
  the pull toward a full predicate language.
- **False positives from unresolved imports**: Use the existing confidence system.
  If an import couldn't be resolved, violations involving that edge are reported at
  lower confidence.
- **Config file proliferation**: Keep it to one file (`.statik/rules.toml`). Do not
  add per-directory overrides in v1.
- **Performance**: Rule evaluation is O(edges * rules). For a 10K-file project with
  100 rules, this is ~1M checks -- trivially fast. No optimization needed in v1.

---

### Phase 3: Multi-Language Foundation (Java via tree-sitter)

**Goal**: Add Java as the second supported language, proving that the architecture
genuinely supports multiple languages. C++ is explicitly deferred -- the C
preprocessor makes tree-sitter output unreliable for dependency analysis, and the
effort-to-value ratio is poor compared to Java.

**Deliverables**:

1. **Language enum expansion** -- Add `Language::Java` variant. The `Language` enum
   already has Python and Rust variants for discovery; Java follows the same pattern.

2. **Java tree-sitter parser** -- Implement `LanguageParser` for Java using
   `tree-sitter-java`. Extract: classes, interfaces, enums, methods, fields,
   annotations. Map to existing `SymbolKind` variants (may need `Annotation`,
   `Package` additions).

3. **Java import resolver** -- This is where 70% of the effort lives. Java import
   resolution requires:
   - Package-to-directory mapping (standard `src/main/java` layout)
   - Classpath resolution (compile-time dependencies)
   - Maven/Gradle dependency manifest parsing (to identify external vs internal)
   - Wildcard import handling (`import java.util.*`)

   Start with source-only resolution (same project) and treat all classpath/JAR
   imports as `External`. Do NOT attempt full Maven dependency resolution in v1.

4. **Java file discovery** -- Extend `DiscoveryConfig` to support `.java` files.
   Add standard Java ignore patterns (`target/`, `build/`, `.gradle/`).

5. **Mixed-project support** -- A single project may contain both TS and Java files
   (e.g., a full-stack monorepo). Ensure `FileGraph` correctly handles cross-language
   boundaries (they won't have import edges, but should appear in the graph).

**Dependencies**: Phase 1 lazy loading (Java projects are typically large).

**Complexity**: Large. The parser is Medium; the resolver is Large by itself.

**Success Criteria**:
- `statik index` correctly indexes a standard Maven project (e.g., Spring Boot
  starter).
- `statik deps src/main/java/com/example/UserService.java` shows correct imports.
- `statik dead-code` identifies unused Java files with High confidence.
- External dependencies (from Maven) are correctly classified as `External`.

**Risks & Mitigations**:
- **Java resolver complexity**: Start with convention-based resolution
  (`src/main/java` package layout) before attempting build-tool integration.
  Accept that confidence will be Medium for projects with non-standard layouts.
- **SymbolKind expansion**: Java has constructs (annotations, packages) not in the
  current enum. Add them to the enum but ensure existing analysis code handles
  unknown kinds gracefully.
- **Maintenance multiplier**: Each tree-sitter grammar updates independently.
  Pin grammar versions and update on a deliberate schedule, not reactively.

**Why not C++?**:
The C preprocessor (`#ifdef`, `#include` with search paths, macro expansion) means
tree-sitter sees the pre-preprocessed source, which may not reflect the actual
compilation. A C++ file that `#include`s a header doesn't import symbols in the
Java/TS sense -- it textually includes them. Tools like `clang-tidy` and
`include-what-you-use` solve this better because they operate on the compiler's
actual AST. statik should not compete with compiler-integrated tools.

If C++ support is revisited, the approach should be `libclang`-based (using clang's
own AST), not tree-sitter-based. This is a fundamentally different architecture
decision and belongs in a separate evaluation.

---

### Phase 3b: Rust Support (COMPLETE)

**Goal**: Add Rust as the third supported language using the same tree-sitter
general mode architecture proven with TypeScript and Java. Dogfood statik on its
own codebase.

**Delivered**:

1. **RustParser** (`src/parser/rust.rs`) -- tree-sitter-rust 0.23 extractor
   implementing the `LanguageParser` trait. Extracts functions, structs, enums,
   traits, type aliases, constants, statics, modules, `macro_rules!`, `impl` blocks,
   `use` declarations (simple, grouped, wildcard, aliased, nested), `mod foo;`
   declarations, `pub use` re-exports, `extern crate`, visibility tracking, call
   references, inheritance references, and intra-file reference resolution.

2. **RustResolver** (`src/resolver/rust.rs`) -- filesystem-based module resolution.
   Handles `crate::`, `super::` (including chained), `self::`, external crate
   detection from `Cargo.toml` `[dependencies]`, ambiguous module detection
   (`foo.rs` vs `foo/mod.rs`), and crate root auto-detection (`src/lib.rs`,
   `src/main.rs`, `src/bin/*.rs`).

3. **Entry point detection** -- `lib.rs`, `main.rs`, `src/bin/*.rs`, `tests/`,
   `examples/`, `benches/`, `build.rs` are recognized as entry points.

4. **Integration** -- per-language resolver dispatch in `build_file_graph()`,
   parser registration, test fixtures in `tests/fixtures/rust_project/`, integration
   tests in `tests/rust_integration.rs`.

**What remains (future work)**:

- Cargo workspace cross-crate resolution
- Proc macro expansion (derive, attribute macros)
- `#[macro_export]` visibility detection
- `#[cfg]` evaluation beyond `#[cfg(test)]` (feature flags, platform-specific)
- `#[path = "..."]` custom module paths
- Build script generated code visibility
- Feature flag resolution

**Note**: `#[cfg(test)]` is now handled -- imports inside `#[cfg(test)]` blocks
are tagged and excluded from production dependency edges. Other `#[cfg]` variants
remain unhandled.

---

### Phase 4: Deep Analysis (Type-Aware Dependencies, Symbol-Level Intelligence)

**Goal**: Move beyond file-level analysis to symbol-level precision where tree-sitter
can support it. Activate the deferred v2 commands (`symbols`, `references`,
`callers`).

**Deliverables**:

1. **Symbol-level dead code** -- Extend dead code detection from file-level to
   export-level to symbol-level. A function that is exported but only called within
   its own file is "internally live, externally dead."

2. **Reference resolution improvements** -- Currently, references use placeholder
   SymbolIds (`u64::MAX - counter`) and are not stored in the DB. Improve intra-file
   reference resolution so that call graphs within a single file are accurate.

3. **Activate `symbols` command** -- List all symbols in a file or matching a pattern.
   Backed by existing DB queries (`find_symbols_by_name`, `find_symbols_by_kind`).

4. **Activate `references` command** -- Find all references to a symbol. Requires
   improved reference storage (currently references are not persisted for
   placeholder-target refs).

5. **Activate `callers` command** -- Find all call sites for a function. This is
   `references` filtered to `RefKind::Call`.

6. **Type-only dependency separation** -- `is_type_only` is tracked on imports and
   exports but not used in analysis. Add a `--runtime-only` flag to `deps` and
   `impact` that excludes type-only imports. This is valuable for tree-shaking and
   bundle analysis.

7. **Java-specific deep analysis** -- For Java, extract: inheritance hierarchies
   (`extends`/`implements`), annotation usage, method overrides. These are
   achievable with tree-sitter (syntactic, not semantic) and provide value for
   impact analysis.

**Dependencies**: Phase 3 (Java parser) for Java-specific items. Phase 1 (wildcard
re-exports) for accurate symbol-level dead code.

**Complexity**: Large. Reference resolution is the hardest part.

**Success Criteria**:
- `statik symbols --file src/utils.ts` lists all symbols with their kinds.
- `statik callers UserService.getUser` finds all call sites in the project.
- `statik deps --runtime-only` excludes `import type` edges.
- Symbol-level dead code reports functions that are exported but never imported.

**Risks & Mitigations**:
- **Reference resolution accuracy**: tree-sitter cannot resolve overloaded methods,
  generic type parameters, or dynamic dispatch. Document these as known limitations
  and ensure the confidence system reflects them. Intra-file resolution (same scope)
  is achievable; cross-file resolution without type information is not.
- **Scope creep toward IDE**: Resist the pull toward "just add type checking." That
  path leads to reimplementing tsc/javac. The boundary is: if it requires type
  inference, it's out of scope.

---

### Phase 5: Refactoring Intelligence (Structural Diff, Change Classification)

**Goal**: Help developers understand what changed between two versions of a codebase
at a structural level. This is the "dream feature" -- scoped to what's achievable
without solving the research-grade tree matching problem.

**Deliverables**:

1. **Snapshot comparison** -- Compare two statik indexes (from different commits).
   Report:
   - Files added/removed/modified
   - Exports added/removed/renamed
   - Import edges added/removed
   - New cycles introduced / cycles broken

2. **Change classification** -- Categorize changes as:
   - **Safe**: Internal-only changes (no export surface change)
   - **Breaking**: Removed or renamed exports that have importers
   - **Expanding**: New exports (safe, additive)
   - **Restructuring**: Moved exports between files (detectable via
     add-in-one-file + remove-in-another with same name)

3. **Git integration** -- `statik diff HEAD~1 HEAD` indexes both commits (or uses
   cached indexes) and shows the structural diff. Requires reading source files at
   specific git revisions.

4. **CI integration hook** -- `statik diff --ci` outputs machine-readable breaking
   change report for use in pull request checks. Exit code 1 if breaking changes
   detected.

**Dependencies**: Phase 1 structural diff foundation. Phase 4 for symbol-level
change detail.

**Complexity**: Large for snapshot comparison. XL for full git integration with
caching.

**Success Criteria**:
- `statik diff <sha1> <sha2>` completes in under 5 seconds for a 10K-file project.
- Breaking changes (removed exports with importers) are detected with Certain
  confidence.
- Restructuring (moved exports) is detected with High confidence.
- CI mode produces actionable output that can block a PR on breaking changes.

**Risks & Mitigations**:
- **Rename detection**: Matching "export removed in file A" with "export added in
  file B" is heuristic. Use name + kind matching as the primary signal. Accept that
  confidence will be Medium for rename detection and document this clearly.
- **Performance**: Indexing two full commits is 2x the work. Mitigate with
  incremental indexing -- only re-parse files that differ between commits (use
  `git diff --name-only` to get the changed file list).
- **NOT attempting**: Full AST diff (GumTree-style tree matching), semantic rename
  tracking across type-aware resolution, or refactoring pattern classification
  (extract method, inline variable, etc.). These are research problems. If they
  become tractable, they belong in Phase 7+.

---

### Phase 6: Ecosystem & Integrations (IDE, CI/CD, Visualization)

**Goal**: Make statik's analysis accessible beyond the CLI. This phase is
intentionally last because the analysis must be solid before building UIs on top.

**Deliverables**:

1. **JSON API stabilization** -- Formalize the JSON output schema with versioning.
   The current `--format json` output is ad-hoc (struct-per-command). Define a
   stable schema that external tools can depend on.

2. **VS Code extension** -- Display dependency graph, dead code highlights, and
   impact analysis inline. Uses `statik` CLI as the backend (no daemon).

3. **GitHub Action** -- Run `statik diff` on PRs. Comment with breaking change
   report. Block merge on configurable thresholds.

4. **Dependency visualization** -- `statik graph` outputs DOT/SVG/HTML for the
   file dependency graph. Interactive HTML viewer for exploring large graphs.

5. **Watch mode** -- `statik watch` monitors file changes and keeps the index
   up-to-date incrementally. Enables near-instant queries after the initial index.

6. **Language Server Protocol (LSP)** -- Expose statik's analysis through LSP for
   integration with any editor. This is a large undertaking and should only be
   considered after the analysis layer is mature.

**Dependencies**: Phases 1-5 for stable analysis. Phase 1 (JSON output) for all
integrations.

**Complexity**: Varies. VS Code extension is Medium. LSP is XL.

**Success Criteria**:
- JSON schema is documented and versioned.
- VS Code extension shows dead code and impact analysis inline.
- GitHub Action runs on a real project's PR pipeline.
- `statik graph` produces a navigable HTML visualization.

**Risks & Mitigations**:
- **Maintenance burden of integrations**: Each integration (VS Code, GitHub Action)
  is a separate project with its own release cycle. Mitigate by keeping integrations
  thin -- they should call the CLI and display results, not contain analysis logic.
- **LSP scope**: LSP is a massive undertaking that competes with language-specific
  servers (tsserver, jdtls). Only pursue if there is clear demand for cross-language
  analysis that existing LSPs don't provide.

---

### Phase 10: Human / Committer Analysis (VCS History Intelligence) — COMPLETE

**Goal**: Add a "people layer" to statik's code graph. Git history records every
human interaction with every file. Combined with the existing dependency graph,
this answers questions no other CLI tool can: "if I change this file, who should
I talk to?", "what's the bus factor of this critical module?", and "which files
change together but have no import relationship?"

**Status**: All items delivered. Core commands (10.1-10.5), dogfooding fixes
(10.4b, 10.4c, 10.7, 10.8, 10.9), impact-aware reviewer suggestion (10.3),
and team boundary analysis (10.6) are complete. External evaluation on a large
multi-module project (~7K files, 130K commits) validated the approach.

**Delivered**:

1. **Git history extraction** (10.1 ✅) -- `statik index --with-history` parses
   `git log --numstat`, stores per-file commit records in SQLite. Incremental
   indexing and `--history-depth` supported. 50K commits indexed in ~80s.

2. **Ownership model** (10.2 ✅) -- `statik owners <glob>` with recency-weighted
   scoring, `--top N`, `--half-life`, `--half-life-mode` flags. Adaptive
   half-life (default) scales with file age so original creators retain
   meaningful ownership.

3. **Impact-aware reviewer suggestion** (10.3 ✅) -- `statik who <file>`
   combines blast radius analysis with ownership data to suggest a minimal
   reviewer set. Runs `statik impact` to find affected files, computes
   ownership for each, aggregates downstream owners, and uses a greedy
   set-cover algorithm to produce a minimal covering set of reviewers.
   Supports `--half-life`, `--half-life-mode`, `--top`, and the global
   `--max-depth` flag for limiting blast radius depth.

4. **Bus factor analysis** (10.4 ✅) -- `statik bus-factor` computes
   knowledge concentration risk. Uses FileId-based lookup for fan_in
   (10.4b fix). Supports `--by-author` for per-person ownership
   concentration view (10.4c).

5. **Change frequency and co-change** (10.5 ✅) -- `statik churn` with
   `--co-change` mode. Found hundreds of hidden couplings on a real project,
   including framework-level coupling invisible to static analysis.

6. **Adaptive ownership half-life** (10.7 ✅) -- `--half-life-mode` flag
   with `adaptive` (default) and `fixed` modes. Adaptive mode scales
   half-life with file age, preventing original creators from decaying to
   zero ownership on old files.

7. **Wildcard import source set boundaries** (10.8 ✅) -- Wildcard imports
   are scoped to source set visibility, preventing false dependency edges
   from production code to test source sets in other modules.

8. **Suppressed unknown language warnings** (10.9 ✅) -- Single summary
   line instead of per-file warnings for files in unsupported languages.

9. **Team boundary analysis** (10.6 ✅) -- `statik team-coupling [glob]`
   maps authors to teams via `[teams]` config in `.statik/rules.toml` (or
   infers teams from email domains). Reports cross-team coordination costs
   per file and identifies dependency edges that cross team boundaries.
   Flags files where >2 teams have >10% ownership each.

**Dependencies**: Existing `impact` command for `statik who`. Existing
`FileGraph` for fan-in data in `bus-factor`. Git repository required (already
detected via `is_git_repo()`).

**Complexity**: Medium overall. The git log parsing is straightforward. The
ownership model is simple math. The value multiplier comes from composing
ownership with the existing dependency graph, which is already built.

**Success Criteria**:
- `statik who src/core/engine.ts` returns meaningful reviewer suggestions in
  under 2 seconds, combining blast radius with ownership data.
- `statik bus-factor --sort risk` correctly identifies single-owner,
  high-fan-in files as the highest organizational risk.
- `statik churn --co-change` identifies file pairs that change together
  without a direct dependency relationship.

**Risks & Mitigations**:
- **Git log performance on large repos**: `git log` on a repo with 100K+
  commits can be slow. Mitigate with `--history-depth` limit and incremental
  indexing (only process new commits).
- **Author identity fragmentation**: The same person may appear under
  different names/emails. For v1, treat each name+email pair as distinct.
  A `.mailmap` integration could be added later if needed.
- **Ownership model subjectivity**: Any weighting scheme is debatable.
  Start with sensible defaults (recency-weighted) and expose the model
  parameters in config if tuning is needed.

---

### Phase 11: SCIP Ingestion (Compiler-Grade Precision Without a Compiler) — IN PROGRESS

**Goal**: Optionally ingest SCIP (Source Code Intelligence Protocol) indexes
from external language servers and compilers. This gives statik fully resolved,
type-aware symbol references without embedding any compiler frontend. Statik
stays fast and lightweight for its own tree-sitter indexing (file-level deps,
architectural lint, CI checks), but can optionally consume richer data for
deep analysis.

**Status**: Core SCIP ingestion is delivered and integrated into all
file-level analyses. The `statik enrich` command reads `.scip` files and
merges compiler-resolved references into the existing SQLite index. SCIP
definitions are matched to tree-sitter symbols by name and line proximity
(11.7) — no duplicate symbol rows are created. SCIP cross-file call
references now create supplemental edges in the file graph (11.8), so
enrichment improves all graph-based commands: `dead-code` (fewer false
positives), `impact` (more complete blast radius), `cycles` (discovers
hidden cycles), `deps` (shows call-based dependencies), `lint`
(catches SCIP-discovered violations), and `who`/`bus-factor` (more
accurate impact radius). SCIP-derived edges are annotated with `[scip]`
in text output and `"is_scip_derived": true` in JSON. Staleness tracking
(per-file mtime vs SCIP timestamp) and confidence upgrades for enriched
data are implemented. Remaining: C++ support via scip-clang (11.4),
cross-language dependency edges (11.5), and large-file benchmarks.

**Key insight from external evaluation**: "The graph algorithms are the hard
and valuable part — the parsing is commodity." Statik's moat is impact
analysis, dead code detection, cycle detection, ownership, and architectural
linting over the dependency graph. SCIP ingestion lets purpose-built indexers
handle the precision problem while statik focuses on graph intelligence.

**Why SCIP**: SCIP is a standardized protobuf format created by Sourcegraph.
Indexers already exist for all languages statik cares about:
- `scip-typescript` — via tsc, fully type-resolved
- `scip-java` — via a Gradle/Maven plugin using JDT
- `rust-analyzer` — emits SCIP natively
- `scip-clang` — via clang, works on Chromium/LLVM-scale codebases

**What it solves**:

1. **Unresolved imports**: An external evaluation on a 7K-file multi-module
   project had 5,000+ unresolved imports with tree-sitter heuristics. SCIP
   resolution would eliminate most of these, upgrading dead code detection
   from "high confidence" to "certain confidence" on many more files.

2. **C++ support**: The roadmap explicitly excludes C++ via tree-sitter (the
   preprocessor makes it unreliable). `scip-clang` sidesteps this entirely —
   statik gets precise C++ dependency data without touching the preprocessor.

3. **Cross-language dependency graphs**: A project with C++ client code and
   Java server code could have a unified dependency graph. `statik impact
   SomeFile.cpp` could trace through the C++ call graph, cross the language
   boundary, and show affected Java files on the server side.

4. **Symbol-level precision**: Tree-sitter can't resolve dynamic dispatch,
   overloaded methods, or generic type parameters. SCIP indexes have fully
   resolved references — `statik impact --symbol SomeClass.someMethod` could
   show the precise 8 files affected by changing that specific method, not the
   29 files that depend on anything in SomeClass.

5. **Wildcard import precision**: Java `import package.*` currently resolves
   heuristically and can cross source set boundaries. SCIP knows exactly which
   symbols are used at compile time.

**Benchmarks** (from external testing on a 2.5M-line multi-module codebase,
~7K Java files + ~7K C++ translation units):

| Operation | Time | Notes |
|-----------|------|-------|
| `statik index` (tree-sitter) | 15s cold, 1.6s incremental | No build needed, works from fresh clone |
| `scip-clang` (C++) | 19s | No build needed — reads `compile_commands.json` directly |
| `scip-java` (Java, cold) | 49s | Requires JDK + Gradle build (35s compile + 14s convert) |
| `scip-java` (Java, incremental) | 25s | 10s compile + 15s convert (no incremental convert) |
| Both SCIP in parallel | ~49s | Wall clock bottlenecked by Java |
| **Total: tree-sitter + both SCIP** | **~64s** | Full precise cross-language analysis |

Index sizes: Java SCIP = 179MB, C++ SCIP = 33MB. These are ephemeral
artifacts (CI-generated, pulled by devs), not committed to git.

Key observation: `scip-clang` is the better UX model — no build needed, no
plugin injection, just point at `compile_commands.json`. `scip-java` requires
instrumenting the build with a `semanticdb` compiler plugin and has no
incremental conversion (always reprocesses all files, ~15s floor).

**Architecture**: Two-tier resolution — tree-sitter is always the fast path,
SCIP is optional enrichment:

```
Fast path (always works, no build, <2s):
  statik index         → tree-sitter       → file-level graph
  statik lint          → 2s                 → architecture rules

Precise path (needs build artifacts, ~25-49s):
  scip-clang ...  &    → reads compile_commands.json
  scip-java ...   &    → instruments Gradle build
  wait
  statik enrich *.scip → merges into SQLite → symbol-level graph
  statik dead-code     → precise            → Certain confidence
  statik impact X.java → method-level       → specific symbol blast radius
```

**Critical design decisions**:

1. **Tree-sitter is always the fast path.** `statik index` and `statik lint`
   never require a build. Tree-sitter resolution is "good enough" for
   architectural lint rules (which are file-level anyway).

2. **SCIP data has a timestamp.** When a file's mtime is newer than the SCIP
   index, statik falls back to tree-sitter resolution for that file. Stale
   SCIP is worse than no SCIP — it gives false confidence.

3. **SCIP is generated in CI, not locally** (by default). The natural place
   is the CI build pipeline — the build is already happening, adding SCIP
   indexing is 30-50% extra time but amortized. CI stores SCIP indexes as
   artifacts; developers pull them. Local analysis gets precision for free
   on files they haven't changed. However, the 49s cold / 25s incremental
   numbers are fast enough that some developers may choose to run SCIP
   locally when they need precision.

4. **All graph-based commands benefit from SCIP.** SCIP cross-file call
   references are added as supplemental edges in the file graph. All
   commands (`dead-code`, `impact`, `cycles`, `deps`, `lint`, `who`,
   `bus-factor`) automatically use SCIP-derived edges when available.

**Deliverables**:

1. **SCIP index reader** (11.1 ✅) — Read `.scip` files using the official
   [`scip` Rust crate](https://crates.io/crates/scip) (v0.6, provides
   protobuf types and utilities). Map SCIP occurrences/symbols to statik's
   existing `SymbolId`, `FileId`, `Reference` types. Implemented in
   `src/scip/mod.rs`. Handles multi-language indexes, symbol kind mapping,
   role classification (definition/reference/import), and name extraction
   from SCIP symbol strings.

2. **`statik enrich` command** (11.2 ✅) — Import one or more SCIP index files
   into the existing database. Uses two-pass symbol matching (11.7): pass 1
   matches SCIP definitions to existing tree-sitter symbols by name + line
   proximity, building a global mapping; pass 2 inserts SCIP references
   remapped to tree-sitter symbol IDs. No duplicate symbol rows are created.
   Previous SCIP data for a file is cleared before re-enrichment (idempotent).
   Stores the SCIP generation timestamp and tool name as DB metadata.

3. **Staleness tracking** (11.3 ✅) — Per-file mtime comparison against the
   SCIP enrichment timestamp. When a file's mtime is newer than the SCIP
   timestamp, that file's SCIP data is considered stale. Staleness is
   surfaced in `statik summary` output ("X files enriched, Y stale,
   Z tree-sitter only").

4. **Cross-language edge creation** — When multiple SCIP indexes are imported
   (e.g., one from `scip-java`, one from `scip-clang`), create cross-language
   edges based on shared symbol names or configured boundary mappings.

5. **C++ support via scip-clang** — With SCIP ingestion working, C++ is
   automatically supported. No tree-sitter C++ parser needed. Add `Language::Cpp`
   variant and ensure file discovery handles `.cpp`, `.h`, `.hpp`.

6. **Confidence upgrade** (11.6 ✅) — When SCIP data is available for a file,
   SCIP-enriched files with unresolved tree-sitter imports no longer reduce
   confidence. Dead code detection and impact analysis both check the
   `scip_enriched` set on `FileGraph` and upgrade confidence to Certain
   when all unresolved imports come from enriched files.

7. **Symbol deduplication** (11.7 ✅) — SCIP enrichment no longer inserts
   duplicate symbol rows. Two-pass approach: pass 1 matches each SCIP
   definition to the closest tree-sitter symbol (by name + line proximity
   within the same file), building a global `scip_symbol_string → SymbolId`
   map; pass 2 inserts SCIP references remapped to tree-sitter IDs.
   Eliminates the 5→583 dead symbol inflation observed during dogfooding.

8. **SCIP cross-file refs in FileGraph** (11.8 ✅) — SCIP cross-file call
   references are now queried from the `refs` table and added as
   supplemental edges in `build_file_graph()`. Edges are deduplicated
   against existing import-based edges and marked with `is_scip_derived`.
   All file-level analyses (`dead-code`, `impact`, `cycles`, `deps`,
   `lint`, `who`, `bus-factor`) automatically benefit. SCIP-derived edges
   are annotated with `[scip]` in text output and `"is_scip_derived": true`
   in JSON output.

**Dependencies**: None — SCIP ingestion can be built alongside any other phase.
It only needs the existing database schema and graph infrastructure.

**Complexity**: Medium. The SCIP protobuf format is well-documented. The main
work is mapping SCIP's symbol scheme to statik's existing types and handling
the merge logic. Cross-language edges are the hardest part.

**Open questions**:

1. **SCIP ingestion time** — How fast can statik read 179MB + 33MB of SCIP
   data and merge it into SQLite? This determines whether the workflow feels
   smooth. Target: <5s for the merge step.
2. **Index storage** — 212MB total of SCIP indexes. Store in `.statik/`?
   The SQLite DB after ingestion may be much smaller if statik only stores
   resolved edges, not the full SCIP data.
3. **Incremental SCIP conversion** — `scip-java`'s `index-semanticdb` has no
   incremental mode (~15s floor even for 1-file changes). Worth filing as
   feedback with Sourcegraph.

**Success Criteria**:
- `statik enrich java-index.scip` imports a SCIP index and upgrades reference
  precision. Unresolved imports drop by >90%.
- `statik dead-code` after enrichment reports more dead code at higher
  confidence.
- C++ files indexed via `scip-clang` appear in the dependency graph alongside
  Java/TS files.
- All existing commands work unchanged on enriched data.
- `statik index` (tree-sitter only) performance is unaffected — the fast path
  never regresses.
- Stale SCIP data (file edited after SCIP generation) falls back to
  tree-sitter automatically.

**Risks & Mitigations**:
- **SCIP index generation adds build time**: Benchmarked at 19s for C++
  (no build needed) and 49s for Java (requires build). Mitigate: run SCIP
  indexing in CI and distribute artifacts. The 25s incremental Java number
  is also acceptable for local use.
- **SCIP format stability**: SCIP is versioned and maintained by Sourcegraph.
  Pin the protobuf version and update deliberately.
- **Large SCIP index files**: 179MB for Java on a 7K-file project. Mitigate:
  store as CI artifacts, not in git. Statik extracts only resolved edges
  into SQLite, not the full SCIP payload.
- **Cross-language edge heuristics**: Matching symbols across language
  boundaries requires project-specific configuration. Provide a
  `[cross_language]` config section for custom mappings rather than
  attempting automatic detection.
- **Java 25 not yet supported by scip-java**: The `semanticdb-javac` plugin
  requires JDK 21 or earlier. Monitor Sourcegraph releases for updates.

---

## Risk Summary

| Risk | Severity | Mitigation |
|------|----------|------------|
| Rule DSL becomes a language | High | Start with globs only. No predicate language in v1. Extend to regex if needed. |
| Multi-language fragments focus | Critical | Java only in Phase 3. No C++ via tree-sitter. C++ possible via SCIP ingestion (Phase 11). |
| Semantic diff is research-grade | Critical | Structural diff instead. Compare export surfaces, not AST trees. |
| SQLite won't scale for large codebases | High | Lazy loading in Phase 1. Streaming queries before adding languages. |
| Competing in a saturated market | High | Architectural linting + confidence scores are unique differentiators. |
| JDT-like depth exceeds tree-sitter capabilities | Medium | Define "deep" as symbol-level, not type-level. Tree-sitter is a parser, not a type checker. SCIP ingestion (Phase 11) provides type-resolved data without embedding a compiler. |
| Java resolver is 70% of the effort | High | Start with convention-based resolution. Treat classpath imports as External. |
| Integration maintenance burden | Medium | Keep integrations thin. CLI is the source of truth. |
| Git log slow on huge repos | Medium | Incremental indexing + `--history-depth` limit. Opt-in via `--with-history`. Confirmed: 50K commits in ~80s is acceptable. |
| Author identity fragmentation | Low | Treat name+email as identity for v1. `.mailmap` support later if needed. |
| Ownership recency overweighting | ~~Medium~~ Resolved | ~~Fixed 180-day half-life makes old file creators invisible.~~ Adaptive half-life mode implemented (default). Scales with file age. |
| Cross-subsystem path formats | ~~Medium~~ Resolved | ~~File graph uses absolute paths, git history uses relative. Caused bus-factor fan_in bug.~~ Fixed: uses FileId-based joins instead of string matching. |

---

## What This Roadmap Explicitly Does NOT Include

- **C++ support via tree-sitter**: The preprocessor makes this unreliable.
  C++ support is planned via SCIP ingestion (Phase 11) using `scip-clang`,
  which leverages clang's own AST rather than tree-sitter's pre-preprocessed view.
- **Full semantic diff**: Tree matching (GumTree-style) is an active research area.
  We do structural diff instead.
- **Embedding compilers (JDT, libclang, tsc)**: statik does not run compilers.
  Instead, Phase 11 (SCIP ingestion) consumes compiler output via the
  standardized SCIP format. The heavy compilation happens externally (during
  normal builds); statik reads the result in seconds.
- **node_modules resolution**: Documented as a limitation. External packages are
  identified but not resolved into node_modules.
- **Python parser**: Discovery supports Python but a parser is not planned. Focus
  remains on TS/JS, Java, and Rust.
- **Content-level linting**: statik's architectural linting operates on the
  dependency graph (file-to-file edges, import metadata). It does not lint code
  style, naming conventions, or AST patterns within a file -- that's ESLint/Clippy
  territory. statik lints structure, not syntax.
