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
use crate::model::FileId;
use crate::resolver::typescript::TypeScriptResolver;
use crate::resolver::{Resolution, Resolver};

use super::OutputFormat;

/// Build a FileGraph from the database and resolver.
pub fn build_file_graph(db: &Database, project_root: &Path) -> Result<FileGraph> {
    let files = db.all_files()?;
    let mut graph = FileGraph::new();

    // Collect all known file paths for the resolver
    let known_paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
    let resolver = TypeScriptResolver::new_auto(project_root.to_path_buf(), known_paths);

    // Build path -> FileId lookup
    let path_to_id: HashMap<PathBuf, FileId> =
        files.iter().map(|f| (f.path.clone(), f.id)).collect();

    // Add files to the graph
    for file in &files {
        let exports = db.get_exports_by_file(file.id)?;
        let is_entry = is_entry_point(&file.path);

        graph.add_file(FileInfo {
            id: file.id,
            path: file.path.clone(),
            language: file.language,
            exports,
            is_entry_point: is_entry,
        });
    }

    // Resolve imports and add edges
    for file in &files {
        let imports = db.get_imports_by_file(file.id)?;

        // Group imports by source_path to create one edge per target file
        let mut edges_by_target: HashMap<FileId, Vec<String>> = HashMap::new();

        for import in &imports {
            let resolution = resolver.resolve(&import.source_path, &file.path);

            match resolution {
                Resolution::Resolved(resolved_path)
                | Resolution::ResolvedWithCaveat(resolved_path, _) => {
                    if let Some(&target_id) = path_to_id.get(&resolved_path) {
                        edges_by_target
                            .entry(target_id)
                            .or_default()
                            .push(import.imported_name.clone());
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
        for (target_id, names) in edges_by_target {
            graph.add_import(FileImport {
                from: file.id,
                to: target_id,
                imported_names: names,
                is_type_only: false,
                line: 0,
            });
        }
    }

    Ok(graph)
}

/// Check if a file is an entry point.
fn is_entry_point(path: &Path) -> bool {
    let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let file_name_with_ext = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

    let entry_patterns = ["index", "main", "app", "server", "cli"];
    for pattern in &entry_patterns {
        if file_name == *pattern {
            return true;
        }
    }

    // Test files are entry points
    if file_name_with_ext.contains(".test.")
        || file_name_with_ext.contains(".spec.")
        || file_name.ends_with("_test")
        || file_name.ends_with("_spec")
    {
        return true;
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

/// Run the `deps` command.
pub fn run_deps(
    project_path: &Path,
    file_path: &str,
    transitive: bool,
    direction_str: &str,
    max_depth: Option<usize>,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

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

    Ok(format_analysis_output(&result, "deps", format))
}

/// Run the `dead-code` command.
pub fn run_dead_code(
    project_path: &Path,
    scope_str: &str,
    format: &OutputFormat,
    no_index: bool,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

    let scope = match scope_str {
        "files" => DeadCodeScope::Files,
        "exports" => DeadCodeScope::Exports,
        _ => DeadCodeScope::Both,
    };

    let result = detect_dead_code(&graph, scope);
    Ok(format_analysis_output(&result, "dead-code", format))
}

/// Run the `cycles` command.
pub fn run_cycles(project_path: &Path, format: &OutputFormat, no_index: bool) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

    let result = detect_cycles(&graph);
    Ok(format_analysis_output(&result, "cycles", format))
}

/// Run the `impact` command.
pub fn run_impact(
    project_path: &Path,
    file_path: &str,
    max_depth: Option<usize>,
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

    let result =
        analyze_impact(&graph, target_id, max_depth).context("Failed to analyze impact")?;

    Ok(format_analysis_output(&result, "impact", format))
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

    Ok(format_analysis_output(&result, "exports", format))
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

    Ok(format_analysis_output(&result, "summary", format))
}

/// Format any serializable analysis result.
fn format_analysis_output<T: serde::Serialize>(
    value: &T,
    _command_name: &str,
    format: &OutputFormat,
) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(value).unwrap_or_default(),
        OutputFormat::Compact => serde_json::to_string(value).unwrap_or_default(),
        OutputFormat::Text => {
            // For text output, use pretty JSON as fallback
            // Commands that want custom text formatting override this
            serde_json::to_string_pretty(value).unwrap_or_default()
        }
    }
}
