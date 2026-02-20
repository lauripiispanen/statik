# statik Roadmap

## Vision

statik aims to be the fastest, most precise CLI-first dependency analysis tool for
typed languages -- starting with TypeScript/JavaScript and expanding to Java. It
prioritizes correctness over completeness, reporting confidence levels rather than
guessing, and ships as a single binary with zero runtime dependencies.

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
extend to Java with a clear-eyed view of where the effort actually lives (resolvers,
not parsers).

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

## Risk Summary

| Risk | Severity | Mitigation |
|------|----------|------------|
| Rule DSL becomes a language | High | Start with globs only. No predicate language in v1. Extend to regex if needed. |
| Multi-language fragments focus | Critical | Java only in Phase 3. No C++ via tree-sitter. |
| Semantic diff is research-grade | Critical | Structural diff instead. Compare export surfaces, not AST trees. |
| SQLite won't scale for large codebases | High | Lazy loading in Phase 1. Streaming queries before adding languages. |
| Competing in a saturated market | High | Architectural linting + confidence scores are unique differentiators. |
| JDT-like depth exceeds tree-sitter capabilities | Medium | Define "deep" as symbol-level, not type-level. Tree-sitter is a parser, not a type checker. |
| Java resolver is 70% of the effort | High | Start with convention-based resolution. Treat classpath imports as External. |
| Integration maintenance burden | Medium | Keep integrations thin. CLI is the source of truth. |

---

## What This Roadmap Explicitly Does NOT Include

- **C++ support via tree-sitter**: The preprocessor makes this unreliable. If C++
  is revisited, it should be via libclang, which is a different architecture.
- **Full semantic diff**: Tree matching (GumTree-style) is an active research area.
  We do structural diff instead.
- **Type inference or type checking**: statik is a parser-based tool. If it requires
  running tsc or javac, it's out of scope.
- **node_modules resolution**: Documented as a limitation. External packages are
  identified but not resolved into node_modules.
- **Python or Rust parsers**: Discovery supports these languages but parsers are not
  planned. Focus on TS/JS and Java.
- **Content-level linting**: statik's architectural linting operates on the
  dependency graph (file-to-file edges, import metadata). It does not lint code
  style, naming conventions, or AST patterns within a file -- that's ESLint/Clippy
  territory. statik lints structure, not syntax.
