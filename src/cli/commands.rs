use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::analysis::cycles::detect_cycles;
use crate::analysis::dead_code::{detect_dead_code, DeadCodeScope};
use crate::analysis::dependencies::{analyze_deps, Direction};
use crate::analysis::impact::analyze_impact;
use crate::db::Database;
use crate::model::file_graph::{
    FileGraph, FileImport, FileInfo, UnresolvedImport, UnresolvedReason,
};
use crate::model::graph::SymbolGraph;
use crate::model::{FileId, Language, ParseResult, RefKind, SymbolKind};
use crate::resolver::java::JavaResolver;
use crate::resolver::rust::RustResolver;
use crate::resolver::typescript::TypeScriptResolver;
use crate::resolver::{Resolution, Resolver};

use super::OutputFormat;

/// Build a FileGraph from the database and resolver.
pub fn build_file_graph(db: &Database, project_root: &Path) -> Result<FileGraph> {
    let files = db.all_files()?;
    let mut graph = FileGraph::new();

    // Load user-configured entry points
    let ep_config = crate::linting::config::load_entry_point_config(project_root);
    let custom_pattern_matcher = if ep_config.patterns.is_empty() {
        None
    } else {
        Some(crate::linting::matcher::FileMatcher::new(
            &ep_config.patterns,
        )?)
    };

    // Build language semantics for entry point detection
    let registry = crate::parser::ParserRegistry::with_defaults();
    let semantics = registry.semantics_map();

    // Collect all known file paths for resolvers
    let known_paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
    let ts_resolver = TypeScriptResolver::new_auto(project_root.to_path_buf(), known_paths.clone());
    let java_config = crate::linting::config::load_java_config(project_root);
    let mut java_resolver = JavaResolver::new(
        project_root.to_path_buf(),
        known_paths.clone(),
        java_config.map(|c| c.source_roots),
    );
    let rust_resolver = RustResolver::new(project_root.to_path_buf(), known_paths);

    // Build path -> FileId lookup
    let path_to_id: HashMap<PathBuf, FileId> =
        files.iter().map(|f| (f.path.clone(), f.id)).collect();

    // Batch-load all imports and exports (3 queries total instead of 2N+1)
    let all_imports = db.all_imports()?;
    let all_exports = db.all_exports()?;

    let mut imports_by_file: HashMap<FileId, Vec<crate::model::ImportRecord>> = HashMap::new();
    for imp in all_imports {
        imports_by_file.entry(imp.file).or_default().push(imp);
    }

    let mut exports_by_file: HashMap<FileId, Vec<crate::model::ExportRecord>> = HashMap::new();
    for exp in all_exports {
        exports_by_file.entry(exp.file).or_default().push(exp);
    }

    // Pre-scan all files for annotation/attribute-based entry points.
    // Each language defines its own entry point annotations via LanguageSemantics.
    // Parsers emit @annotation: synthetic imports which we check here.
    let mut annotation_entry_files: std::collections::HashSet<FileId> =
        std::collections::HashSet::new();
    for file in &files {
        if let Some(imports) = imports_by_file.get(&file.id) {
            let lang_annotations = semantics
                .get(&file.language)
                .map(|s| s.entry_point_annotations())
                .unwrap_or(&[]);
            for import in imports {
                if let Some(ann) = import.source_path.strip_prefix("@annotation:") {
                    if lang_annotations.iter().any(|a| *a == ann)
                        || ep_config.annotations.iter().any(|a| a == ann)
                    {
                        annotation_entry_files.insert(file.id);
                        break;
                    }
                }
            }
        }
    }

    // Add files to the graph
    for file in &files {
        let exports = exports_by_file.remove(&file.id).unwrap_or_default();
        let rel_path = crate::linting::matcher::to_relative(&file.path, project_root);
        let lang_sem = semantics.get(&file.language).copied();
        let is_entry = is_entry_point(&file.path, lang_sem)
            || annotation_entry_files.contains(&file.id)
            || custom_pattern_matcher
                .as_ref()
                .is_some_and(|m| m.matches(rel_path));

        graph.add_file(FileInfo {
            id: file.id,
            path: file.path.clone(),
            language: file.language,
            exports,
            is_entry_point: is_entry,
        });
    }

    // Build file language lookup for resolver dispatch
    let file_language: HashMap<FileId, Language> =
        files.iter().map(|f| (f.id, f.language)).collect();

    // Resolve imports and add edges
    for file in &files {
        let imports = imports_by_file.remove(&file.id).unwrap_or_default();

        // Set up per-file wildcard context for Java type-ref resolution
        if file.language == Language::Java {
            java_resolver.set_file_wildcards(&imports);
        }

        // Group imports by target file, tracking metadata per import
        // Tuple: (name, is_type_only, line, is_mod_declaration)
        let mut edges_by_target: HashMap<FileId, Vec<(String, bool, usize, bool)>> = HashMap::new();

        for import in &imports {
            // Skip annotation marker imports (handled during entry point detection)
            if import.source_path.starts_with("@annotation:") {
                continue;
            }

            let is_mod = import.source_path.starts_with("@mod:");

            let lang = file_language
                .get(&file.id)
                .copied()
                .unwrap_or(Language::TypeScript);
            let resolution: Resolution =
                if let Some(type_name) = import.source_path.strip_prefix("@type-ref:") {
                    java_resolver.resolve_type_ref(type_name, &file.path)
                } else if import.is_namespace && lang == Language::Java {
                    // Wildcard import: resolve to all files in the package
                    let files = java_resolver.resolve_wildcard(&import.source_path);
                    if files.is_empty() {
                        if JavaResolver::is_likely_external(&import.source_path) {
                            let pkg = import
                                .source_path
                                .split('.')
                                .take(3)
                                .collect::<Vec<_>>()
                                .join(".");
                            Resolution::External(pkg)
                        } else {
                            Resolution::External(import.source_path.clone())
                        }
                    } else {
                        for resolved_path in &files {
                            if let Some(&target_id) = path_to_id.get(resolved_path) {
                                if target_id != file.id {
                                    edges_by_target.entry(target_id).or_default().push((
                                        "*".to_string(),
                                        import.is_type_only,
                                        import.line_span.start.line,
                                        false,
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                } else {
                    match lang {
                        Language::Java => java_resolver.resolve(&import.source_path, &file.path),
                        Language::Rust => rust_resolver.resolve(&import.source_path, &file.path),
                        _ => ts_resolver.resolve(&import.source_path, &file.path),
                    }
                };

            match resolution {
                Resolution::Resolved(resolved_path)
                | Resolution::ResolvedWithCaveat(resolved_path, _) => {
                    if let Some(&target_id) = path_to_id.get(&resolved_path) {
                        edges_by_target.entry(target_id).or_default().push((
                            import.imported_name.clone(),
                            import.is_type_only,
                            import.line_span.start.line,
                            is_mod,
                        ));
                    }
                }
                Resolution::External(pkg) => {
                    graph.add_unresolved(UnresolvedImport {
                        file: file.id,
                        import_path: import.source_path.clone(),
                        reason: UnresolvedReason::External(pkg),
                        line: import.line_span.start.line,
                    });
                }
                Resolution::Unresolved(reason) => {
                    let reason = match reason {
                        crate::resolver::UnresolvedReason::DynamicPath => {
                            UnresolvedReason::DynamicPath
                        }
                        crate::resolver::UnresolvedReason::FileNotFound(s) => {
                            UnresolvedReason::FileNotFound(s)
                        }
                        crate::resolver::UnresolvedReason::NodeModules => {
                            UnresolvedReason::External(import.source_path.clone())
                        }
                        crate::resolver::UnresolvedReason::UnsupportedSyntax(s) => {
                            UnresolvedReason::FileNotFound(s)
                        }
                    };
                    graph.add_unresolved(UnresolvedImport {
                        file: file.id,
                        import_path: import.source_path.clone(),
                        reason,
                        line: import.line_span.start.line,
                    });
                }
            }
        }

        // Create edges
        for (target_id, imports_meta) in edges_by_target {
            let names: Vec<String> = imports_meta.iter().map(|(n, _, _, _)| n.clone()).collect();
            // Edge is type-only only if ALL grouped imports are type-only
            let is_type_only = imports_meta.iter().all(|(_, t, _, _)| *t);
            // Edge is mod-declaration if ANY grouped import is a mod declaration
            let is_mod_declaration = imports_meta.iter().any(|(_, _, _, m)| *m);
            // Use the earliest line number
            let line = imports_meta.iter().map(|(_, _, l, _)| *l).min().unwrap_or(0);
            graph.add_import(FileImport {
                from: file.id,
                to: target_id,
                imported_names: names,
                is_type_only,
                is_mod_declaration,
                line,
            });
        }
    }

    Ok(graph)
}

/// Build a SymbolGraph from the database (symbols + resolved references).
fn build_symbol_graph(db: &Database) -> Result<SymbolGraph> {
    let mut graph = SymbolGraph::new();

    let all_symbols = db.all_symbols()?;
    let all_refs = db.all_references()?;
    let all_files = db.all_files()?;
    let all_exports = db.all_exports()?;

    for file in &all_files {
        graph.add_file(file.clone());
    }

    // Build a set of valid symbol IDs for filtering unresolved references
    let valid_ids: std::collections::HashSet<crate::model::SymbolId> =
        all_symbols.iter().map(|s| s.id).collect();

    // Group symbols and references by file for add_parse_result
    let mut file_symbols: HashMap<FileId, Vec<crate::model::Symbol>> = HashMap::new();
    for sym in all_symbols {
        file_symbols.entry(sym.file).or_default().push(sym);
    }

    // Only keep references where both source and target are resolved
    let mut file_refs: HashMap<FileId, Vec<crate::model::Reference>> = HashMap::new();
    for r in all_refs {
        if valid_ids.contains(&r.source) && valid_ids.contains(&r.target) {
            file_refs.entry(r.file).or_default().push(r);
        }
    }

    // Group exports by file
    let mut file_exports: HashMap<FileId, Vec<crate::model::ExportRecord>> = HashMap::new();
    for export in all_exports {
        file_exports.entry(export.file).or_default().push(export);
    }

    for file in &all_files {
        let symbols = file_symbols.remove(&file.id).unwrap_or_default();
        let references = file_refs.remove(&file.id).unwrap_or_default();
        let exports = file_exports.remove(&file.id).unwrap_or_default();
        if !symbols.is_empty() || !references.is_empty() || !exports.is_empty() {
            graph.add_parse_result(ParseResult {
                file_id: file.id,
                symbols,
                references,
                imports: vec![],
                exports,
                type_references: vec![],
                annotations: vec![],
            });
        }
    }

    Ok(graph)
}

/// Check if a file is an entry point.
///
/// Universal patterns (index, main, app, server, cli) are checked first.
/// Language-specific patterns are delegated to `LanguageSemantics`.
fn is_entry_point(
    path: &Path,
    semantics: Option<&dyn crate::model::LanguageSemantics>,
) -> bool {
    let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    // Universal entry point patterns
    let entry_patterns = ["index", "main", "app", "server", "cli"];
    for pattern in &entry_patterns {
        if file_name == *pattern {
            return true;
        }
    }

    // Language-specific entry point detection
    if let Some(sem) = semantics {
        if sem.is_entry_point_file(path) {
            return true;
        }
    }

    false
}

/// Build the set of file IDs where ALL symbols should be seeded as alive.
///
/// Combines two sources:
/// 1. Language defaults: files under directories named in `seed_all_symbols_dirs()`
///    (e.g., `tests/`, `examples/`, `benches/` for Rust, `test/` for Java)
/// 2. User config: files matching `entry_points.seed_all_patterns` globs
fn build_seed_all_file_ids(file_graph: &FileGraph, project_root: &Path) -> HashSet<FileId> {
    let registry = crate::parser::ParserRegistry::with_defaults();
    let semantics = registry.semantics_map();
    let ep_config = crate::linting::config::load_entry_point_config(project_root);

    let config_matcher = if ep_config.always_alive.is_empty() {
        None
    } else {
        crate::linting::matcher::FileMatcher::new(&ep_config.always_alive).ok()
    };

    let mut seed_all = HashSet::new();
    for (file_id, info) in &file_graph.files {
        let rel_path = crate::linting::matcher::to_relative(&info.path, project_root);

        // Check language-specific seed-all directories
        if let Some(sem) = semantics.get(&info.language) {
            let dirs = sem.seed_all_symbols_dirs();
            if !dirs.is_empty()
                && info.path.components().any(|c| {
                    dirs.iter()
                        .any(|d| c.as_os_str().to_str().is_some_and(|s| s == *d))
                })
            {
                seed_all.insert(*file_id);
                continue;
            }
        }

        // Check user-configured seed-all patterns
        if let Some(ref matcher) = config_matcher {
            if matcher.matches(rel_path) {
                seed_all.insert(*file_id);
            }
        }
    }
    seed_all
}

/// Ensure index exists, creating it if needed.
pub fn ensure_index(project_path: &Path, no_index: bool) -> Result<Database> {
    let statik_dir = project_path.join(".statik");
    let db_path = statik_dir.join("index.db");

    if !db_path.exists() {
        if no_index {
            anyhow::bail!(
                "No index found at {}. Run `statik index` first, or remove --no-index.",
                db_path.display()
            );
        }
        // Auto-index
        eprintln!("No index found. Running auto-index...");
        let config = crate::discovery::DiscoveryConfig::default();
        let result = crate::cli::index::run_index(project_path, &config)?;
        eprintln!(
            "Indexed {} files ({} symbols) in {}ms",
            result.files_indexed + result.files_unchanged,
            result.symbols_extracted,
            result.duration_ms,
        );
    }

    Database::open(&db_path)
}

/// Apply --runtime-only filtering if requested.
fn maybe_filter_type_only(graph: FileGraph, runtime_only: bool) -> FileGraph {
    if runtime_only {
        graph.without_type_only_edges()
    } else {
        graph
    }
}

/// Apply --path glob filtering if requested.
fn maybe_filter_paths(
    graph: FileGraph,
    path_glob: Option<&str>,
    project_root: &Path,
) -> Result<FileGraph> {
    match path_glob {
        Some(pattern) => {
            let glob = globset::Glob::new(pattern)
                .with_context(|| format!("Invalid glob pattern: {}", pattern))?
                .compile_matcher();
            Ok(graph.filter_to_paths(&glob, project_root))
        }
        None => Ok(graph),
    }
}

/// Run the `deps` command.
#[allow(clippy::too_many_arguments)]
pub fn run_deps(
    project_path: &Path,
    file_path: &str,
    transitive: bool,
    direction_str: &str,
    max_depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let direction = match direction_str {
        "in" => Direction::ImportedBy,
        "out" => Direction::Imports,
        _ => Direction::Both,
    };

    // Resolve file path to FileId
    let abs_path = project_path.join(file_path);
    let target_id = graph
        .file_by_path(&abs_path)
        .or_else(|| {
            // Try matching by suffix
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(file_path))
                .map(|f| f.id)
        })
        .context(format!("File not found in index: {}", file_path))?;

    let result = analyze_deps(&graph, target_id, direction, transitive, max_depth)
        .context("Failed to analyze dependencies")?;

    Ok(match format {
        OutputFormat::Text => format_deps_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run the `deps --between` command: list all edges where source matches from_glob
/// and target matches to_glob.
#[allow(clippy::too_many_arguments)]
pub fn run_deps_between(
    project_path: &Path,
    from_glob: &str,
    to_glob: &str,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let from_matcher = globset::Glob::new(from_glob)
        .with_context(|| format!("Invalid from glob: {}", from_glob))?
        .compile_matcher();
    let to_matcher = globset::Glob::new(to_glob)
        .with_context(|| format!("Invalid to glob: {}", to_glob))?
        .compile_matcher();

    #[derive(serde::Serialize)]
    struct BetweenEdge {
        from: String,
        to: String,
        imported_names: Vec<String>,
        is_type_only: bool,
        line: usize,
    }

    #[derive(serde::Serialize)]
    struct BetweenResult {
        command: String,
        from_glob: String,
        to_glob: String,
        edges: Vec<BetweenEdge>,
        count: usize,
    }

    let mut edges = Vec::new();
    for file_edges in graph.imports.values() {
        for edge in file_edges {
            let from_info = match graph.files.get(&edge.from) {
                Some(f) => f,
                None => continue,
            };
            let to_info = match graph.files.get(&edge.to) {
                Some(f) => f,
                None => continue,
            };
            let from_rel = from_info
                .path
                .strip_prefix(project_path)
                .unwrap_or(&from_info.path);
            let to_rel = to_info
                .path
                .strip_prefix(project_path)
                .unwrap_or(&to_info.path);
            if from_matcher.is_match(from_rel) && to_matcher.is_match(to_rel) {
                edges.push(BetweenEdge {
                    from: display_path(&from_info.path),
                    to: display_path(&to_info.path),
                    imported_names: edge.imported_names.clone(),
                    is_type_only: edge.is_type_only,
                    line: edge.line,
                });
            }
        }
    }

    let count = edges.len();
    let result = BetweenResult {
        command: "deps-between".to_string(),
        from_glob: from_glob.to_string(),
        to_glob: to_glob.to_string(),
        edges,
        count,
    };

    Ok(match format {
        OutputFormat::Text => {
            let mut out = String::new();
            out.push_str(&format!(
                "Dependencies from '{}' to '{}' ({}):\n\n",
                from_glob, to_glob, count
            ));
            if result.edges.is_empty() {
                out.push_str("No matching edges found.\n");
            } else {
                for e in &result.edges {
                    let names = if e.imported_names.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", e.imported_names.join(", "))
                    };
                    let type_label = if e.is_type_only { " [type-only]" } else { "" };
                    out.push_str(&format!(
                        "  {} -> {}{}{}\n",
                        e.from, e.to, names, type_label
                    ));
                }
            }
            out
        }
        _ => format_json(&result, format),
    })
}

/// Run the `dead-code` command.
#[allow(clippy::too_many_arguments)]
pub fn run_dead_code(
    project_path: &Path,
    scope_str: &str,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
    lang: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    if scope_str == "symbols" {
        // Symbol-level dead code analysis
        let file_graph = build_file_graph(&db, project_path)?;
        let file_graph = maybe_filter_paths(file_graph, path_glob, project_path)?;
        let symbol_graph = build_symbol_graph(&db)?;
        let linker_result = crate::analysis::linker::link_cross_file_symbols(&file_graph);

        // Build the set of files where ALL symbols should be seeded as alive.
        // This combines language-specific test directories and user config patterns.
        let seed_all_file_ids = build_seed_all_file_ids(&file_graph, project_path);

        let mut result = crate::analysis::dead_code::detect_dead_symbols(
            &symbol_graph,
            &file_graph,
            &linker_result,
            &seed_all_file_ids,
        );
        if let Some(lang_filter) = lang.and_then(lang_str_to_language) {
            result.dead_symbols.retain(|s| {
                Path::new(&s.file)
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(Language::from_extension)
                    == Some(lang_filter)
            });
            result.summary.dead_symbols = result.dead_symbols.len();
        }
        return Ok(match format {
            OutputFormat::Text => format_dead_symbols_text(&result),
            _ => format_json(&result, format),
        });
    }

    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let scope = match scope_str {
        "files" => DeadCodeScope::Files,
        "exports" => DeadCodeScope::Exports,
        _ => DeadCodeScope::Both,
    };

    let seed_all_file_ids = build_seed_all_file_ids(&graph, project_path);
    let mut result = detect_dead_code(&graph, scope, &seed_all_file_ids);
    if let Some(lang_filter) = lang.and_then(lang_str_to_language) {
        result.dead_files.retain(|f| {
            f.path
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Language::from_extension)
                == Some(lang_filter)
        });
        result.dead_exports.retain(|e| {
            e.path
                .extension()
                .and_then(|ext| ext.to_str())
                .and_then(Language::from_extension)
                == Some(lang_filter)
        });
        result.summary.dead_files = result.dead_files.len();
        result.summary.dead_exports = result.dead_exports.len();
    }
    Ok(match format {
        OutputFormat::Text => format_dead_code_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run the `cycles` command.
pub fn run_cycles(
    project_path: &Path,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = graph.without_mod_declaration_edges();

    let result = detect_cycles(&graph);
    Ok(match format {
        OutputFormat::Text => format_cycles_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run the `impact` command.
#[allow(clippy::too_many_arguments)]
pub fn run_impact(
    project_path: &Path,
    file_path: &str,
    max_depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let abs_path = project_path.join(file_path);
    let target_id = graph
        .file_by_path(&abs_path)
        .or_else(|| {
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(file_path))
                .map(|f| f.id)
        })
        .context(format!("File not found in index: {}", file_path))?;

    let result =
        analyze_impact(&graph, target_id, max_depth).context("Failed to analyze impact")?;

    Ok(match format {
        OutputFormat::Text => format_impact_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run the `exports` command.
pub fn run_exports(
    project_path: &Path,
    file_path: &str,
    format: &OutputFormat,
    no_index: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let abs_path = project_path.join(file_path);
    let target_id = graph
        .file_by_path(&abs_path)
        .or_else(|| {
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(file_path))
                .map(|f| f.id)
        })
        .context(format!("File not found in index: {}", file_path))?;

    let file_info = graph.files.get(&target_id).unwrap();

    // Check which exports are used
    let mut used_exports = std::collections::HashSet::new();
    let target_file_stem = file_info
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    for edges in graph.imports.values() {
        for edge in edges {
            if edge.to == target_id {
                for name in &edge.imported_names {
                    used_exports.insert(name.clone());
                }
                // Rust module-path import: if imported name matches the file stem,
                // all exports are considered used (e.g. `use crate::cli::commands`)
                if file_info.language == Language::Rust
                    && !target_file_stem.is_empty()
                    && edge.imported_names.iter().any(|n| n == target_file_stem)
                {
                    for exp in &file_info.exports {
                        used_exports.insert(exp.exported_name.clone());
                    }
                }
            }
        }
    }

    #[derive(serde::Serialize)]
    struct ExportInfo {
        name: String,
        is_default: bool,
        is_reexport: bool,
        is_used: bool,
    }

    let exports: Vec<ExportInfo> = file_info
        .exports
        .iter()
        .map(|e| ExportInfo {
            name: e.exported_name.clone(),
            is_default: e.is_default,
            is_reexport: e.is_reexport,
            is_used: used_exports.contains(&e.exported_name)
                || (e.is_default && used_exports.contains("default")),
        })
        .collect();

    #[derive(serde::Serialize)]
    struct ExportsResult {
        command: String,
        tier: String,
        file: String,
        exports: Vec<ExportInfo>,
        summary: ExportsSummary,
    }

    #[derive(serde::Serialize)]
    struct ExportsSummary {
        total: usize,
        used: usize,
        unused: usize,
    }

    let used_count = exports.iter().filter(|e| e.is_used).count();
    let result = ExportsResult {
        command: "exports".to_string(),
        tier: "general".to_string(),
        file: file_path.to_string(),
        summary: ExportsSummary {
            total: exports.len(),
            used: used_count,
            unused: exports.len() - used_count,
        },
        exports,
    };

    Ok(match format {
        OutputFormat::Text => {
            let value = serde_json::to_value(&result).unwrap_or_default();
            format_exports_text(&value)
        }
        _ => format_json(&result, format),
    })
}

/// Run the `summary` command.
pub fn run_summary(
    project_path: &Path,
    format: &OutputFormat,
    no_index: bool,
    path_glob: Option<&str>,
    by_directory: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    if by_directory {
        return run_summary_by_directory(&graph, project_path, format);
    }

    let seed_all_file_ids = build_seed_all_file_ids(&graph, project_path);
    let dead = detect_dead_code(&graph, DeadCodeScope::Both, &seed_all_file_ids);
    let graph_no_mod = graph.without_mod_declaration_edges();
    let cycles = detect_cycles(&graph_no_mod);

    // Count files by language
    let mut by_language: HashMap<String, usize> = HashMap::new();
    for file in graph.files.values() {
        *by_language.entry(file.language.to_string()).or_default() += 1;
    }

    let total_exports: usize = graph.files.values().map(|f| f.exports.len()).sum();
    let total_imports: usize = graph.imports.values().map(|v| v.len()).sum();

    #[derive(serde::Serialize)]
    struct SummaryResult {
        command: String,
        tier: String,
        files: FileSummary,
        dependencies: DepSummary,
        dead_code: DeadCodeSummaryCompact,
        cycles: CycleSummaryCompact,
    }

    #[derive(serde::Serialize)]
    struct FileSummary {
        total: usize,
        by_language: HashMap<String, usize>,
        entry_points: usize,
    }

    #[derive(serde::Serialize)]
    struct DepSummary {
        total_imports: usize,
        external_imports: usize,
        unresolved_imports: usize,
    }

    #[derive(serde::Serialize)]
    struct DeadCodeSummaryCompact {
        dead_files: usize,
        dead_exports: usize,
        total_exports: usize,
    }

    #[derive(serde::Serialize)]
    struct CycleSummaryCompact {
        cycle_count: usize,
        files_in_cycles: usize,
    }

    let result = SummaryResult {
        command: "summary".to_string(),
        tier: "general".to_string(),
        files: FileSummary {
            total: graph.file_count(),
            by_language,
            entry_points: graph.entry_points().len(),
        },
        dependencies: DepSummary {
            total_imports,
            external_imports: graph
                .unresolved
                .iter()
                .filter(|u| matches!(u.reason, UnresolvedReason::External(_)))
                .count(),
            unresolved_imports: graph
                .unresolved
                .iter()
                .filter(|u| {
                    matches!(
                        u.reason,
                        UnresolvedReason::FileNotFound(_) | UnresolvedReason::DynamicPath
                    )
                })
                .count(),
        },
        dead_code: DeadCodeSummaryCompact {
            dead_files: dead.dead_files.len(),
            dead_exports: dead.dead_exports.len(),
            total_exports,
        },
        cycles: CycleSummaryCompact {
            cycle_count: cycles.cycles.len(),
            files_in_cycles: cycles.summary.files_in_cycles,
        },
    };

    Ok(match format {
        OutputFormat::Text => {
            let value = serde_json::to_value(&result).unwrap_or_default();
            format_summary_text(&value)
        }
        _ => format_json(&result, format),
    })
}

/// Run the `summary --by-directory` command: aggregate stats per directory.
fn run_summary_by_directory(
    graph: &crate::model::file_graph::FileGraph,
    project_root: &Path,
    format: &OutputFormat,
) -> Result<String> {
    use std::collections::HashSet;

    let seed_all_file_ids = build_seed_all_file_ids(graph, project_root);
    let dead = detect_dead_code(graph, DeadCodeScope::Both, &seed_all_file_ids);

    // Build set of dead export keys for quick lookup
    let dead_export_keys: HashSet<(FileId, String)> = dead
        .dead_exports
        .iter()
        .map(|e| (e.file_id, e.export_name.clone()))
        .collect();

    // Build file -> directory mapping using relative paths
    let file_dir: HashMap<FileId, String> = graph
        .files
        .iter()
        .map(|(id, info)| {
            let rel = info.path.strip_prefix(project_root).unwrap_or(&info.path);
            let dir = rel
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            (*id, if dir.is_empty() { ".".to_string() } else { dir })
        })
        .collect();

    // Aggregate per directory
    #[derive(Default)]
    struct DirStats {
        files: usize,
        exports: usize,
        dead_exports: usize,
        fan_out_sum: usize,
        fan_in_sum: usize,
    }

    let mut dir_stats: HashMap<String, DirStats> = HashMap::new();

    for (file_id, info) in &graph.files {
        let dir = file_dir.get(file_id).unwrap();
        let stats = dir_stats.entry(dir.clone()).or_default();
        stats.files += 1;
        stats.exports += info.exports.len();

        // Count dead exports for this file
        for exp in &info.exports {
            if dead_export_keys.contains(&(*file_id, exp.exported_name.clone())) {
                stats.dead_exports += 1;
            }
        }

        // Fan-out: number of distinct files this file imports
        let fan_out = graph.direct_imports(*file_id).len();
        stats.fan_out_sum += fan_out;

        // Fan-in: number of distinct files that import this file
        let fan_in = graph.direct_importers(*file_id).len();
        stats.fan_in_sum += fan_in;
    }

    #[derive(serde::Serialize)]
    struct DirSummaryEntry {
        directory: String,
        files: usize,
        exports: usize,
        dead_exports: usize,
        avg_fan_out: f64,
        avg_fan_in: f64,
    }

    let mut directories: Vec<DirSummaryEntry> = dir_stats
        .into_iter()
        .map(|(dir, stats)| {
            let avg_fan_out = if stats.files > 0 {
                (stats.fan_out_sum as f64 / stats.files as f64 * 100.0).round() / 100.0
            } else {
                0.0
            };
            let avg_fan_in = if stats.files > 0 {
                (stats.fan_in_sum as f64 / stats.files as f64 * 100.0).round() / 100.0
            } else {
                0.0
            };
            DirSummaryEntry {
                directory: dir,
                files: stats.files,
                exports: stats.exports,
                dead_exports: stats.dead_exports,
                avg_fan_out,
                avg_fan_in,
            }
        })
        .collect();

    directories.sort_by(|a, b| a.directory.cmp(&b.directory));

    #[derive(serde::Serialize)]
    struct DirSummaryResult {
        command: String,
        directories: Vec<DirSummaryEntry>,
        count: usize,
    }

    let count = directories.len();
    let result = DirSummaryResult {
        command: "summary".to_string(),
        directories,
        count,
    };

    Ok(match format {
        OutputFormat::Text => format_dir_summary_text(&result),
        _ => format_json(&result, format),
    })
}

fn format_dir_summary_text(result: &impl serde::Serialize) -> String {
    let value = serde_json::to_value(result).unwrap_or_default();
    let mut out = String::new();
    out.push_str("Directory Summary\n");
    out.push_str(&format!("{}\n\n", "=".repeat(40)));

    if let Some(dirs) = value.get("directories").and_then(|v| v.as_array()) {
        out.push_str(&format!(
            "  {:<40} {:>5} {:>7} {:>12} {:>10} {:>9}\n",
            "Directory", "Files", "Exports", "Dead Exports", "Avg Fan-Out", "Avg Fan-In"
        ));
        out.push_str(&format!("  {}\n", "-".repeat(85)));

        for d in dirs {
            let dir = d.get("directory").and_then(|v| v.as_str()).unwrap_or("?");
            let files = d.get("files").and_then(|v| v.as_u64()).unwrap_or(0);
            let exports = d.get("exports").and_then(|v| v.as_u64()).unwrap_or(0);
            let dead = d.get("dead_exports").and_then(|v| v.as_u64()).unwrap_or(0);
            let fan_out = d.get("avg_fan_out").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let fan_in = d.get("avg_fan_in").and_then(|v| v.as_f64()).unwrap_or(0.0);
            out.push_str(&format!(
                "  {:<40} {:>5} {:>7} {:>12} {:>10.2} {:>9.2}\n",
                dir, files, exports, dead, fan_out, fan_in
            ));
        }

        let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        out.push_str(&format!("\n{} directories\n", count));
    }

    out
}

/// Run the `lint` command. Returns (output_string, has_errors).
#[allow(clippy::too_many_arguments)]
pub fn run_lint(
    project_path: &Path,
    config_path: Option<&str>,
    rule_filter: Option<&str>,
    severity_threshold: &str,
    format: &OutputFormat,
    no_index: bool,
    path_glob: Option<&str>,
    freeze: bool,
) -> Result<(String, bool)> {
    use crate::linting::baseline::Baseline;
    use crate::linting::config::{find_config_path, load_config, Severity};
    use crate::linting::rules::evaluate_rules;

    // Find and load config
    let config_override = config_path.map(PathBuf::from);
    let config_file = find_config_path(project_path, config_override.as_deref())
        .context("No lint config found. Create .statik/rules.toml or use --config <path>.")?;

    let mut config = load_config(&config_file)?;

    // Filter to a specific rule if requested
    if let Some(rule_id) = rule_filter {
        config.rules.retain(|r| r.id == rule_id);
        if config.rules.is_empty() {
            anyhow::bail!("No rule found with id '{}'", rule_id);
        }
    }

    // Parse severity threshold
    let threshold = match severity_threshold {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    };

    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let mut result = evaluate_rules(&config, &graph, project_path)?;

    // Filter by severity threshold
    result.violations.retain(|v| match threshold {
        Severity::Error => v.severity == Severity::Error,
        Severity::Warning => v.severity == Severity::Error || v.severity == Severity::Warning,
        Severity::Info => true,
    });

    if freeze {
        // Save current violations as the baseline
        let baseline = Baseline::from_violations(&result.violations);
        baseline.save(project_path)?;
        eprintln!(
            "Baseline saved with {} violations to .statik/lint-baseline.json",
            result.violations.len()
        );
    } else {
        // Filter out known baseline violations
        if let Some(baseline) = Baseline::load(project_path)? {
            let total_before = result.violations.len();
            result.violations = baseline.filter_known(result.violations);
            let suppressed = total_before - result.violations.len();
            if suppressed > 0 {
                eprintln!(
                    "{} baseline violations suppressed (use --update-baseline to refresh)",
                    suppressed
                );
            }
        }
    }

    // Recompute summary after filtering
    let errors = result
        .violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    let warnings = result
        .violations
        .iter()
        .filter(|v| v.severity == Severity::Warning)
        .count();
    let infos = result
        .violations
        .iter()
        .filter(|v| v.severity == Severity::Info)
        .count();
    result.summary.total_violations = result.violations.len();
    result.summary.errors = errors;
    result.summary.warnings = warnings;
    result.summary.infos = infos;

    let has_errors = errors > 0;

    let output = match format {
        OutputFormat::Text => format_lint_text(&result),
        _ => format_json(&result, format),
    };

    Ok((output, has_errors))
}

/// Run the `diff` command.
/// Run diff comparing two pre-opened databases (used for --before and git ref modes).
pub fn run_diff_from_dbs(
    db_before: &Database,
    db_after: &Database,
    project_root_before: &Path,
    project_root_after: &Path,
    format: &OutputFormat,
) -> Result<String> {
    use crate::analysis::diff::compare_snapshots_with_graphs;

    let graph_before = build_file_graph(db_before, project_root_before)?;
    let graph_after = build_file_graph(db_after, project_root_after)?;

    let result =
        compare_snapshots_with_graphs(db_before, db_after, &graph_before, &graph_after)?;

    Ok(match format {
        OutputFormat::Text => format_diff_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run diff using --before flag (backward-compat DB path comparison).
pub fn run_diff(
    project_path: &Path,
    before_path: &str,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    let db_before = Database::open(std::path::Path::new(before_path))
        .context(format!("Failed to open baseline database: {}", before_path))?;
    let db_after = ensure_index(project_path, no_index)?;

    run_diff_from_dbs(&db_before, &db_after, project_path, project_path, format)
}

/// Run diff comparing two git refs.
pub fn run_diff_git(
    project_path: &Path,
    ref1: &str,
    ref2: Option<&str>,
    format: &OutputFormat,
) -> Result<String> {
    use crate::git;

    let sha1 = git::resolve_git_ref(project_path, ref1)
        .context(format!("Failed to resolve ref1: {}", ref1))?;

    // Index the first ref (with caching)
    let db_before = index_git_ref(project_path, &sha1)?;

    // For the second ref: if provided, resolve and index; otherwise use working tree
    let db_after = if let Some(r2) = ref2 {
        let sha2 = git::resolve_git_ref(project_path, r2)
            .context(format!("Failed to resolve ref2: {}", r2))?;
        index_git_ref(project_path, &sha2)?
    } else {
        // Use current working tree
        ensure_index(project_path, false)?
    };

    run_diff_from_dbs(
        &db_before,
        &db_after,
        project_path,
        project_path,
        format,
    )
}

/// Index a git ref, using cache if available.
fn index_git_ref(project_path: &Path, sha: &str) -> Result<Database> {
    use crate::git;

    let cache_path = git::snapshot_cache_path(project_path, sha);

    if cache_path.exists() {
        return Database::open(&cache_path)
            .context(format!("Failed to open cached snapshot: {}", cache_path.display()));
    }

    // Export tree to temp dir and index it
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    git::export_tree_at_ref(project_path, sha, temp_dir.path())
        .context(format!("Failed to export tree at {}", sha))?;

    let config = crate::discovery::DiscoveryConfig::default();
    let index_result = crate::cli::index::run_index(temp_dir.path(), &config)?;

    // Relativize paths in the DB so they match across different temp dirs
    let temp_db_path = temp_dir.path().join(".statik/index.db");
    if temp_db_path.exists() {
        let temp_db = Database::open(&temp_db_path)?;
        temp_db.relativize_paths(temp_dir.path())?;
    }

    // Copy the indexed DB to cache
    if temp_db_path.exists() {
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&temp_db_path, &cache_path).context(format!(
            "Failed to cache snapshot at {}",
            cache_path.display()
        ))?;

        eprintln!(
            "Indexed {} at {}: {} files, {} symbols",
            &sha[..8.min(sha.len())],
            temp_dir.path().display(),
            index_result.files_indexed + index_result.files_unchanged,
            index_result.symbols_extracted,
        );
    }

    Database::open(&cache_path)
        .context(format!("Failed to open indexed snapshot for {}", sha))
}

/// Run the `symbols` command.
pub fn run_symbols(
    project_path: &Path,
    file: Option<&str>,
    kind: Option<&str>,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    let symbols = match (file, kind) {
        (Some(f), _) => {
            // Find file in DB by path suffix match
            let all_files = db.all_files()?;
            let file_record = all_files
                .iter()
                .find(|fr| {
                    let abs = project_path.join(f);
                    fr.path == abs || fr.path.ends_with(f)
                })
                .context(format!("File not found in index: {}", f))?;
            db.get_symbols_by_file(file_record.id)?
        }
        (_, Some(k)) => {
            let sk: SymbolKind = k.parse().map_err(|e: String| anyhow::anyhow!("{}", e))?;
            db.find_symbols_by_kind(sk)?
        }
        _ => db.all_symbols()?,
    };

    #[derive(serde::Serialize)]
    struct SymbolInfo {
        name: String,
        qualified_name: String,
        kind: String,
        file: String,
        line: usize,
        visibility: String,
    }

    // Build file ID -> path lookup
    let all_files = db.all_files()?;
    let file_paths: HashMap<FileId, PathBuf> =
        all_files.iter().map(|f| (f.id, f.path.clone())).collect();

    let symbol_infos: Vec<SymbolInfo> = symbols
        .iter()
        .map(|s| {
            let file_path = file_paths
                .get(&s.file)
                .map(|p| display_path(p))
                .unwrap_or_else(|| format!("file:{}", s.file.0));
            SymbolInfo {
                name: s.name.clone(),
                qualified_name: s.qualified_name.clone(),
                kind: s.kind.as_str().to_string(),
                file: file_path,
                line: s.line_span.start.line,
                visibility: s.visibility.as_str().to_string(),
            }
        })
        .collect();

    #[derive(serde::Serialize)]
    struct SymbolsResult {
        command: String,
        symbols: Vec<SymbolInfo>,
        count: usize,
    }

    let count = symbol_infos.len();
    let result = SymbolsResult {
        command: "symbols".to_string(),
        symbols: symbol_infos,
        count,
    };

    Ok(match format {
        OutputFormat::Text => format_symbols_text(&result),
        _ => format_json(&result, format),
    })
}

fn format_symbols_text(result: &impl serde::Serialize) -> String {
    let value = serde_json::to_value(result).unwrap_or_default();
    let mut out = String::new();

    if let Some(symbols) = value.get("symbols").and_then(|v| v.as_array()) {
        let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        out.push_str(&format!("Symbols ({}):\n\n", count));
        out.push_str(&format!(
            "  {:<30} {:<12} {:<40} {:<6} {:<10}\n",
            "Name", "Kind", "File", "Line", "Visibility"
        ));
        out.push_str(&format!("  {}\n", "-".repeat(100)));

        for sym in symbols {
            let name = sym.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = sym.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let file = sym.get("file").and_then(|v| v.as_str()).unwrap_or("?");
            let line = sym.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            let vis = sym
                .get("visibility")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            out.push_str(&format!(
                "  {:<30} {:<12} {:<40} {:<6} {:<10}\n",
                name, kind, file, line, vis
            ));
        }
    }

    out
}

/// Run the `references` command.
pub fn run_references(
    project_path: &Path,
    symbol_name: &str,
    kind_filter: Option<&str>,
    file_filter: Option<&str>,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    let all_refs = db.all_references()?;
    let all_symbols = db.all_symbols()?;
    let all_files = db.all_files()?;

    // Build lookups
    let symbol_map: HashMap<crate::model::SymbolId, &crate::model::Symbol> =
        all_symbols.iter().map(|s| (s.id, s)).collect();
    let file_paths: HashMap<FileId, PathBuf> =
        all_files.iter().map(|f| (f.id, f.path.clone())).collect();

    // Find all symbols matching the name
    let matching_symbols: Vec<crate::model::SymbolId> = all_symbols
        .iter()
        .filter(|s| s.name == symbol_name)
        .map(|s| s.id)
        .collect();

    if matching_symbols.is_empty() {
        anyhow::bail!("No symbol found with name: {}", symbol_name);
    }

    // Parse kind filter
    let kind_filter: Option<RefKind> = kind_filter.map(|k| match k {
        "call" => RefKind::Call,
        "type_usage" => RefKind::TypeUsage,
        "inheritance" => RefKind::Inheritance,
        "import" => RefKind::Import,
        "export" => RefKind::Export,
        "field_access" => RefKind::FieldAccess,
        "assignment" => RefKind::Assignment,
        _ => RefKind::Call,
    });

    // Resolve file filter
    let file_filter_id: Option<FileId> = file_filter.and_then(|f| {
        all_files
            .iter()
            .find(|fr| {
                let abs = project_path.join(f);
                fr.path == abs || fr.path.ends_with(f)
            })
            .map(|fr| fr.id)
    });

    // Find all references where source or target matches
    let matching_refs: Vec<_> = all_refs
        .iter()
        .filter(|r| {
            let matches_symbol =
                matching_symbols.contains(&r.source) || matching_symbols.contains(&r.target);
            let matches_kind = kind_filter.is_none_or(|k| r.kind == k);
            let matches_file = file_filter_id.is_none_or(|fid| r.file == fid);
            matches_symbol && matches_kind && matches_file
        })
        .collect();

    #[derive(serde::Serialize)]
    struct RefInfo {
        source: String,
        target: String,
        kind: String,
        file: String,
        line: usize,
        cross_file: bool,
    }

    let mut ref_infos: Vec<RefInfo> = matching_refs
        .iter()
        .map(|r| {
            let source_name = symbol_map
                .get(&r.source)
                .map(|s| s.qualified_name.as_str())
                .unwrap_or("?");
            let target_name = symbol_map
                .get(&r.target)
                .map(|s| s.qualified_name.as_str())
                .unwrap_or("?");
            let file_path = file_paths
                .get(&r.file)
                .map(|p| display_path(p))
                .unwrap_or_else(|| format!("file:{}", r.file.0));
            RefInfo {
                source: source_name.to_string(),
                target: target_name.to_string(),
                kind: r.kind.as_str().to_string(),
                file: file_path,
                line: r.line_span.start.line,
                cross_file: false,
            }
        })
        .collect();

    // Cross-file linking: match imports to exports across file boundaries
    let file_graph = build_file_graph(&db, project_path)?;
    let link_result =
        crate::analysis::linker::link_cross_file_symbols(&file_graph);
    for xref in &link_result.references {
        // Only include cross-file refs where target symbol matches our search
        if !matching_symbols.contains(&xref.target_symbol) {
            continue;
        }
        // Apply file filter
        if file_filter_id.is_some_and(|fid| fid != xref.source_file) {
            continue;
        }
        // Apply kind filter: cross-file refs are import-kind references
        if kind_filter.is_some_and(|k| k != RefKind::Import) {
            continue;
        }
        let source_path = file_paths
            .get(&xref.source_file)
            .map(|p| display_path(p))
            .unwrap_or_else(|| format!("file:{}", xref.source_file.0));
        let target_name = symbol_map
            .get(&xref.target_symbol)
            .map(|s| s.qualified_name.as_str())
            .unwrap_or("?");
        ref_infos.push(RefInfo {
            source: format!("(import from {})", source_path),
            target: target_name.to_string(),
            kind: "import".to_string(),
            file: source_path,
            line: xref.line,
            cross_file: true,
        });
    }

    // Deduplicate refs by (file, target, line)
    {
        let mut seen = HashSet::new();
        ref_infos.retain(|r| seen.insert((r.file.clone(), r.target.clone(), r.line)));
    }

    #[derive(serde::Serialize)]
    struct RefsResult {
        command: String,
        symbol: String,
        references: Vec<RefInfo>,
        count: usize,
    }

    let count = ref_infos.len();
    let result = RefsResult {
        command: "references".to_string(),
        symbol: symbol_name.to_string(),
        references: ref_infos,
        count,
    };

    Ok(match format {
        OutputFormat::Text => format_references_text(&result),
        _ => format_json(&result, format),
    })
}

fn format_references_text(result: &impl serde::Serialize) -> String {
    let value = serde_json::to_value(result).unwrap_or_default();
    let mut out = String::new();

    let symbol = value.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
    let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    out.push_str(&format!("References for '{}' ({}):\n\n", symbol, count));

    if let Some(refs) = value.get("references").and_then(|v| v.as_array()) {
        if refs.is_empty() {
            out.push_str("No references found.\n");
        } else {
            for r in refs {
                let source = r.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                let target = r.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let file = r.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                let line = r.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                let is_cross = r
                    .get("cross_file")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let marker = if is_cross { " [cross-file]" } else { "" };
                out.push_str(&format!(
                    "  {} -> {} [{}] at {}:{}{}\n",
                    source, target, kind, file, line, marker
                ));
            }
        }
    }

    out
}

/// Run the `callers` command.
pub fn run_callers(
    project_path: &Path,
    symbol_name: &str,
    file_filter: Option<&str>,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    // Callers is just references filtered to RefKind::Call, showing only incoming calls
    let db = ensure_index(project_path, no_index)?;

    let all_refs = db.all_references()?;
    let all_symbols = db.all_symbols()?;
    let all_files = db.all_files()?;

    let symbol_map: HashMap<crate::model::SymbolId, &crate::model::Symbol> =
        all_symbols.iter().map(|s| (s.id, s)).collect();
    let file_paths: HashMap<FileId, PathBuf> =
        all_files.iter().map(|f| (f.id, f.path.clone())).collect();

    // Find target symbols matching the name
    let target_symbols: Vec<crate::model::SymbolId> = all_symbols
        .iter()
        .filter(|s| s.name == symbol_name)
        .map(|s| s.id)
        .collect();

    if target_symbols.is_empty() {
        anyhow::bail!("No symbol found with name: {}", symbol_name);
    }

    let file_filter_id: Option<FileId> = file_filter.and_then(|f| {
        all_files
            .iter()
            .find(|fr| {
                let abs = project_path.join(f);
                fr.path == abs || fr.path.ends_with(f)
            })
            .map(|fr| fr.id)
    });

    // Find call references where target matches
    let callers: Vec<_> = all_refs
        .iter()
        .filter(|r| {
            r.kind == RefKind::Call
                && target_symbols.contains(&r.target)
                && file_filter_id.is_none_or(|fid| r.file == fid)
        })
        .collect();

    #[derive(serde::Serialize)]
    struct CallerInfo {
        caller: String,
        kind: String,
        file: String,
        line: usize,
        cross_file: bool,
    }

    let mut caller_infos: Vec<CallerInfo> = callers
        .iter()
        .map(|r| {
            let caller_name = symbol_map
                .get(&r.source)
                .map(|s| s.qualified_name.as_str())
                .unwrap_or("?");
            let file_path = file_paths
                .get(&r.file)
                .map(|p| display_path(p))
                .unwrap_or_else(|| format!("file:{}", r.file.0));
            CallerInfo {
                caller: caller_name.to_string(),
                kind: "call".to_string(),
                file: file_path,
                line: r.line_span.start.line,
                cross_file: false,
            }
        })
        .collect();

    // Cross-file linking: find importers of the target symbol across files
    let file_graph = build_file_graph(&db, project_path)?;
    let link_result =
        crate::analysis::linker::link_cross_file_symbols(&file_graph);
    for xref in &link_result.references {
        if !target_symbols.contains(&xref.target_symbol) {
            continue;
        }
        if file_filter_id.is_some_and(|fid| fid != xref.source_file) {
            continue;
        }
        let source_path = file_paths
            .get(&xref.source_file)
            .map(|p| display_path(p))
            .unwrap_or_else(|| format!("file:{}", xref.source_file.0));
        caller_infos.push(CallerInfo {
            caller: format!("(import '{}')", xref.imported_name),
            kind: "import".to_string(),
            file: source_path,
            line: xref.line,
            cross_file: true,
        });
    }

    // Deduplicate callers by (file, caller, line)
    {
        let mut seen = HashSet::new();
        caller_infos.retain(|c| seen.insert((c.file.clone(), c.caller.clone(), c.line)));
    }

    #[derive(serde::Serialize)]
    struct CallersResult {
        command: String,
        symbol: String,
        callers: Vec<CallerInfo>,
        count: usize,
    }

    let count = caller_infos.len();
    let result = CallersResult {
        command: "callers".to_string(),
        symbol: symbol_name.to_string(),
        callers: caller_infos,
        count,
    };

    Ok(match format {
        OutputFormat::Text => {
            let mut out = String::new();
            out.push_str(&format!("Callers of '{}' ({}):\n\n", symbol_name, count));
            if result.callers.is_empty() {
                out.push_str("No callers found.\n");
            } else {
                for c in &result.callers {
                    let marker = if c.cross_file { " [cross-file]" } else { "" };
                    out.push_str(&format!(
                        "  {} at {}:{}{}\n",
                        c.caller, c.file, c.line, marker
                    ));
                }
            }
            out
        }
        _ => format_json(&result, format),
    })
}

/// Run the `graph` command.
#[allow(clippy::too_many_arguments)]
pub fn run_graph(
    project_path: &Path,
    graph_format: &str,
    focus: Option<&str>,
    depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;

    let (graph, focus_id) = if let Some(focus_path) = focus {
        let abs_path = project_path.join(focus_path);
        let fid = graph
            .file_by_path(&abs_path)
            .or_else(|| {
                graph
                    .files
                    .values()
                    .find(|f| f.path.ends_with(focus_path))
                    .map(|f| f.id)
            })
            .context(format!("Focus file not found in index: {}", focus_path))?;
        let subgraph = extract_subgraph(&graph, fid, depth);
        (subgraph, Some(fid))
    } else {
        (graph, None)
    };

    // When --format json/compact is explicitly requested, always output JSON
    match format {
        OutputFormat::Json | OutputFormat::Compact | OutputFormat::Csv => {
            let result = build_graph_json(&graph, project_path, focus_id);
            Ok(format_json(&result, format))
        }
        _ => match graph_format {
            "dot" => Ok(generate_dot(&graph, project_path, focus_id)),
            "svg" => generate_svg(&graph, project_path, focus_id),
            "html" => Ok(generate_html(&graph, project_path, focus_id)),
            "json" => {
                let result = build_graph_json(&graph, project_path, focus_id);
                Ok(format_json(&result, &OutputFormat::Json))
            }
            _ => Ok(generate_dot(&graph, project_path, focus_id)),
        },
    }
}

/// Extract a subgraph around a focus file using BFS in both directions.
fn extract_subgraph(graph: &FileGraph, focus: FileId, max_depth: Option<usize>) -> FileGraph {
    use std::collections::{HashSet, VecDeque};

    let max_depth = max_depth.unwrap_or(usize::MAX);
    let mut visited: HashSet<FileId> = HashSet::new();
    let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();

    visited.insert(focus);
    queue.push_back((focus, 0));

    while let Some((file_id, current_depth)) = queue.pop_front() {
        if current_depth >= max_depth {
            continue;
        }
        for target in graph.direct_imports(file_id) {
            if visited.insert(target) {
                queue.push_back((target, current_depth + 1));
            }
        }
        for source in graph.direct_importers(file_id) {
            if visited.insert(source) {
                queue.push_back((source, current_depth + 1));
            }
        }
    }

    let mut new_graph = FileGraph::new();
    for &file_id in &visited {
        if let Some(info) = graph.get_file(file_id) {
            new_graph.add_file(info.clone());
        }
    }
    for (&_from_id, edges) in graph.all_import_edges() {
        for edge in edges {
            if visited.contains(&edge.from) && visited.contains(&edge.to) {
                new_graph.add_import(edge.clone());
            }
        }
    }
    new_graph
}

/// Generate DOT format output.
fn generate_dot(graph: &FileGraph, project_root: &Path, focus_id: Option<FileId>) -> String {
    let mut out = String::new();
    out.push_str("digraph dependencies {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box, style=filled, fillcolor=\"#e8e8e8\"];\n");
    out.push('\n');

    let mut files: Vec<_> = graph.all_files().collect();
    files.sort_by_key(|(id, _)| **id);

    for (_, info) in &files {
        let rel_path = relative_path(&info.path, project_root);
        let color = if Some(info.id) == focus_id {
            "#8888ff"
        } else if info.is_entry_point {
            "#a8d8a8"
        } else {
            "#e8e8e8"
        };
        out.push_str(&format!(
            "  \"{}\" [fillcolor=\"{}\"];\n",
            rel_path, color
        ));
    }
    out.push('\n');

    let mut edges: Vec<(String, String)> = Vec::new();
    for (_, info) in &files {
        if let Some(import_edges) = graph.import_edges(info.id) {
            for edge in import_edges {
                if let Some(target) = graph.get_file(edge.to) {
                    let from = relative_path(&info.path, project_root);
                    let to = relative_path(&target.path, project_root);
                    edges.push((from, to));
                }
            }
        }
    }
    edges.sort();
    edges.dedup();

    for (from, to) in &edges {
        out.push_str(&format!("  \"{}\" -> \"{}\";\n", from, to));
    }
    out.push_str("}\n");
    out
}

/// Generate SVG by shelling out to the `dot` command.
fn generate_svg(
    graph: &FileGraph,
    project_root: &Path,
    focus_id: Option<FileId>,
) -> Result<String> {
    let dot = generate_dot(graph, project_root, focus_id);

    let result = std::process::Command::new("dot")
        .arg("-Tsvg")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    match result {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(dot.as_bytes())?;
            }
            let output = child.wait_with_output()?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("dot command failed: {}", stderr);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "graphviz `dot` command not found. Install graphviz:\n  \
                 macOS:  brew install graphviz\n  \
                 Ubuntu: sudo apt install graphviz\n  \
                 Or use --graph-format dot to get raw DOT output."
            );
        }
        Err(e) => anyhow::bail!("Failed to run dot command: {}", e),
    }
}

/// Build the JSON representation for the graph.
fn build_graph_json(
    graph: &FileGraph,
    project_root: &Path,
    focus_id: Option<FileId>,
) -> serde_json::Value {
    let mut nodes = Vec::new();
    let mut edges_out = Vec::new();
    let mut files: Vec<_> = graph.all_files().collect();
    files.sort_by_key(|(id, _)| **id);

    for (_, info) in &files {
        let rel = relative_path(&info.path, project_root);
        let lang = format!("{:?}", info.language);
        let mut node = serde_json::json!({
            "path": rel,
            "is_entry_point": info.is_entry_point,
            "language": lang,
        });
        if Some(info.id) == focus_id {
            node["is_focus"] = serde_json::json!(true);
        }
        nodes.push(node);
    }

    let mut edge_set: Vec<(String, String, Vec<String>)> = Vec::new();
    for (_, info) in &files {
        if let Some(import_edges) = graph.import_edges(info.id) {
            let mut by_target: HashMap<FileId, Vec<String>> = HashMap::new();
            for edge in import_edges {
                by_target
                    .entry(edge.to)
                    .or_default()
                    .extend(edge.imported_names.clone());
            }
            for (target_id, mut names) in by_target {
                if let Some(target) = graph.get_file(target_id) {
                    let from = relative_path(&info.path, project_root);
                    let to = relative_path(&target.path, project_root);
                    names.sort();
                    names.dedup();
                    edge_set.push((from, to, names));
                }
            }
        }
    }
    edge_set.sort();

    for (from, to, names) in &edge_set {
        edges_out.push(serde_json::json!({
            "from": from,
            "to": to,
            "imported_names": names,
        }));
    }

    serde_json::json!({
        "nodes": nodes,
        "edges": edges_out,
        "summary": {
            "node_count": nodes.len(),
            "edge_count": edges_out.len(),
        }
    })
}

/// Generate a self-contained HTML file with a force-directed graph visualization.
fn generate_html(graph: &FileGraph, project_root: &Path, focus_id: Option<FileId>) -> String {
    let graph_json = build_graph_json(graph, project_root, focus_id);
    let json_str = serde_json::to_string(&graph_json).unwrap_or_default();
    HTML_TEMPLATE.replace("/*GRAPH_DATA*/", &format!("const graphData = {};", json_str))
}

const HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>statik - Dependency Graph</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; background: #1a1a2e; overflow: hidden; }
  svg { width: 100vw; height: 100vh; display: block; }
  #tooltip { position: absolute; background: #16213e; color: #e8e8e8; border: 1px solid #0f3460; border-radius: 6px; padding: 10px 14px; font-size: 13px; pointer-events: none; display: none; max-width: 400px; z-index: 10; box-shadow: 0 4px 12px rgba(0,0,0,0.4); }
  #tooltip .path { font-weight: bold; color: #e2b714; margin-bottom: 4px; }
  #tooltip .meta { color: #a8a8b8; font-size: 12px; }
</style>
</head>
<body>
<div id="tooltip"></div>
<svg id="graph"></svg>
<script>
/*GRAPH_DATA*/

const svg = document.getElementById("graph");
const tooltip = document.getElementById("tooltip");
const width = window.innerWidth;
const height = window.innerHeight;
const ns = "http://www.w3.org/2000/svg";

const mainGroup = document.createElementNS(ns, "g");
svg.appendChild(mainGroup);
const edgeGroup = document.createElementNS(ns, "g");
mainGroup.appendChild(edgeGroup);
const nodeGroup = document.createElementNS(ns, "g");
mainGroup.appendChild(nodeGroup);

const nodes = graphData.nodes.map((n, i) => ({
  ...n, x: width/2 + (Math.random()-0.5)*Math.min(width,600),
  y: height/2 + (Math.random()-0.5)*Math.min(height,400), vx: 0, vy: 0, index: i
}));
const nodeByPath = {};
nodes.forEach(n => { nodeByPath[n.path] = n; });
const edges = graphData.edges.map(e => ({
  source: nodeByPath[e.from], target: nodeByPath[e.to], imported_names: e.imported_names
})).filter(e => e.source && e.target);

const edgeEls = [];
edges.forEach(e => {
  const line = document.createElementNS(ns, "line");
  line.style.stroke = "#555"; line.style.strokeWidth = "1"; line.style.strokeOpacity = "0.5";
  edgeGroup.appendChild(line);
  edgeEls.push({ el: line, data: e });
});

function escapeHtml(s) { return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }
function textWidth(t) { return t.length * 6.8 + 16; }

const nodeEls = [];
nodes.forEach(n => {
  const g = document.createElementNS(ns, "g");
  const label = n.path.split("/").pop() || n.path;
  const w = Math.max(textWidth(label), 60), h = 26;
  const rect = document.createElementNS(ns, "rect");
  rect.setAttribute("width", w); rect.setAttribute("height", h);
  rect.setAttribute("rx", 4); rect.setAttribute("ry", 4);
  rect.setAttribute("x", -w/2); rect.setAttribute("y", -h/2);
  rect.style.fill = n.is_focus ? "#8888ff" : n.is_entry_point ? "#a8d8a8" : "#e8e8e8";
  rect.style.stroke = "#555"; rect.style.strokeWidth = "1"; rect.style.cursor = "pointer";
  const text = document.createElementNS(ns, "text");
  text.setAttribute("text-anchor", "middle"); text.setAttribute("dominant-baseline", "central");
  text.style.fontSize = "11px"; text.style.fill = "#222"; text.style.pointerEvents = "none";
  text.textContent = label;
  g.appendChild(rect); g.appendChild(text); nodeGroup.appendChild(g);

  g.addEventListener("mouseenter", () => {
    const imp = edges.filter(e => e.source === n).map(e => e.target.path);
    const by = edges.filter(e => e.target === n).map(e => e.source.path);
    let html = '<div class="path">'+escapeHtml(n.path)+'</div><div class="meta">Language: '+escapeHtml(n.language)+'</div>';
    if (n.is_entry_point) html += '<div class="meta">Entry point</div>';
    if (imp.length) html += '<div class="meta">Imports: '+escapeHtml(imp.join(", "))+'</div>';
    if (by.length) html += '<div class="meta">Imported by: '+escapeHtml(by.join(", "))+'</div>';
    tooltip.innerHTML = html; tooltip.style.display = "block";
    edgeEls.forEach(ee => { if (ee.data.source===n||ee.data.target===n) { ee.el.style.stroke="#ff6b6b"; ee.el.style.strokeWidth="2"; ee.el.style.strokeOpacity="1"; }});
    rect.style.stroke = "#ff6b6b"; rect.style.strokeWidth = "2.5";
  });
  g.addEventListener("mousemove", ev => { tooltip.style.left=(ev.pageX+14)+"px"; tooltip.style.top=(ev.pageY+14)+"px"; });
  g.addEventListener("mouseleave", () => {
    tooltip.style.display = "none";
    edgeEls.forEach(ee => { ee.el.style.stroke="#555"; ee.el.style.strokeWidth="1"; ee.el.style.strokeOpacity="0.5"; });
    rect.style.stroke = "#555"; rect.style.strokeWidth = "1";
  });
  nodeEls.push({ el: g, data: n, w, h });
});

let dragNode = null, dragOffX = 0, dragOffY = 0;
svg.addEventListener("mousedown", ev => {
  const t = ev.target.closest("g"); if (!t) return;
  const ne = nodeEls.find(n => n.el === t); if (!ne) return;
  dragNode = ne.data; dragOffX = ev.clientX - dragNode.x; dragOffY = ev.clientY - dragNode.y;
  dragNode.fx = dragNode.x; dragNode.fy = dragNode.y;
});
svg.addEventListener("mousemove", ev => { if (!dragNode) return; dragNode.fx=ev.clientX-dragOffX; dragNode.fy=ev.clientY-dragOffY; dragNode.x=dragNode.fx; dragNode.y=dragNode.fy; });
svg.addEventListener("mouseup", () => { if (dragNode) { delete dragNode.fx; delete dragNode.fy; } dragNode = null; });

let transform = { x: 0, y: 0, k: 1 };
svg.addEventListener("wheel", ev => {
  ev.preventDefault(); const f = ev.deltaY > 0 ? 0.92 : 1.08;
  transform.k *= f; transform.x = ev.clientX-(ev.clientX-transform.x)*f; transform.y = ev.clientY-(ev.clientY-transform.y)*f;
  mainGroup.setAttribute("transform", "translate("+transform.x+","+transform.y+") scale("+transform.k+")");
});
let isPanning = false, panSX, panSY;
svg.addEventListener("mousedown", ev => { if (ev.target===svg) { isPanning=true; panSX=ev.clientX-transform.x; panSY=ev.clientY-transform.y; }});
svg.addEventListener("mousemove", ev => { if (!isPanning) return; transform.x=ev.clientX-panSX; transform.y=ev.clientY-panSY; mainGroup.setAttribute("transform","translate("+transform.x+","+transform.y+") scale("+transform.k+")"); });
svg.addEventListener("mouseup", () => { isPanning = false; });

function tick() {
  const rep=800, ld=120, ls=0.05, cs=0.01;
  for (let i=0;i<nodes.length;i++) for (let j=i+1;j<nodes.length;j++) {
    let dx=nodes[j].x-nodes[i].x, dy=nodes[j].y-nodes[i].y, d2=dx*dx+dy*dy; if(d2<1) d2=1;
    let f=rep/d2, fx=dx*f, fy=dy*f;
    if(!nodes[i].fx){nodes[i].vx-=fx;nodes[i].vy-=fy;} if(!nodes[j].fx){nodes[j].vx+=fx;nodes[j].vy+=fy;}
  }
  edges.forEach(e => { let dx=e.target.x-e.source.x, dy=e.target.y-e.source.y, d=Math.sqrt(dx*dx+dy*dy)||1, f=(d-ld)*ls, fx=dx/d*f, fy=dy/d*f; if(!e.source.fx){e.source.vx+=fx;e.source.vy+=fy;} if(!e.target.fx){e.target.vx-=fx;e.target.vy-=fy;} });
  nodes.forEach(n => { if(!n.fx){n.vx+=(width/2-n.x)*cs;n.vy+=(height/2-n.y)*cs;} });
  nodes.forEach(n => { if(n.fx!==undefined){n.x=n.fx;n.y=n.fy;return;} n.vx*=0.6;n.vy*=0.6;n.x+=n.vx*0.3;n.y+=n.vy*0.3; });
  nodeEls.forEach(ne => ne.el.setAttribute("transform","translate("+ne.data.x+","+ne.data.y+")"));
  edgeEls.forEach(ee => { ee.el.setAttribute("x1",ee.data.source.x); ee.el.setAttribute("y1",ee.data.source.y); ee.el.setAttribute("x2",ee.data.target.x); ee.el.setAttribute("y2",ee.data.target.y); });
  requestAnimationFrame(tick);
}
tick();
</script>
</body>
</html>
"##;

/// Get a relative path from a file path and project root.
fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Format any serializable analysis result as JSON.
fn format_json<T: serde::Serialize>(value: &T, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json | OutputFormat::Csv => {
            serde_json::to_string_pretty(value).unwrap_or_default()
        }
        OutputFormat::Compact => serde_json::to_string(value).unwrap_or_default(),
        OutputFormat::Text => unreachable!("text format should be handled by caller"),
    }
}

/// Strip a common project root prefix from a path for display.
fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// Map a CLI language string (e.g. "java", "typescript", "ts") to a `Language`.
fn lang_str_to_language(s: &str) -> Option<Language> {
    let ext = match s.to_lowercase().as_str() {
        "typescript" | "ts" => "ts",
        "javascript" | "js" => "js",
        "python" | "py" => "py",
        "rust" | "rs" => "rs",
        "java" => "java",
        _ => return None,
    };
    Language::from_extension(ext)
}

// --- Text formatters for each command ---

fn format_deps_text(result: &crate::analysis::dependencies::DepsResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Dependencies for {}\n",
        display_path(&result.target_path)
    ));
    out.push('\n');

    if !result.imports.is_empty() {
        out.push_str(&format!("Imports ({}):\n", result.imports.len()));
        for dep in &result.imports {
            let indent = "  ".repeat(dep.depth);
            let names = if dep.imported_names.is_empty() {
                String::new()
            } else {
                format!(" ({})", dep.imported_names.join(", "))
            };
            out.push_str(&format!("{}{}{}\n", indent, display_path(&dep.path), names));
        }
        out.push('\n');
    }

    if !result.imported_by.is_empty() {
        out.push_str(&format!("Imported by ({}):\n", result.imported_by.len()));
        for dep in &result.imported_by {
            let indent = "  ".repeat(dep.depth);
            let names = if dep.imported_names.is_empty() {
                String::new()
            } else {
                format!(" ({})", dep.imported_names.join(", "))
            };
            out.push_str(&format!("{}{}{}\n", indent, display_path(&dep.path), names));
        }
        out.push('\n');
    }

    if result.imports.is_empty() && result.imported_by.is_empty() {
        out.push_str("No dependencies found.\n");
    }

    out.push_str(&format!("Confidence: {}", result.confidence));
    out
}

fn format_dead_code_text(result: &crate::analysis::dead_code::DeadCodeResult) -> String {
    let mut out = String::new();

    if !result.dead_files.is_empty() {
        out.push_str(&format!("Dead files ({}):\n", result.dead_files.len()));
        for f in &result.dead_files {
            out.push_str(&format!("  {} [{}]\n", display_path(&f.path), f.confidence));
        }
        out.push('\n');
    }

    if !result.dead_exports.is_empty() {
        out.push_str(&format!("Dead exports ({}):\n", result.dead_exports.len()));
        for e in &result.dead_exports {
            out.push_str(&format!(
                "  {}  {}  [{}]\n",
                display_path(&e.path),
                e.export_name,
                e.confidence,
            ));
        }
        out.push('\n');
    }

    if result.dead_files.is_empty() && result.dead_exports.is_empty() {
        out.push_str("No dead code found.\n\n");
    }

    out.push_str(&format!(
        "Summary: {}/{} dead files, {}/{} dead exports, {} entry points\n",
        result.summary.dead_files,
        result.summary.total_files,
        result.summary.dead_exports,
        result.summary.total_exports,
        result.summary.entry_points,
    ));
    out.push_str(&format!("Confidence: {}", result.confidence));

    if !result.limitations.is_empty() {
        out.push('\n');
        for lim in &result.limitations {
            out.push_str(&format!("  Warning: {}\n", lim.description));
        }
    }

    out
}

fn format_dead_symbols_text(result: &crate::analysis::dead_code::DeadSymbolResult) -> String {
    let mut out = String::new();

    if result.dead_symbols.is_empty() {
        out.push_str("No dead symbols found.\n\n");
    } else {
        out.push_str(&format!(
            "Dead symbols ({}):\n\n",
            result.dead_symbols.len()
        ));
        out.push_str(&format!(
            "  {:<30} {:<12} {:<40} {:<6} {:<8}\n",
            "Name", "Kind", "File", "Line", "Confidence"
        ));
        out.push_str(&format!("  {}\n", "-".repeat(98)));

        for s in &result.dead_symbols {
            out.push_str(&format!(
                "  {:<30} {:<12} {:<40} {:<6} {:<8}\n",
                s.name,
                s.kind,
                s.file,
                s.line,
                format!("{}", s.confidence),
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Summary: {}/{} dead symbols, {} entry point symbols, {}/{} refs resolved\n",
        result.summary.dead_symbols,
        result.summary.total_symbols,
        result.summary.entry_point_symbols,
        result.summary.resolved_references,
        result.summary.resolved_references + result.summary.unresolved_references,
    ));
    out.push_str(&format!("Confidence: {}", result.confidence));

    if !result.limitations.is_empty() {
        out.push('\n');
        for lim in &result.limitations {
            out.push_str(&format!("  Warning: {}\n", lim.description));
        }
    }

    out
}

fn format_cycles_text(result: &crate::analysis::cycles::CycleResult) -> String {
    let mut out = String::new();

    if result.cycles.is_empty() {
        out.push_str("No circular dependencies found.\n");
    } else {
        out.push_str(&format!(
            "Circular dependencies ({} cycles, {} files involved):\n\n",
            result.summary.cycle_count, result.summary.files_in_cycles,
        ));
        for (i, cycle) in result.cycles.iter().enumerate() {
            out.push_str(&format!("  Cycle {} ({} files):\n", i + 1, cycle.length));
            for (j, file) in cycle.files.iter().enumerate() {
                if j < cycle.files.len() - 1 {
                    out.push_str(&format!("    {} ->\n", display_path(&file.path)));
                } else {
                    out.push_str(&format!("    {}\n", display_path(&file.path)));
                    out.push_str(&format!(
                        "    -> {} (cycle back)\n",
                        display_path(&cycle.files[0].path)
                    ));
                }
            }
            out.push('\n');
        }
    }

    out.push_str(&format!("Confidence: {}", result.confidence));
    out
}

fn format_impact_text(result: &crate::analysis::impact::ImpactResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Impact of changing {}\n\n",
        display_path(&result.target_path)
    ));

    if result.affected.is_empty() {
        out.push_str("No files affected.\n");
    } else {
        out.push_str(&format!(
            "Affected files ({} total, max depth {}):\n",
            result.summary.total_affected, result.summary.max_depth,
        ));

        let mut max_depth = 0;
        for af in &result.affected {
            if af.depth > max_depth {
                max_depth = af.depth;
            }
        }

        for depth in 1..=max_depth {
            if let Some(files) = result.by_depth.get(&depth) {
                out.push_str(&format!("  Depth {}:\n", depth));
                for af in files {
                    out.push_str(&format!("    {}\n", display_path(&af.path)));
                }
            }
        }
        out.push('\n');
    }

    out.push_str(&format!("Confidence: {}", result.confidence));
    out
}

fn format_exports_text(result: &serde_json::Value) -> String {
    let mut out = String::new();

    let file = result
        .get("file")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    out.push_str(&format!("Exports for {}\n\n", file));

    if let Some(exports) = result.get("exports").and_then(|v| v.as_array()) {
        if exports.is_empty() {
            out.push_str("No exports found.\n");
        } else {
            // Table header
            out.push_str(&format!(
                "  {:<30} {:<10} {:<12} {:<6}\n",
                "Name", "Default", "Re-export", "Used"
            ));
            out.push_str(&format!("  {}\n", "-".repeat(60)));

            for exp in exports {
                let name = exp.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let is_default = exp
                    .get("is_default")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_reexport = exp
                    .get("is_reexport")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_used = exp
                    .get("is_used")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let used_marker = if is_used { "yes" } else { "NO" };
                out.push_str(&format!(
                    "  {:<30} {:<10} {:<12} {:<6}\n",
                    name,
                    if is_default { "yes" } else { "" },
                    if is_reexport { "yes" } else { "" },
                    used_marker,
                ));
            }
            out.push('\n');
        }
    }

    if let Some(summary) = result.get("summary") {
        let total = summary.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let used = summary.get("used").and_then(|v| v.as_u64()).unwrap_or(0);
        let unused = summary.get("unused").and_then(|v| v.as_u64()).unwrap_or(0);
        out.push_str(&format!(
            "Summary: {} total, {} used, {} unused",
            total, used, unused
        ));
    }

    out
}

fn format_summary_text(result: &serde_json::Value) -> String {
    let mut out = String::new();
    out.push_str("Project Summary\n");
    out.push_str(&format!("{}\n\n", "=".repeat(40)));

    if let Some(files) = result.get("files") {
        let total = files.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let entry_points = files
            .get("entry_points")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        out.push_str(&format!(
            "Files: {} total, {} entry points\n",
            total, entry_points
        ));

        if let Some(by_lang) = files.get("by_language").and_then(|v| v.as_object()) {
            let mut langs: Vec<_> = by_lang.iter().collect();
            langs.sort_by(|a, b| b.1.as_u64().unwrap_or(0).cmp(&a.1.as_u64().unwrap_or(0)));
            for (lang, count) in &langs {
                out.push_str(&format!("  {}: {}\n", lang, count.as_u64().unwrap_or(0)));
            }
        }
        out.push('\n');
    }

    if let Some(deps) = result.get("dependencies") {
        let total = deps
            .get("total_imports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let external = deps
            .get("external_imports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let unresolved = deps
            .get("unresolved_imports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if unresolved > 0 {
            out.push_str(&format!(
                "Dependencies: {} imports, {} external, {} unresolved\n",
                total, external, unresolved,
            ));
        } else {
            out.push_str(&format!(
                "Dependencies: {} imports, {} external\n",
                total, external,
            ));
        }
    }

    if let Some(dc) = result.get("dead_code") {
        let dead_files = dc.get("dead_files").and_then(|v| v.as_u64()).unwrap_or(0);
        let dead_exports = dc.get("dead_exports").and_then(|v| v.as_u64()).unwrap_or(0);
        let total_exports = dc
            .get("total_exports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        out.push_str(&format!(
            "Dead code: {} dead files, {}/{} dead exports\n",
            dead_files, dead_exports, total_exports,
        ));
    }

    if let Some(cy) = result.get("cycles") {
        let count = cy.get("cycle_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let files_in = cy
            .get("files_in_cycles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        out.push_str(&format!(
            "Cycles: {} cycles, {} files involved",
            count, files_in,
        ));
    }

    out
}

fn format_lint_text(result: &crate::linting::rules::LintResult) -> String {
    use crate::linting::config::Severity;

    let mut out = String::new();

    if result.violations.is_empty() {
        out.push_str("No lint violations found.\n");
    } else {
        for v in &result.violations {
            let severity_label = match v.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            };
            out.push_str(&format!(
                "{}[{}] {}\n",
                severity_label, v.rule_id, v.description
            ));
            if v.source_file == v.target_file && v.line == 0 {
                out.push_str(&format!("  {}\n", display_path(&v.source_file)));
            } else {
                out.push_str(&format!(
                    "  {}:{} -> {}\n",
                    display_path(&v.source_file),
                    v.line,
                    display_path(&v.target_file),
                ));
            }
            if !v.imported_names.is_empty() {
                out.push_str(&format!("    imports: {}\n", v.imported_names.join(", ")));
            }
            if let Some(ref fix) = v.fix_direction {
                out.push_str(&format!("    fix: {}\n", fix));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "{} errors, {} warnings across {} rules\n",
        result.summary.errors, result.summary.warnings, result.summary.rules_evaluated,
    ));

    out
}

fn format_diff_text(result: &crate::analysis::diff::DiffResult) -> String {
    use crate::analysis::diff::{ChangeKind, EdgeChangeKind};

    let mut out = String::new();

    if result.changes.is_empty() {
        out.push_str("No export changes detected.\n");
    } else {
        let breaking: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Breaking)
            .collect();
        let expanding: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Expanding)
            .collect();
        let restructuring: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Restructuring)
            .collect();

        if !breaking.is_empty() {
            out.push_str(&format!("Breaking changes ({}):\n", breaking.len()));
            for c in &breaking {
                out.push_str(&format!(
                    "  - {}  {}  ({})\n",
                    display_path(&c.file_path),
                    c.export_name,
                    c.detail,
                ));
                for importer in &c.affected_importers {
                    out.push_str(&format!(
                        "      imported by: {}\n",
                        display_path(importer),
                    ));
                }
            }
            out.push('\n');
        }

        if !restructuring.is_empty() {
            out.push_str(&format!("Restructured ({}):\n", restructuring.len()));
            for c in &restructuring {
                out.push_str(&format!(
                    "  ~ {}  {}  ({})\n",
                    display_path(&c.file_path),
                    c.export_name,
                    c.detail,
                ));
            }
            out.push('\n');
        }

        if !expanding.is_empty() {
            out.push_str(&format!("New exports ({}):\n", expanding.len()));
            for c in &expanding {
                out.push_str(&format!(
                    "  + {}  {}  ({})\n",
                    display_path(&c.file_path),
                    c.export_name,
                    c.detail,
                ));
            }
            out.push('\n');
        }
    }

    if !result.import_edge_changes.is_empty() {
        let added: Vec<_> = result
            .import_edge_changes
            .iter()
            .filter(|e| e.change == EdgeChangeKind::Added)
            .collect();
        let removed: Vec<_> = result
            .import_edge_changes
            .iter()
            .filter(|e| e.change == EdgeChangeKind::Removed)
            .collect();

        if !added.is_empty() {
            out.push_str(&format!("New dependency edges ({}):\n", added.len()));
            for e in &added {
                out.push_str(&format!(
                    "  + {} -> {}  [{}]\n",
                    display_path(&e.from_path),
                    display_path(&e.to_path),
                    e.imported_names.join(", "),
                ));
            }
            out.push('\n');
        }

        if !removed.is_empty() {
            out.push_str(&format!("Removed dependency edges ({}):\n", removed.len()));
            for e in &removed {
                out.push_str(&format!(
                    "  - {} -> {}  [{}]\n",
                    display_path(&e.from_path),
                    display_path(&e.to_path),
                    e.imported_names.join(", "),
                ));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "Summary: {} added, {} removed, {} changed, {} unchanged files\n",
        result.summary.files_added,
        result.summary.files_removed,
        result.summary.files_changed,
        result.summary.files_unchanged,
    ));
    out.push_str(&format!(
        "  {} breaking, {} expanding, {} restructuring changes\n",
        result.summary.breaking_changes,
        result.summary.expanding_changes,
        result.summary.restructuring_changes,
    ));
    if result.summary.import_edges_added > 0 || result.summary.import_edges_removed > 0 {
        out.push_str(&format!(
            "  {} edges added, {} edges removed\n",
            result.summary.import_edges_added, result.summary.import_edges_removed,
        ));
    }

    if !result.cycle_changes.is_empty() {
        use crate::analysis::diff::CycleChangeKind;

        let introduced: Vec<_> = result
            .cycle_changes
            .iter()
            .filter(|c| c.change == CycleChangeKind::Introduced)
            .collect();
        let resolved: Vec<_> = result
            .cycle_changes
            .iter()
            .filter(|c| c.change == CycleChangeKind::Resolved)
            .collect();

        if !introduced.is_empty() {
            out.push_str(&format!(
                "\nNew cycles introduced ({}):\n",
                introduced.len()
            ));
            for c in &introduced {
                let paths: Vec<String> = c.files.iter().map(|p| display_path(p)).collect();
                out.push_str(&format!("  ! {} (length {})\n", paths.join(" -> "), c.length));
            }
        }

        if !resolved.is_empty() {
            out.push_str(&format!("\nCycles resolved ({}):\n", resolved.len()));
            for c in &resolved {
                let paths: Vec<String> = c.files.iter().map(|p| display_path(p)).collect();
                out.push_str(&format!("  * {} (length {})\n", paths.join(" -> "), c.length));
            }
        }

        out.push_str(&format!(
            "  {} introduced, {} resolved\n",
            result.summary.cycles_introduced, result.summary.cycles_resolved,
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::cycles::{Cycle, CycleFile, CycleResult, CycleSummary};
    use crate::analysis::dead_code::{DeadCodeResult, DeadCodeSummary, DeadFile};
    use crate::analysis::dependencies::{DepNode, DepsResult, DepsSummary};
    use crate::analysis::impact::{AffectedFile, ImpactResult, ImpactSummary};
    use crate::analysis::Confidence;

    #[test]
    fn test_format_deps_text_with_imports() {
        let result = DepsResult {
            target_file: FileId(1),
            target_path: PathBuf::from("src/index.ts"),
            imports: vec![
                DepNode {
                    file_id: FileId(2),
                    path: PathBuf::from("src/utils.ts"),
                    depth: 1,
                    imported_names: vec!["helper".to_string()],
                },
                DepNode {
                    file_id: FileId(3),
                    path: PathBuf::from("src/lib.ts"),
                    depth: 2,
                    imported_names: vec![],
                },
            ],
            imported_by: vec![],
            confidence: Confidence::Certain,
            summary: DepsSummary {
                direct_imports: 1,
                transitive_imports: 2,
                direct_importers: 0,
                transitive_importers: 0,
            },
        };

        let text = format_deps_text(&result);
        assert!(text.contains("Dependencies for src/index.ts"));
        assert!(text.contains("Imports (2):"));
        assert!(text.contains("src/utils.ts (helper)"));
        assert!(text.contains("src/lib.ts"));
        assert!(text.contains("Confidence: certain"));
    }

    #[test]
    fn test_format_deps_text_no_deps() {
        let result = DepsResult {
            target_file: FileId(1),
            target_path: PathBuf::from("src/lonely.ts"),
            imports: vec![],
            imported_by: vec![],
            confidence: Confidence::Certain,
            summary: DepsSummary {
                direct_imports: 0,
                transitive_imports: 0,
                direct_importers: 0,
                transitive_importers: 0,
            },
        };

        let text = format_deps_text(&result);
        assert!(text.contains("No dependencies found."));
    }

    #[test]
    fn test_format_dead_code_text() {
        let result = DeadCodeResult {
            dead_files: vec![DeadFile {
                file_id: FileId(3),
                path: PathBuf::from("src/orphan.ts"),
                confidence: Confidence::Certain,
            }],
            dead_exports: vec![],
            confidence: Confidence::Certain,
            limitations: vec![],
            summary: DeadCodeSummary {
                total_files: 3,
                dead_files: 1,
                total_exports: 5,
                dead_exports: 0,
                entry_points: 1,
                files_with_unresolvable_imports: 0,
            },
        };

        let text = format_dead_code_text(&result);
        assert!(text.contains("Dead files (1):"));
        assert!(text.contains("src/orphan.ts [certain]"));
        assert!(text.contains("Summary: 1/3 dead files, 0/5 dead exports, 1 entry points"));
    }

    #[test]
    fn test_format_dead_code_text_clean() {
        let result = DeadCodeResult {
            dead_files: vec![],
            dead_exports: vec![],
            confidence: Confidence::Certain,
            limitations: vec![],
            summary: DeadCodeSummary {
                total_files: 2,
                dead_files: 0,
                total_exports: 3,
                dead_exports: 0,
                entry_points: 1,
                files_with_unresolvable_imports: 0,
            },
        };

        let text = format_dead_code_text(&result);
        assert!(text.contains("No dead code found."));
    }

    #[test]
    fn test_format_cycles_text_no_cycles() {
        let result = CycleResult {
            cycles: vec![],
            confidence: Confidence::Certain,
            summary: CycleSummary {
                total_files: 3,
                files_in_cycles: 0,
                cycle_count: 0,
                shortest_cycle: 0,
                longest_cycle: 0,
            },
        };

        let text = format_cycles_text(&result);
        assert!(text.contains("No circular dependencies found."));
    }

    #[test]
    fn test_format_cycles_text_with_cycle() {
        let result = CycleResult {
            cycles: vec![Cycle {
                files: vec![
                    CycleFile {
                        file_id: FileId(1),
                        path: PathBuf::from("src/a.ts"),
                    },
                    CycleFile {
                        file_id: FileId(2),
                        path: PathBuf::from("src/b.ts"),
                    },
                ],
                length: 2,
            }],
            confidence: Confidence::Certain,
            summary: CycleSummary {
                total_files: 3,
                files_in_cycles: 2,
                cycle_count: 1,
                shortest_cycle: 2,
                longest_cycle: 2,
            },
        };

        let text = format_cycles_text(&result);
        assert!(text.contains("Circular dependencies (1 cycles, 2 files involved):"));
        assert!(text.contains("src/a.ts ->"));
        assert!(text.contains("    src/b.ts\n"));
        assert!(text.contains("-> src/a.ts (cycle back)"));
    }

    #[test]
    fn test_format_impact_text() {
        let mut by_depth = HashMap::new();
        by_depth.insert(
            1,
            vec![AffectedFile {
                file_id: FileId(2),
                path: PathBuf::from("src/a.ts"),
                depth: 1,
            }],
        );
        by_depth.insert(
            2,
            vec![AffectedFile {
                file_id: FileId(3),
                path: PathBuf::from("src/b.ts"),
                depth: 2,
            }],
        );

        let result = ImpactResult {
            target_file: FileId(1),
            target_path: PathBuf::from("src/core.ts"),
            affected: vec![
                AffectedFile {
                    file_id: FileId(2),
                    path: PathBuf::from("src/a.ts"),
                    depth: 1,
                },
                AffectedFile {
                    file_id: FileId(3),
                    path: PathBuf::from("src/b.ts"),
                    depth: 2,
                },
            ],
            by_depth,
            confidence: Confidence::Certain,
            summary: ImpactSummary {
                direct_dependents: 1,
                total_affected: 2,
                max_depth: 2,
            },
        };

        let text = format_impact_text(&result);
        assert!(text.contains("Impact of changing src/core.ts"));
        assert!(text.contains("Affected files (2 total, max depth 2):"));
        assert!(text.contains("Depth 1:"));
        assert!(text.contains("src/a.ts"));
        assert!(text.contains("Depth 2:"));
        assert!(text.contains("src/b.ts"));
    }

    #[test]
    fn test_format_impact_text_no_affected() {
        let result = ImpactResult {
            target_file: FileId(1),
            target_path: PathBuf::from("src/leaf.ts"),
            affected: vec![],
            by_depth: HashMap::new(),
            confidence: Confidence::Certain,
            summary: ImpactSummary {
                direct_dependents: 0,
                total_affected: 0,
                max_depth: 0,
            },
        };

        let text = format_impact_text(&result);
        assert!(text.contains("No files affected."));
    }

    #[test]
    fn test_format_exports_text() {
        let value = serde_json::json!({
            "file": "src/utils.ts",
            "exports": [
                {"name": "helper", "is_default": false, "is_reexport": false, "is_used": true},
                {"name": "unused_fn", "is_default": false, "is_reexport": false, "is_used": false},
                {"name": "default", "is_default": true, "is_reexport": false, "is_used": true},
            ],
            "summary": {"total": 3, "used": 2, "unused": 1}
        });

        let text = format_exports_text(&value);
        assert!(text.contains("Exports for src/utils.ts"));
        assert!(text.contains("Name"));
        assert!(text.contains("helper"));
        assert!(text.contains("unused_fn"));
        assert!(text.contains("NO")); // unused_fn
        assert!(text.contains("Summary: 3 total, 2 used, 1 unused"));
    }

    #[test]
    fn test_format_summary_text() {
        let value = serde_json::json!({
            "files": {
                "total": 10,
                "by_language": {"TypeScript": 8, "JavaScript": 2},
                "entry_points": 2
            },
            "dependencies": {
                "total_imports": 25,
                "external_imports": 5,
                "unresolved_imports": 3
            },
            "dead_code": {
                "dead_files": 1,
                "dead_exports": 4,
                "total_exports": 20
            },
            "cycles": {
                "cycle_count": 1,
                "files_in_cycles": 3
            }
        });

        let text = format_summary_text(&value);
        assert!(text.contains("Project Summary"));
        assert!(text.contains("Files: 10 total, 2 entry points"));
        assert!(text.contains("TypeScript: 8"));
        assert!(text.contains("JavaScript: 2"));
        assert!(text.contains("Dependencies: 25 imports, 5 external, 3 unresolved"));
        assert!(text.contains("Dead code: 1 dead files, 4/20 dead exports"));
        assert!(text.contains("Cycles: 1 cycles, 3 files involved"));
    }

    #[test]
    fn test_format_lint_text_with_violations() {
        use crate::linting::config::Severity;
        use crate::linting::rules::{LintResult, LintSummary, LintViolation};

        let result = LintResult {
            violations: vec![LintViolation {
                rule_id: "no-ui-to-db".to_string(),
                severity: Severity::Error,
                description: "UI must not import DB".to_string(),
                rationale: None,
                source_file: PathBuf::from("src/ui/Button.ts"),
                target_file: PathBuf::from("src/db/connection.ts"),
                imported_names: vec!["getConnection".to_string()],
                line: 5,
                confidence: Confidence::Certain,
                fix_direction: Some("Use service layer".to_string()),
            }],
            rules_evaluated: 1,
            summary: LintSummary {
                total_violations: 1,
                errors: 1,
                warnings: 0,
                infos: 0,
                rules_evaluated: 1,
            },
        };

        let text = format_lint_text(&result);
        assert!(text.contains("error[no-ui-to-db] UI must not import DB"));
        assert!(text.contains("src/ui/Button.ts:5 -> src/db/connection.ts"));
        assert!(text.contains("imports: getConnection"));
        assert!(text.contains("fix: Use service layer"));
        assert!(text.contains("1 errors, 0 warnings across 1 rules"));
    }

    #[test]
    fn test_format_lint_text_no_violations() {
        use crate::linting::rules::{LintResult, LintSummary};

        let result = LintResult {
            violations: vec![],
            rules_evaluated: 2,
            summary: LintSummary {
                total_violations: 0,
                errors: 0,
                warnings: 0,
                infos: 0,
                rules_evaluated: 2,
            },
        };

        let text = format_lint_text(&result);
        assert!(text.contains("No lint violations found."));
        assert!(text.contains("0 errors, 0 warnings across 2 rules"));
    }

    #[test]
    fn test_format_lint_text_fan_limit_no_arrow() {
        use crate::linting::config::Severity;
        use crate::linting::rules::{LintResult, LintSummary, LintViolation};

        let result = LintResult {
            violations: vec![LintViolation {
                rule_id: "no-god-modules".to_string(),
                severity: Severity::Warning,
                description: "Too many dependencies (fan-out 25 exceeds limit 20)".to_string(),
                rationale: None,
                source_file: PathBuf::from("src/app.ts"),
                target_file: PathBuf::from("src/app.ts"),
                imported_names: vec![],
                line: 0,
                confidence: Confidence::Certain,
                fix_direction: Some("Split this file into smaller modules".to_string()),
            }],
            rules_evaluated: 1,
            summary: LintSummary {
                total_violations: 1,
                errors: 0,
                warnings: 1,
                infos: 0,
                rules_evaluated: 1,
            },
        };

        let text = format_lint_text(&result);
        assert!(
            text.contains("warning[no-god-modules] Too many dependencies"),
            "should contain severity and rule description"
        );
        assert!(
            text.contains("  src/app.ts\n"),
            "fan-limit violation should show just the file path, got: {}",
            text
        );
        assert!(
            !text.contains("->"),
            "fan-limit violation should not contain arrow"
        );
        assert!(
            !text.contains(":0"),
            "fan-limit violation should not show line 0"
        );
        assert!(text.contains("fix: Split this file into smaller modules"));
    }

    /// Regression test for bug #32: summary cycle count must match cycles command.
    /// The summary command must filter mod-declaration edges before counting cycles,
    /// just like run_cycles does.
    #[test]
    fn test_summary_cycle_count_matches_cycles_command() {
        use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
        use crate::model::{FileId, Language};

        let mut graph = FileGraph::new();

        // Create files: main -> a <-> b (real cycle), main -> c via mod declaration
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/main.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: true,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/a.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
        });
        graph.add_file(FileInfo {
            id: FileId(3),
            path: PathBuf::from("src/b.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
        });
        graph.add_file(FileInfo {
            id: FileId(4),
            path: PathBuf::from("src/c.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
        });

        // main imports a (normal edge)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        // a <-> b: real cycle
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["b".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 2,
        });
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        // main -> c via mod declaration (should NOT count as cycle edge)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(4),
            imported_names: vec!["c".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 3,
        });
        // c -> main via mod declaration (would be false cycle without filtering)
        graph.add_import(FileImport {
            from: FileId(4),
            to: FileId(1),
            imported_names: vec!["main".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });

        // Simulate what run_cycles does
        let cycles_graph = graph.without_mod_declaration_edges();
        let cycles_result = crate::analysis::cycles::detect_cycles(&cycles_graph);

        // Simulate what run_summary does
        let summary_graph = graph.without_mod_declaration_edges();
        let summary_cycles = crate::analysis::cycles::detect_cycles(&summary_graph);

        assert_eq!(
            cycles_result.cycles.len(),
            summary_cycles.cycles.len(),
            "summary and cycles commands must report the same cycle count"
        );
        assert_eq!(
            cycles_result.summary.files_in_cycles,
            summary_cycles.summary.files_in_cycles,
            "summary and cycles commands must report the same files_in_cycles count"
        );
        // The real cycle is a <-> b (1 cycle, 2 files)
        assert_eq!(cycles_result.cycles.len(), 1);
        assert_eq!(cycles_result.summary.files_in_cycles, 2);
    }

    #[test]
    fn test_unresolved_import_breakdown() {
        use crate::model::file_graph::{FileGraph, FileInfo, UnresolvedImport, UnresolvedReason};
        use crate::model::{FileId, Language};

        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/App.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
        });

        // 2 external imports
        graph.add_unresolved(UnresolvedImport {
            file: FileId(1),
            import_path: "org.springframework.boot".to_string(),
            reason: UnresolvedReason::External("org.springframework.boot".to_string()),
            line: 1,
        });
        graph.add_unresolved(UnresolvedImport {
            file: FileId(1),
            import_path: "java.util.List".to_string(),
            reason: UnresolvedReason::External("java.util.List".to_string()),
            line: 2,
        });

        // 1 FileNotFound
        graph.add_unresolved(UnresolvedImport {
            file: FileId(1),
            import_path: "com.missing.Foo".to_string(),
            reason: UnresolvedReason::FileNotFound("com.missing.Foo".to_string()),
            line: 3,
        });

        // 1 DynamicPath
        graph.add_unresolved(UnresolvedImport {
            file: FileId(1),
            import_path: "dynamic".to_string(),
            reason: UnresolvedReason::DynamicPath,
            line: 4,
        });

        let external_count = graph
            .unresolved
            .iter()
            .filter(|u| matches!(u.reason, UnresolvedReason::External(_)))
            .count();
        let unresolved_count = graph
            .unresolved
            .iter()
            .filter(|u| {
                matches!(
                    u.reason,
                    UnresolvedReason::FileNotFound(_) | UnresolvedReason::DynamicPath
                )
            })
            .count();

        assert_eq!(external_count, 2, "Should have 2 external imports");
        assert_eq!(
            unresolved_count, 2,
            "Should have 2 truly unresolved imports"
        );
        assert_eq!(
            external_count + unresolved_count,
            graph.unresolved.len(),
            "External + unresolved should equal total unresolved entries"
        );
    }

    // =========================================================================
    // Graph command tests
    // =========================================================================

    fn make_test_graph() -> (FileGraph, PathBuf) {
        let root = PathBuf::from("/project");
        let mut graph = FileGraph::new();
        graph.add_file(crate::model::file_graph::FileInfo {
            id: FileId(1),
            path: PathBuf::from("/project/src/main.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: true,
        });
        graph.add_file(crate::model::file_graph::FileInfo {
            id: FileId(2),
            path: PathBuf::from("/project/src/utils.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        });
        graph.add_file(crate::model::file_graph::FileInfo {
            id: FileId(3),
            path: PathBuf::from("/project/src/db.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["helper".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["query".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 2,
        });
        (graph, root)
    }

    #[test]
    fn test_generate_dot_basic() {
        let (graph, root) = make_test_graph();
        let dot = generate_dot(&graph, &root, None);

        assert!(dot.starts_with("digraph dependencies {"));
        assert!(dot.contains("rankdir=LR"));
        assert!(dot.contains("\"src/main.ts\""));
        assert!(dot.contains("\"src/utils.ts\""));
        assert!(dot.contains("\"src/db.ts\""));
        assert!(dot.contains("\"src/main.ts\" -> \"src/utils.ts\""));
        assert!(dot.contains("\"src/utils.ts\" -> \"src/db.ts\""));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn test_generate_dot_entry_point_coloring() {
        let (graph, root) = make_test_graph();
        let dot = generate_dot(&graph, &root, None);

        // main.ts is entry point -> green
        assert!(dot.contains("\"src/main.ts\" [fillcolor=\"#a8d8a8\"]"));
        // utils.ts is not entry point -> default gray
        assert!(dot.contains("\"src/utils.ts\" [fillcolor=\"#e8e8e8\"]"));
    }

    #[test]
    fn test_generate_dot_focus_coloring() {
        let (graph, root) = make_test_graph();
        let dot = generate_dot(&graph, &root, Some(FileId(2)));

        // Focus file gets blue color
        assert!(dot.contains("\"src/utils.ts\" [fillcolor=\"#8888ff\"]"));
        // Entry point still green (not focus)
        assert!(dot.contains("\"src/main.ts\" [fillcolor=\"#a8d8a8\"]"));
    }

    #[test]
    fn test_extract_subgraph_depth_1() {
        let (graph, _root) = make_test_graph();
        // Focus on utils.ts (FileId(2)) with depth 1
        let sub = extract_subgraph(&graph, FileId(2), Some(1));

        // Should include: utils(2), main(1) because main imports utils,
        // and db(3) because utils imports db
        assert_eq!(sub.file_count(), 3);
        assert!(sub.get_file(FileId(1)).is_some());
        assert!(sub.get_file(FileId(2)).is_some());
        assert!(sub.get_file(FileId(3)).is_some());
    }

    #[test]
    fn test_extract_subgraph_depth_limited() {
        // Build a longer chain: A -> B -> C -> D
        let mut graph = FileGraph::new();
        for i in 1..=4u64 {
            graph.add_file(crate::model::file_graph::FileInfo {
                id: FileId(i),
                path: PathBuf::from(format!("/project/f{}.ts", i)),
                language: Language::TypeScript,
                exports: vec![],
                is_entry_point: i == 1,
            });
        }
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["b".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(3),
            to: FileId(4),
            imported_names: vec!["c".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        // Focus on FileId(2) with depth 1: should get 1, 2, 3 but NOT 4
        let sub = extract_subgraph(&graph, FileId(2), Some(1));
        assert_eq!(sub.file_count(), 3);
        assert!(sub.get_file(FileId(1)).is_some());
        assert!(sub.get_file(FileId(2)).is_some());
        assert!(sub.get_file(FileId(3)).is_some());
        assert!(sub.get_file(FileId(4)).is_none());
    }

    #[test]
    fn test_build_graph_json_structure() {
        let (graph, root) = make_test_graph();
        let json = build_graph_json(&graph, &root, None);

        let nodes = json["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);

        let edges = json["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);

        // Check summary
        assert_eq!(json["summary"]["node_count"], 3);
        assert_eq!(json["summary"]["edge_count"], 2);

        // Check node structure
        let node0 = &nodes[0];
        assert!(node0["path"].as_str().is_some());
        assert!(node0["is_entry_point"].as_bool().is_some());
        assert!(node0["language"].as_str().is_some());

        // Check edge structure
        let edge0 = &edges[0];
        assert!(edge0["from"].as_str().is_some());
        assert!(edge0["to"].as_str().is_some());
        assert!(edge0["imported_names"].as_array().is_some());
    }

    #[test]
    fn test_build_graph_json_focus_flag() {
        let (graph, root) = make_test_graph();
        let json = build_graph_json(&graph, &root, Some(FileId(2)));

        let nodes = json["nodes"].as_array().unwrap();
        let focus_node = nodes.iter().find(|n| n["path"] == "src/utils.ts").unwrap();
        assert_eq!(focus_node["is_focus"], true);

        // Non-focus node should not have is_focus
        let other_node = nodes.iter().find(|n| n["path"] == "src/main.ts").unwrap();
        assert!(other_node.get("is_focus").is_none());
    }

    #[test]
    fn test_generate_html_contains_json() {
        let (graph, root) = make_test_graph();
        let html = generate_html(&graph, &root, None);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("const graphData ="));
        assert!(html.contains("src/main.ts"));
        assert!(html.contains("src/utils.ts"));
        assert!(html.contains("requestAnimationFrame"));
    }
}
