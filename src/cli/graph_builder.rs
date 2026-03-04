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

    // Load scope config for file classification
    let scope_config = crate::linting::config::load_scope_config(project_root);
    let scope_index = if !scope_config.is_empty() {
        Some(crate::linting::config::ScopeIndex::build(&scope_config)?)
    } else {
        None
    };

    // Load source set config for visibility filtering
    let source_set_configs = crate::linting::config::load_source_set_config(project_root);
    let source_set_index = if !source_set_configs.is_empty() {
        let index =
            crate::resolver::source_sets::SourceSetIndex::build(&source_set_configs, project_root)?;
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

    // Batch-load all imports, exports, suppressions, and SCIP enrichment status
    let all_imports = db.all_imports()?;
    let all_exports = db.all_exports()?;
    let all_suppressions = db.all_suppressions()?;
    let scip_enriched_ids = db.scip_enriched_file_ids().unwrap_or_default();

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
                    if lang_annotations.contains(&ann)
                        || ep_config.annotations.iter().any(|a| a == ann)
                    {
                        annotation_entry_files.insert(file.id);
                        break;
                    }
                }
            }
        }
    }

    // Check if there are any Java files (for auto-detection fallback)
    let has_java_files = files.iter().any(|f| f.language == Language::Java);

    // Add files to the graph
    for file in &files {
        let exports = exports_by_file.remove(&file.id).unwrap_or_default();
        let rel_path = crate::linting::matcher::to_relative(&file.path, project_root);
        let lang_sem = semantics.get(&file.language).copied();
        let file_source_set = scope_index
            .as_ref()
            .and_then(|idx| idx.classify(rel_path).map(|s| s.to_string()))
            .or_else(|| {
                // Auto-detect Java test directories when no scope config is present
                if scope_index.is_none() && has_java_files && file.language == Language::Java {
                    auto_detect_java_source_set(rel_path)
                } else {
                    None
                }
            });
        let is_entry = is_entry_point(&file.path, lang_sem)
            || annotation_entry_files.contains(&file.id)
            || custom_pattern_matcher
                .as_ref()
                .is_some_and(|m| m.matches(rel_path))
            || file_source_set.as_ref().is_some_and(|set| {
                scope_index
                    .as_ref()
                    .is_some_and(|idx| idx.has_role(set, "entry_point"))
            })
            // Auto-detected Java test files are entry points
            || (scope_index.is_none()
                && file_source_set.as_deref() == Some("test"));

        let file_suppressions = all_suppressions.get(&file.id).cloned().unwrap_or_default();
        graph.add_file(FileInfo {
            id: file.id,
            path: file.path.clone(),
            language: file.language,
            exports,
            is_entry_point: is_entry,
            suppressions: file_suppressions,
            source_set: file_source_set,
        });
    }

    // Set SCIP enrichment info on the graph
    graph.scip_enriched = scip_enriched_ids;

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

            // Skip imports from #[cfg(test)] blocks -- they are test-only
            // and should not create production dependency edges.
            if import.is_cfg_test {
                continue;
            }

            let is_mod = import.source_path.starts_with("@mod:");

            let lang = file_language
                .get(&file.id)
                .copied()
                .unwrap_or(Language::TypeScript);
            let resolution: Resolution = if let Some(type_name) =
                import.source_path.strip_prefix("@type-ref:")
            {
                java_resolver.resolve_type_ref(type_name, &file.path)
            } else if import.is_namespace && lang == Language::Java {
                // Wildcard import: resolve to all files in the package
                let files = java_resolver.resolve_wildcard_scoped(&import.source_path, &file.path);
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
            let line = imports_meta
                .iter()
                .map(|(_, _, l, _)| *l)
                .min()
                .unwrap_or(0);
            graph.add_import(FileImport {
                from: file.id,
                to: target_id,
                imported_names: names,
                is_type_only,
                is_mod_declaration,
                is_scip_derived: false,
                line,
            });
        }
    }

    // Add SCIP-derived cross-file edges that aren't already covered by import edges
    if let Ok(scip_edges) = db.scip_cross_file_edges() {
        if !scip_edges.is_empty() {
            // Build set of existing (from, to) pairs to dedup
            let existing_pairs: std::collections::HashSet<(FileId, FileId)> = graph
                .imports
                .values()
                .flat_map(|edges| edges.iter().map(|e| (e.from, e.to)))
                .collect();

            for (from, to) in scip_edges {
                if !existing_pairs.contains(&(from, to))
                    && graph.files.contains_key(&from)
                    && graph.files.contains_key(&to)
                {
                    graph.add_import(FileImport {
                        from,
                        to,
                        imported_names: vec![],
                        is_type_only: false,
                        is_mod_declaration: false,
                        is_scip_derived: true,
                        line: 0,
                    });
                }
            }
        }
    }

    replace_tree_sitter_edges_for_enriched_files(&mut graph);

    // Post-filter edges by source set visibility
    if let Some(ref index) = source_set_index {
        graph = graph.filter_by_source_sets(index);
    }

    // Record which source sets have analysis disabled so callers can
    // filter without re-reading the config file.
    if let Some(ref idx) = scope_index {
        for name in idx.analysis_disabled_set_names() {
            graph.analysis_disabled_sets.insert(name.to_string());
        }
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
                suppressions: HashMap::new(),
            });
        }
    }

    Ok(graph)
}

/// Check if a file is an entry point.
///
/// Universal patterns (index, main, app, server, cli) are checked first.
/// Language-specific patterns are delegated to `LanguageSemantics`.
fn is_entry_point(path: &Path, semantics: Option<&dyn crate::model::LanguageSemantics>) -> bool {
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

/// Apply --scope filtering if requested.
///
/// Restricts the graph to files belonging to the named source set.
/// Returns an error if the scope name is not found in any file.
pub fn maybe_filter_scope(graph: FileGraph, scope: Option<&str>) -> Result<FileGraph> {
    match scope {
        Some(name) => {
            let available = graph.available_scopes();
            if !available.iter().any(|s| s == name) {
                if available.is_empty() {
                    anyhow::bail!(
                        "Unknown scope '{}'. No source sets are configured. \
                         Add [scope.<name>] sections to .statik/rules.toml.",
                        name,
                    );
                } else {
                    anyhow::bail!(
                        "Unknown scope '{}'. Available scopes: {}",
                        name,
                        available.join(", "),
                    );
                }
            }
            Ok(graph.filter_to_scope(name))
        }
        None => Ok(graph),
    }
}

/// Build a set of file IDs excluded from analysis output.
///
/// Files in source sets with `analysis = false` should not appear in
/// analysis command output (dead-code, deps, cycles, etc.) but remain
/// in the graph for correct resolution.
///
/// Uses the `analysis_disabled_sets` cached in the graph during construction,
/// avoiding redundant config I/O.
pub fn analysis_excluded_files(graph: &FileGraph) -> HashSet<FileId> {
    if graph.analysis_disabled_sets.is_empty() {
        return HashSet::new();
    }
    graph
        .files
        .iter()
        .filter_map(|(file_id, info)| {
            info.source_set
                .as_ref()
                .filter(|set| graph.analysis_disabled_sets.contains(set.as_str()))
                .map(|_| *file_id)
        })
        .collect()
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

/// For SCIP-enriched files, replace outgoing tree-sitter edges with SCIP edges.
///
/// Tree-sitter edges include false positives (unused imports, wrong wildcard
/// resolution), so enriched files should only keep SCIP-derived edges.
/// Non-enriched files keep all their tree-sitter edges unchanged.
fn replace_tree_sitter_edges_for_enriched_files(graph: &mut FileGraph) {
    if graph.scip_enriched.is_empty() {
        return;
    }

    for &file_id in &graph.scip_enriched.clone() {
        // Remove outgoing tree-sitter edges from this enriched file
        if let Some(edges) = graph.imports.get_mut(&file_id) {
            // Collect targets of tree-sitter edges being removed, for imported_by cleanup
            let removed_targets: Vec<FileId> = edges
                .iter()
                .filter(|e| !e.is_scip_derived)
                .map(|e| e.to)
                .collect();

            // Keep only SCIP-derived edges
            edges.retain(|e| e.is_scip_derived);

            // Clean up reverse index for removed edges
            for target_id in removed_targets {
                if let Some(rev_edges) = graph.imported_by.get_mut(&target_id) {
                    rev_edges.retain(|e| e.from != file_id || e.is_scip_derived);
                }
            }
        }
    }

    // Remove unresolved imports for enriched files
    graph
        .unresolved
        .retain(|u| !graph.scip_enriched.contains(&u.file));
}

/// Auto-detect Java source set from directory conventions.
///
/// When no explicit `[scope]` config is present, Java files under
/// `src/test/java` are classified as "test" source set and files
/// under `src/main/java` as "production". This matches the standard
/// Maven/Gradle project layout.
fn auto_detect_java_source_set(rel_path: &Path) -> Option<String> {
    let components: Vec<_> = rel_path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    // Look for src/test/java sequence anywhere in the path
    for window in components.windows(3) {
        if window[0] == "src" && window[1] == "test" && window[2] == "java" {
            return Some("test".to_string());
        }
        if window[0] == "src" && window[1] == "main" && window[2] == "java" {
            return Some("production".to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_auto_detect_java_test_source_set() {
        assert_eq!(
            auto_detect_java_source_set(Path::new("src/test/java/com/example/FooTest.java")),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_auto_detect_java_main_source_set() {
        assert_eq!(
            auto_detect_java_source_set(Path::new("src/main/java/com/example/Foo.java")),
            Some("production".to_string())
        );
    }

    #[test]
    fn test_auto_detect_java_multi_module_test() {
        // Multi-module Maven layout: module/src/test/java/...
        assert_eq!(
            auto_detect_java_source_set(Path::new(
                "api-service/src/test/java/com/example/ApiTest.java"
            )),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_auto_detect_java_multi_module_main() {
        assert_eq!(
            auto_detect_java_source_set(Path::new(
                "api-service/src/main/java/com/example/Api.java"
            )),
            Some("production".to_string())
        );
    }

    #[test]
    fn test_auto_detect_java_non_standard_returns_none() {
        // Non-standard layout: no classification
        assert_eq!(
            auto_detect_java_source_set(Path::new("src/com/example/Foo.java")),
            None
        );
    }

    #[test]
    fn test_auto_detect_java_flat_layout_returns_none() {
        assert_eq!(
            auto_detect_java_source_set(Path::new("com/example/Foo.java")),
            None
        );
    }

    #[test]
    fn test_auto_detect_java_partial_match_returns_none() {
        // Only two of three components match
        assert_eq!(
            auto_detect_java_source_set(Path::new("src/test/Foo.java")),
            None
        );
        assert_eq!(
            auto_detect_java_source_set(Path::new("src/main/Foo.java")),
            None
        );
    }

    use crate::model::file_graph::{
        FileGraph, FileImport, FileInfo, UnresolvedImport, UnresolvedReason,
    };
    use crate::model::{FileId, Language};

    fn make_file_info(id: FileId, name: &str) -> FileInfo {
        FileInfo {
            id,
            path: PathBuf::from(name),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        }
    }

    fn make_edge(from: FileId, to: FileId, scip: bool) -> FileImport {
        FileImport {
            from,
            to,
            imported_names: vec!["Foo".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: scip,
            line: 1,
        }
    }

    #[test]
    fn test_scip_replacement_enriched_file_loses_tree_sitter_edges() {
        let mut graph = FileGraph::new();
        let a = FileId(1);
        let b = FileId(2);
        let c = FileId(3);

        graph.add_file(make_file_info(a, "A.java"));
        graph.add_file(make_file_info(b, "B.java"));
        graph.add_file(make_file_info(c, "C.java"));

        // A has a tree-sitter edge to B and a SCIP edge to C
        graph.add_import(make_edge(a, b, false));
        graph.add_import(make_edge(a, c, true));

        // Mark A as SCIP-enriched
        graph.scip_enriched.insert(a);

        replace_tree_sitter_edges_for_enriched_files(&mut graph);

        // A should only have the SCIP edge to C
        let a_edges = &graph.imports[&a];
        assert_eq!(a_edges.len(), 1);
        assert_eq!(a_edges[0].to, c);
        assert!(a_edges[0].is_scip_derived);
    }

    #[test]
    fn test_scip_replacement_non_enriched_file_keeps_tree_sitter_edges() {
        let mut graph = FileGraph::new();
        let a = FileId(1);
        let b = FileId(2);

        graph.add_file(make_file_info(a, "A.java"));
        graph.add_file(make_file_info(b, "B.java"));

        // A has a tree-sitter edge to B
        graph.add_import(make_edge(a, b, false));

        // Mark some other file as enriched (not A)
        graph.scip_enriched.insert(FileId(99));

        replace_tree_sitter_edges_for_enriched_files(&mut graph);

        // A should still have its tree-sitter edge
        let a_edges = &graph.imports[&a];
        assert_eq!(a_edges.len(), 1);
        assert_eq!(a_edges[0].to, b);
        assert!(!a_edges[0].is_scip_derived);
    }

    #[test]
    fn test_scip_replacement_removes_unresolved_imports_for_enriched_files() {
        let mut graph = FileGraph::new();
        let a = FileId(1);
        let b = FileId(2);

        graph.add_file(make_file_info(a, "A.java"));
        graph.add_file(make_file_info(b, "B.java"));

        // Add unresolved imports for both files
        graph.add_unresolved(UnresolvedImport {
            file: a,
            import_path: "com.example.Missing".to_string(),
            reason: UnresolvedReason::FileNotFound("not found".to_string()),
            line: 5,
        });
        graph.add_unresolved(UnresolvedImport {
            file: b,
            import_path: "com.example.Other".to_string(),
            reason: UnresolvedReason::FileNotFound("not found".to_string()),
            line: 10,
        });

        // Only A is enriched
        graph.scip_enriched.insert(a);

        replace_tree_sitter_edges_for_enriched_files(&mut graph);

        // Only B's unresolved import should remain
        assert_eq!(graph.unresolved.len(), 1);
        assert_eq!(graph.unresolved[0].file, b);
    }

    #[test]
    fn test_scip_replacement_imported_by_stays_consistent() {
        let mut graph = FileGraph::new();
        let a = FileId(1);
        let b = FileId(2);
        let c = FileId(3);

        graph.add_file(make_file_info(a, "A.java"));
        graph.add_file(make_file_info(b, "B.java"));
        graph.add_file(make_file_info(c, "C.java"));

        // A (enriched) -> B via tree-sitter, A -> C via SCIP
        // C (non-enriched) -> B via tree-sitter
        graph.add_import(make_edge(a, b, false));
        graph.add_import(make_edge(a, c, true));
        graph.add_import(make_edge(c, b, false));

        graph.scip_enriched.insert(a);

        replace_tree_sitter_edges_for_enriched_files(&mut graph);

        // B's imported_by should only have C (A's tree-sitter edge was removed)
        let b_imported_by = &graph.imported_by[&b];
        assert_eq!(b_imported_by.len(), 1);
        assert_eq!(b_imported_by[0].from, c);

        // C's imported_by should still have A (SCIP edge kept)
        let c_imported_by = &graph.imported_by[&c];
        assert_eq!(c_imported_by.len(), 1);
        assert_eq!(c_imported_by[0].from, a);
        assert!(c_imported_by[0].is_scip_derived);
    }

    #[test]
    fn test_scip_replacement_no_enriched_files_is_noop() {
        let mut graph = FileGraph::new();
        let a = FileId(1);
        let b = FileId(2);

        graph.add_file(make_file_info(a, "A.java"));
        graph.add_file(make_file_info(b, "B.java"));

        graph.add_import(make_edge(a, b, false));

        // No enriched files
        replace_tree_sitter_edges_for_enriched_files(&mut graph);

        assert_eq!(graph.imports[&a].len(), 1);
        assert_eq!(graph.imported_by[&b].len(), 1);
    }
}
