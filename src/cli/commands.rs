use std::collections::HashMap;
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
        Some(crate::linting::matcher::FileMatcher::new(&ep_config.patterns)?)
    };

    // Collect all known file paths for resolvers
    let known_paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
    let ts_resolver = TypeScriptResolver::new_auto(project_root.to_path_buf(), known_paths.clone());
    let java_resolver = JavaResolver::new(project_root.to_path_buf(), known_paths);

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

    // Pre-scan Java files for annotation-based entry points
    let mut annotation_entry_files: std::collections::HashSet<FileId> =
        std::collections::HashSet::new();
    for file in &files {
        if file.language == Language::Java {
            if let Some(imports) = imports_by_file.get(&file.id) {
                for import in imports {
                    if let Some(ann) = import.source_path.strip_prefix("@annotation:") {
                        if is_entry_point_annotation(ann)
                            || ep_config.annotations.iter().any(|a| a == ann)
                        {
                            annotation_entry_files.insert(file.id);
                            break;
                        }
                    }
                }
            }
        }
    }

    // Add files to the graph
    for file in &files {
        let exports = exports_by_file.remove(&file.id).unwrap_or_default();
        let rel_path = crate::linting::matcher::to_relative(&file.path, project_root);
        let is_entry = is_entry_point(&file.path)
            || annotation_entry_files.contains(&file.id)
            || custom_pattern_matcher
                .as_ref()
                .map_or(false, |m| m.matches(rel_path));

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

        // Group imports by target file, tracking metadata per import
        let mut edges_by_target: HashMap<FileId, Vec<(String, bool, usize)>> = HashMap::new();

        for import in &imports {
            // Skip annotation marker imports (handled during entry point detection)
            if import.source_path.starts_with("@annotation:") {
                continue;
            }

            let lang = file_language.get(&file.id).copied().unwrap_or(Language::TypeScript);
            let resolution: Resolution = if let Some(type_name) =
                import.source_path.strip_prefix("@type-ref:")
            {
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
                                ));
                            }
                        }
                    }
                    continue;
                }
            } else {
                match lang {
                    Language::Java => java_resolver.resolve(&import.source_path, &file.path),
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
            let names: Vec<String> = imports_meta.iter().map(|(n, _, _)| n.clone()).collect();
            // Edge is type-only only if ALL grouped imports are type-only
            let is_type_only = imports_meta.iter().all(|(_, t, _)| *t);
            // Use the earliest line number
            let line = imports_meta.iter().map(|(_, _, l)| *l).min().unwrap_or(0);
            graph.add_import(FileImport {
                from: file.id,
                to: target_id,
                imported_names: names,
                is_type_only,
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

    for file in &all_files {
        let symbols = file_symbols.remove(&file.id).unwrap_or_default();
        let references = file_refs.remove(&file.id).unwrap_or_default();
        if !symbols.is_empty() || !references.is_empty() {
            graph.add_parse_result(ParseResult {
                file_id: file.id,
                symbols,
                references,
                imports: vec![],
                exports: vec![],
                type_references: vec![],
                annotations: vec![],
            });
        }
    }

    Ok(graph)
}

const ENTRY_POINT_ANNOTATIONS: &[&str] = &[
    "SpringBootApplication",
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "Component",
    "Service",
    "Repository",
    "Controller",
    "RestController",
    "Configuration",
    "Bean",
    "Endpoint",
    "WebServlet",
];

fn is_entry_point_annotation(name: &str) -> bool {
    ENTRY_POINT_ANNOTATIONS.contains(&name)
}

/// Check if a file is an entry point.
fn is_entry_point(path: &Path) -> bool {
    let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let file_name_with_ext = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let entry_patterns = ["index", "main", "app", "server", "cli"];
    for pattern in &entry_patterns {
        if file_name == *pattern {
            return true;
        }
    }

    // TS/JS test files are entry points
    if file_name_with_ext.contains(".test.")
        || file_name_with_ext.contains(".spec.")
        || file_name.ends_with("_test")
        || file_name.ends_with("_spec")
    {
        return true;
    }

    // Java-specific entry points
    if ext == "java" {
        // JUnit test conventions
        if file_name.ends_with("Test")
            || file_name.ends_with("Tests")
            || file_name.ends_with("IT")
            || file_name.starts_with("Test")
        {
            return true;
        }
        // Spring Boot entry point
        if file_name == "Application" {
            return true;
        }
    }

    false
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

/// Run the `deps` command.
pub fn run_deps(
    project_path: &Path,
    file_path: &str,
    transitive: bool,
    direction_str: &str,
    max_depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);

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

/// Run the `dead-code` command.
pub fn run_dead_code(
    project_path: &Path,
    scope_str: &str,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    if scope_str == "symbols" {
        // Symbol-level dead code analysis
        let file_graph = build_file_graph(&db, project_path)?;
        let symbol_graph = build_symbol_graph(&db)?;
        let result =
            crate::analysis::dead_code::detect_dead_symbols(&symbol_graph, &file_graph);
        return Ok(match format {
            OutputFormat::Text => format_dead_symbols_text(&result),
            _ => format_json(&result, format),
        });
    }

    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);

    let scope = match scope_str {
        "files" => DeadCodeScope::Files,
        "exports" => DeadCodeScope::Exports,
        _ => DeadCodeScope::Both,
    };

    let result = detect_dead_code(&graph, scope);
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
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);

    let result = detect_cycles(&graph);
    Ok(match format {
        OutputFormat::Text => format_cycles_text(&result),
        _ => format_json(&result, format),
    })
}

/// Run the `impact` command.
pub fn run_impact(
    project_path: &Path,
    file_path: &str,
    max_depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);

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
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

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
    for edges in graph.imports.values() {
        for edge in edges {
            if edge.to == target_id {
                for name in &edge.imported_names {
                    used_exports.insert(name.clone());
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
pub fn run_summary(project_path: &Path, format: &OutputFormat, no_index: bool) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

    let dead = detect_dead_code(&graph, DeadCodeScope::Both);
    let cycles = detect_cycles(&graph);

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
            unresolved_imports: graph.unresolved.len(),
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

/// Run the `lint` command. Returns (output_string, has_errors).
pub fn run_lint(
    project_path: &Path,
    config_path: Option<&str>,
    rule_filter: Option<&str>,
    severity_threshold: &str,
    format: &OutputFormat,
    no_index: bool,
) -> Result<(String, bool)> {
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

    let mut result = evaluate_rules(&config, &graph, project_path)?;

    // Filter by severity threshold
    result.violations.retain(|v| match threshold {
        Severity::Error => v.severity == Severity::Error,
        Severity::Warning => v.severity == Severity::Error || v.severity == Severity::Warning,
        Severity::Info => true,
    });

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
pub fn run_diff(
    project_path: &Path,
    before_path: &str,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    use crate::analysis::diff::compare_snapshots;

    let db_before = Database::open(std::path::Path::new(before_path))
        .context(format!("Failed to open baseline database: {}", before_path))?;
    let db_after = ensure_index(project_path, no_index)?;

    let result = compare_snapshots(&db_before, &db_after)?;

    Ok(match format {
        OutputFormat::Text => format_diff_text(&result),
        _ => format_json(&result, format),
    })
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
            let sk: SymbolKind = k
                .parse()
                .map_err(|e: String| anyhow::anyhow!("{}", e))?;
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
    let file_paths: HashMap<FileId, PathBuf> = all_files.iter().map(|f| (f.id, f.path.clone())).collect();

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
            let vis = sym.get("visibility").and_then(|v| v.as_str()).unwrap_or("?");
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
    let file_paths: HashMap<FileId, PathBuf> = all_files.iter().map(|f| (f.id, f.path.clone())).collect();

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
            let matches_kind = kind_filter.map_or(true, |k| r.kind == k);
            let matches_file = file_filter_id.map_or(true, |fid| r.file == fid);
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
    }

    let ref_infos: Vec<RefInfo> = matching_refs
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
            }
        })
        .collect();

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

    let symbol = value
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
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
                out.push_str(&format!(
                    "  {} -> {} [{}] at {}:{}\n",
                    source, target, kind, file, line
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
    let file_paths: HashMap<FileId, PathBuf> = all_files.iter().map(|f| (f.id, f.path.clone())).collect();

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
                && file_filter_id.map_or(true, |fid| r.file == fid)
        })
        .collect();

    #[derive(serde::Serialize)]
    struct CallerInfo {
        caller: String,
        kind: String,
        file: String,
        line: usize,
    }

    let caller_infos: Vec<CallerInfo> = callers
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
            }
        })
        .collect();

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
                    out.push_str(&format!("  {} at {}:{}\n", c.caller, c.file, c.line));
                }
            }
            out
        }
        _ => format_json(&result, format),
    })
}

/// Format any serializable analysis result as JSON.
fn format_json<T: serde::Serialize>(value: &T, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(value).unwrap_or_default(),
        OutputFormat::Compact => serde_json::to_string(value).unwrap_or_default(),
        OutputFormat::Text => unreachable!("text format should be handled by caller"),
    }
}

/// Strip a common project root prefix from a path for display.
fn display_path(path: &Path) -> String {
    path.display().to_string()
}

// --- Text formatters for each command ---

fn format_deps_text(result: &crate::analysis::dependencies::DepsResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("Dependencies for {}\n", display_path(&result.target_path)));
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
                s.name, s.kind, s.file, s.line, format!("{}", s.confidence),
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
                    out.push_str(&format!("    {} ->\n", display_path(&file.path)));
                    out.push_str(&format!(
                        "    {} (cycle)\n",
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
        let total = summary
            .get("total")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let used = summary.get("used").and_then(|v| v.as_u64()).unwrap_or(0);
        let unused = summary
            .get("unused")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
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
            langs.sort_by(|a, b| {
                b.1.as_u64()
                    .unwrap_or(0)
                    .cmp(&a.1.as_u64().unwrap_or(0))
            });
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
        let unresolved = deps
            .get("unresolved_imports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        out.push_str(&format!(
            "Dependencies: {} imports, {} unresolved\n",
            total, unresolved,
        ));
    }

    if let Some(dc) = result.get("dead_code") {
        let dead_files = dc.get("dead_files").and_then(|v| v.as_u64()).unwrap_or(0);
        let dead_exports = dc
            .get("dead_exports")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
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
        let count = cy
            .get("cycle_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
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
    use crate::analysis::diff::ChangeKind;

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

        if !breaking.is_empty() {
            out.push_str(&format!("Breaking changes ({}):\n", breaking.len()));
            for c in &breaking {
                out.push_str(&format!(
                    "  - {}  {}  ({})\n",
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

    out.push_str(&format!(
        "Summary: {} added, {} removed, {} changed, {} unchanged files\n",
        result.summary.files_added,
        result.summary.files_removed,
        result.summary.files_changed,
        result.summary.files_unchanged,
    ));
    out.push_str(&format!(
        "  {} breaking, {} expanding changes",
        result.summary.breaking_changes, result.summary.expanding_changes,
    ));

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
        assert!(text.contains("src/b.ts ->"));
        assert!(text.contains("src/a.ts (cycle)"));
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
        assert!(text.contains("Dependencies: 25 imports, 3 unresolved"));
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
}
