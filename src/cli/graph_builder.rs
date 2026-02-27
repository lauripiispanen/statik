use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::db::Database;
use crate::model::file_graph::{
    FileGraph, FileImport, FileInfo, UnresolvedImport, UnresolvedReason,
};
use crate::model::graph::SymbolGraph;
use crate::model::{FileId, Language, ParseResult};
use crate::resolver::java::JavaResolver;
use crate::resolver::rust::RustResolver;
use crate::resolver::typescript::TypeScriptResolver;
use crate::resolver::{Resolution, Resolver};

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

    // Load source set config for visibility filtering
    let source_set_configs = crate::linting::config::load_source_set_config(project_root);
    let source_set_index = if !source_set_configs.is_empty() {
        let index = crate::resolver::source_sets::SourceSetIndex::build(
            &source_set_configs,
            project_root,
        )?;
        // Pass a clone to the Java resolver for scoped same-package resolution
        java_resolver.set_source_set_index(index.clone());
        Some(index)
    } else {
        None
    };

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

    // Post-filter edges by source set visibility
    if let Some(ref index) = source_set_index {
        graph = graph.filter_by_source_sets(index);
    }

    Ok(graph)
}

/// Build a SymbolGraph from the database (symbols + resolved references).
pub fn build_symbol_graph(db: &Database) -> Result<SymbolGraph> {
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
pub fn build_seed_all_file_ids(file_graph: &FileGraph, project_root: &Path) -> HashSet<FileId> {
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

/// Apply --runtime-only filtering if requested.
pub fn maybe_filter_type_only(graph: FileGraph, runtime_only: bool) -> FileGraph {
    if runtime_only {
        graph.without_type_only_edges()
    } else {
        graph
    }
}

/// Apply --path glob filtering if requested.
pub fn maybe_filter_paths(
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
