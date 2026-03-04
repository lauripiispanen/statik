use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::analysis::cycles::detect_cycles;
use crate::analysis::dead_code::{detect_dead_code, DeadCodeScope};
use crate::analysis::dependencies::{analyze_deps, Direction};
use crate::analysis::impact::analyze_impact;
use crate::db::Database;
use crate::model::file_graph::FileGraph;
use crate::model::{FileId, Language, RefKind, SymbolKind};

use super::graph_builder::{
    analysis_excluded_files, build_seed_all_file_ids, build_symbol_graph, maybe_filter_paths,
    maybe_filter_scope, maybe_filter_type_only,
};
use super::output::{
    display_path, format_cycles_text, format_dead_code_text, format_dead_symbols_text,
    format_deps_text, format_diff_text, format_dir_summary_text, format_exports_text,
    format_impact_text, format_json, format_lint_text, format_references_text, format_summary_text,
    format_symbols_text,
};
use super::OutputFormat;

// Re-export set_display_root so main.rs can call commands::set_display_root
pub use super::output::set_display_root;
// Re-export build_file_graph to maintain the same public API
pub use super::graph_builder::build_file_graph;

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
        let result = crate::cli::index::run_index(project_path, &config, false)?;
        eprintln!(
            "Indexed {} files ({} symbols) in {}ms",
            result.files_indexed + result.files_unchanged,
            result.symbols_extracted,
            result.duration_ms,
        );
    }

    Database::open(&db_path)
}

/// Resolve a user-provided file path to a FileId in the graph.
///
/// Tries exact path match first, then falls back to suffix matching.
fn resolve_file_id(graph: &FileGraph, project_path: &Path, file_path: &str) -> Result<FileId> {
    let abs_path = project_path.join(file_path);
    graph
        .file_by_path(&abs_path)
        .or_else(|| {
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(file_path))
                .map(|f| f.id)
        })
        .context(format!("File not found in index: {}", file_path))
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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let direction = match direction_str {
        "in" => Direction::ImportedBy,
        "out" => Direction::Imports,
        _ => Direction::Both,
    };

    let target_id = resolve_file_id(&graph, project_path, file_path)?;

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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

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
    source_set: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    if scope_str == "symbols" {
        // Symbol-level dead code analysis
        let file_graph = build_file_graph(&db, project_path)?;
        let file_graph = maybe_filter_paths(file_graph, path_glob, project_path)?;
        let file_graph = maybe_filter_scope(file_graph, source_set)?;
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

        // Filter out symbols in files from source sets with analysis=false
        let excluded = analysis_excluded_files(&file_graph);
        if !excluded.is_empty() {
            let excluded_paths: HashSet<String> = excluded
                .iter()
                .filter_map(|id| file_graph.files.get(id))
                .map(|info| info.path.display().to_string())
                .collect();
            result
                .dead_symbols
                .retain(|s| !excluded_paths.contains(&s.file));
            result.summary.dead_symbols = result.dead_symbols.len();
        }

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
    let graph = maybe_filter_scope(graph, source_set)?;

    let scope = match scope_str {
        "files" => DeadCodeScope::Files,
        "exports" => DeadCodeScope::Exports,
        _ => DeadCodeScope::Both,
    };

    let seed_all_file_ids = build_seed_all_file_ids(&graph, project_path);
    let mut result = detect_dead_code(&graph, scope, &seed_all_file_ids);

    // Filter out files in source sets with analysis=false
    let excluded = analysis_excluded_files(&graph);
    if !excluded.is_empty() {
        result.dead_files.retain(|f| !excluded.contains(&f.file_id));
        result
            .dead_exports
            .retain(|e| !excluded.contains(&e.file_id));
        result.summary.dead_files = result.dead_files.len();
        result.summary.dead_exports = result.dead_exports.len();
    }

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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;
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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let target_id = resolve_file_id(&graph, project_path, file_path)?;

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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let target_id = resolve_file_id(&graph, project_path, file_path)?;

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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

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
        #[serde(skip_serializing_if = "Option::is_none")]
        enrichment: Option<EnrichmentSummary>,
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

    #[derive(serde::Serialize)]
    struct EnrichmentSummary {
        scip_enriched_files: usize,
        scip_stale_files: usize,
        tree_sitter_only_files: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        scip_tool: Option<String>,
    }

    // Check for SCIP enrichment
    let scip_enriched_count = db.scip_enriched_file_count().unwrap_or(0);
    let enrichment = if scip_enriched_count > 0 {
        let scip_tool = db.get_metadata("scip_tool").unwrap_or(None);
        let scip_stale = db.scip_stale_file_count().unwrap_or(0);
        Some(EnrichmentSummary {
            scip_enriched_files: scip_enriched_count,
            scip_stale_files: scip_stale,
            tree_sitter_only_files: graph.file_count().saturating_sub(scip_enriched_count),
            scip_tool,
        })
    } else {
        None
    };

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
            external_imports: graph.external_import_count(),
            unresolved_imports: graph.truly_unresolved_count(),
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
        enrichment,
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
    scope: Option<&str>,
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

    // No rules configured at all
    if config.rules.is_empty() {
        let msg = "No lint rules configured";
        let output = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
                "message": msg,
                "violations": [],
                "summary": { "total": 0, "errors": 0, "warnings": 0 }
            }))?,
            _ => msg.to_string(),
        };
        return Ok((output, false));
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
    let graph = maybe_filter_scope(graph, scope)?;

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

    let result = compare_snapshots_with_graphs(db_before, db_after, &graph_before, &graph_after)?;

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

    run_diff_from_dbs(&db_before, &db_after, project_path, project_path, format)
}

/// Index a git ref, using cache if available.
fn index_git_ref(project_path: &Path, sha: &str) -> Result<Database> {
    use crate::git;

    let cache_path = git::snapshot_cache_path(project_path, sha);

    if cache_path.exists() {
        return Database::open(&cache_path).context(format!(
            "Failed to open cached snapshot: {}",
            cache_path.display()
        ));
    }

    // Export tree to temp dir and index it
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    git::export_tree_at_ref(project_path, sha, temp_dir.path())
        .context(format!("Failed to export tree at {}", sha))?;

    let config = crate::discovery::DiscoveryConfig::default();
    let index_result = crate::cli::index::run_index(temp_dir.path(), &config, false)?;

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

    Database::open(&cache_path).context(format!("Failed to open indexed snapshot for {}", sha))
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
    let link_result = crate::analysis::linker::link_cross_file_symbols(&file_graph);
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
    let link_result = crate::analysis::linker::link_cross_file_symbols(&file_graph);
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

/// Run the `churn` command.
#[allow(clippy::too_many_arguments)]
pub fn run_churn(
    project_path: &Path,
    glob_pattern: Option<&str>,
    co_change: bool,
    since: Option<&str>,
    until: Option<&str>,
    min_co_changes: usize,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    // Parse date filters
    let since_ts = since
        .map(crate::analysis::churn::parse_date_to_timestamp)
        .transpose()?;
    let until_ts = until
        .map(crate::analysis::churn::parse_date_to_timestamp)
        .transpose()?;

    if co_change {
        let graph = build_file_graph(&db, project_path)?;
        let graph = maybe_filter_type_only(graph, runtime_only);
        let graph = maybe_filter_paths(graph, path_glob, project_path)?;
        let graph = maybe_filter_scope(graph, scope)?;

        let mut result = crate::analysis::churn::compute_co_changes(
            &db,
            &graph,
            glob_pattern,
            min_co_changes,
            since_ts,
            until_ts,
        )?;

        // Apply display_path
        for entry in &mut result.pairs {
            entry.file_a = display_path(&project_path.join(&entry.file_a));
            entry.file_b = display_path(&project_path.join(&entry.file_b));
        }

        Ok(match format {
            OutputFormat::Text => format_co_change_text(&result),
            _ => format_json(&result, format),
        })
    } else {
        let mut result =
            crate::analysis::churn::compute_churn(&db, glob_pattern, since_ts, until_ts)?;

        // Apply display_path
        for entry in &mut result.files {
            entry.path = display_path(&project_path.join(&entry.path));
        }

        Ok(match format {
            OutputFormat::Text => format_churn_text(&result),
            _ => format_json(&result, format),
        })
    }
}

/// Format churn result as human-readable text.
fn format_churn_text(result: &crate::analysis::churn::ChurnResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("Churn Analysis ({} files):\n\n", result.count));
    out.push_str(&format!(
        "  {:<50} {:>8} {:>12} {:>10}\n",
        "File", "Commits", "Lines", "Freq/30d"
    ));
    out.push_str(&format!("  {}\n", "-".repeat(84)));

    for entry in &result.files {
        out.push_str(&format!(
            "  {:<50} {:>8} {:>12} {:>10.2}\n",
            entry.path, entry.commit_count, entry.lines_changed, entry.frequency
        ));
    }

    out
}

/// Format co-change result as human-readable text.
fn format_co_change_text(result: &crate::analysis::churn::CoChangeResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Co-Change Analysis ({} pairs, {} hidden coupling):\n\n",
        result.count, result.summary.hidden_coupling_count
    ));
    out.push_str(&format!(
        "  {:<40} {:<40} {:>5} {:>6} {:>5} {}\n",
        "File A", "File B", "Count", "Ratio", "Edge", ""
    ));
    out.push_str(&format!("  {}\n", "-".repeat(100)));

    for entry in &result.pairs {
        let flag = if entry.hidden_coupling {
            " HIDDEN COUPLING"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {:<40} {:<40} {:>5} {:>5.0}% {:>5}{}\n",
            entry.file_a,
            entry.file_b,
            entry.co_change_count,
            entry.co_change_ratio * 100.0,
            if entry.has_import_edge { "yes" } else { "no" },
            flag,
        ));
    }

    out
}

/// Run the `bus-factor` command.
#[allow(clippy::too_many_arguments)]
pub fn run_bus_factor(
    project_path: &Path,
    glob_pattern: Option<&str>,
    threshold: f64,
    half_life_days: f64,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
    half_life_mode: crate::analysis::ownership::HalfLifeMode,
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let mut result = crate::analysis::ownership::compute_bus_factor_analysis(
        &db,
        &graph,
        glob_pattern,
        threshold,
        half_life_days,
        project_path,
        half_life_mode,
    )?;

    // Apply display_path to file paths
    for entry in &mut result.files {
        entry.path = display_path(&project_path.join(&entry.path));
    }

    Ok(match format {
        OutputFormat::Text => format_bus_factor_text(&result),
        _ => format_json(&result, format),
    })
}

/// Format bus-factor result as human-readable text.
fn format_bus_factor_text(result: &crate::analysis::ownership::BusFactorResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Bus Factor Analysis ({} files):\n\n",
        result.count
    ));
    out.push_str(&format!(
        "  {:<6} {:<50} {:<5} {:<30} {:<6}\n",
        "Risk", "File", "BF", "Primary Owner", "Fan-in"
    ));
    out.push_str(&format!("  {}\n", "-".repeat(100)));

    for entry in &result.files {
        out.push_str(&format!(
            "  {:<6.1} {:<50} {:<5} {:<30} {:<6}\n",
            entry.risk_score,
            entry.path,
            entry.bus_factor,
            format!(
                "{} ({:.0}%)",
                entry.primary_owner.author_name, entry.primary_owner.score
            ),
            entry.fan_in,
        ));
    }

    out
}

/// Run the `bus-factor --by-author` command.
#[allow(clippy::too_many_arguments)]
pub fn run_bus_factor_by_author(
    project_path: &Path,
    glob_pattern: Option<&str>,
    half_life_days: f64,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
    half_life_mode: crate::analysis::ownership::HalfLifeMode,
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let result = crate::analysis::ownership::compute_bus_factor_by_author(
        &db,
        &graph,
        project_path,
        glob_pattern,
        half_life_days,
        half_life_mode,
    )?;

    Ok(match format {
        OutputFormat::Text => format_bus_factor_by_author_text(&result),
        _ => format_json(&result, format),
    })
}

/// Format per-author bus factor result as human-readable text.
fn format_bus_factor_by_author_text(
    result: &crate::analysis::ownership::AuthorBusFactorResult,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Bus Factor: Per-Person View ({} people)\n\n",
        result.count
    ));
    out.push_str(&format!(
        "  {:<12} {:<6} {:<14} {:<30} {}\n",
        "Sole-owned", "Total", "Blast radius", "Author", "Key areas"
    ));
    out.push_str(&format!("  {}\n", "-".repeat(100)));

    for entry in &result.authors {
        let areas = entry.key_areas.join(", ");
        out.push_str(&format!(
            "  {:<12} {:<6} {:<14} {:<30} {}\n",
            entry.sole_owned_files,
            entry.total_files,
            entry.total_blast_radius,
            entry.author_email,
            areas,
        ));
    }

    out
}

/// Run the `owners` command.
pub fn run_owners(
    project_path: &Path,
    glob_pattern: &str,
    top: usize,
    half_life_days: f64,
    format: &OutputFormat,
    no_index: bool,
    half_life_mode: crate::analysis::ownership::HalfLifeMode,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;

    let mut result = crate::analysis::ownership::compute_owners(
        &db,
        Some(glob_pattern),
        top,
        half_life_days,
        half_life_mode,
    )?;

    // Apply display_path to file paths for relative display
    for file in &mut result.files {
        file.path = display_path(&project_path.join(&file.path));
    }

    Ok(match format {
        OutputFormat::Text => format_owners_text(&result),
        _ => format_json(&result, format),
    })
}

/// Format owners result as human-readable text.
fn format_owners_text(result: &crate::analysis::ownership::OwnersResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("Owners ({} files):\n\n", result.count));

    for file in &result.files {
        out.push_str(&format!("  {}\n", file.path));
        for owner in &file.owners {
            out.push_str(&format!(
                "    {:<30} {:>5.1}%\n",
                owner.author_name, owner.score
            ));
        }
        out.push('\n');
    }

    out
}

/// Run the `who` command.
#[allow(clippy::too_many_arguments)]
pub fn run_who(
    project_path: &Path,
    file_path: &str,
    max_depth: Option<usize>,
    half_life_days: f64,
    format: &OutputFormat,
    no_index: bool,
    runtime_only: bool,
    path_glob: Option<&str>,
    half_life_mode: crate::analysis::ownership::HalfLifeMode,
    top_per_file: usize,
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let target_id = resolve_file_id(&graph, project_path, file_path)?;

    let mut result = crate::analysis::who::analyze_who(
        &db,
        &graph,
        target_id,
        max_depth,
        half_life_days,
        project_path,
        half_life_mode,
        top_per_file,
    )?;

    // Apply display_path to all paths in the result
    result.target_file = display_path(&project_path.join(&result.target_file));
    for file in &mut result.affected_files {
        file.path = display_path(&project_path.join(&file.path));
    }

    Ok(match format {
        OutputFormat::Text => format_who_text(&result),
        _ => format_json(&result, format),
    })
}

/// Format who result as human-readable text.
fn format_who_text(result: &crate::analysis::who::WhoResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Who to talk to about changes to {}:\n\n",
        result.target_file
    ));

    // Direct owners
    out.push_str("Direct owners:\n");
    if result.direct_owners.is_empty() {
        out.push_str("  (no ownership data)\n");
    } else {
        for owner in &result.direct_owners {
            out.push_str(&format!(
                "  {:<30} {:>5.1}%\n",
                format!("{} <{}>", owner.author_name, owner.author_email),
                owner.score
            ));
        }
    }
    out.push('\n');

    // Suggested reviewers
    if !result.suggested_reviewers.is_empty() {
        out.push_str("Suggested reviewers (minimal covering set):\n");
        for (i, reviewer) in result.suggested_reviewers.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {:<30} covers {} affected files\n",
                i + 1,
                format!("{} <{}>", reviewer.author_name, reviewer.author_email),
                reviewer.files_owned
            ));
        }
        out.push('\n');
    }

    // Downstream owners
    if !result.downstream_owners.is_empty() {
        out.push_str(&format!(
            "Downstream owners ({} affected files):\n",
            result.summary.total_affected_files
        ));
        for owner in &result.downstream_owners {
            out.push_str(&format!(
                "  {:<30} weight: {:>6.1}  files: {}\n",
                format!("{} <{}>", owner.author_name, owner.author_email),
                owner.total_weight,
                owner.files_owned
            ));
        }
        out.push('\n');
    }

    // Summary
    out.push_str(&format!(
        "Blast radius: {} files affected, {} unique downstream owners\n",
        result.summary.total_affected_files, result.summary.unique_downstream_owners
    ));

    out
}

/// Run the `team-coupling` command.
#[allow(clippy::too_many_arguments)]
pub fn run_team_coupling(
    project_path: &Path,
    glob_pattern: Option<&str>,
    half_life_days: f64,
    format: &OutputFormat,
    no_index: bool,
    half_life_mode: crate::analysis::ownership::HalfLifeMode,
    threshold: f64,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;

    let team_config = crate::linting::config::load_team_config(project_path);

    let params = crate::analysis::teams::TeamCouplingParams {
        glob_pattern,
        half_life_days,
        half_life_mode,
        team_config: &team_config,
        ownership_threshold: threshold,
    };
    let mut result =
        crate::analysis::teams::compute_team_coupling(&db, &graph, project_path, &params)?;

    // Apply display_path to file paths for relative display
    for file in &mut result.files {
        file.path = display_path(&project_path.join(&file.path));
    }
    for edge in &mut result.cross_team_edges {
        edge.from_file = display_path(&project_path.join(&edge.from_file));
        edge.to_file = display_path(&project_path.join(&edge.to_file));
    }

    Ok(match format {
        OutputFormat::Text => format_team_coupling_text(&result),
        _ => format_json(&result, format),
    })
}

/// Format team coupling result as human-readable text.
fn format_team_coupling_text(result: &crate::analysis::teams::TeamCouplingResult) -> String {
    let mut out = String::new();

    // Cross-team files
    let cross_team_files: Vec<_> = result.files.iter().filter(|f| f.is_cross_team).collect();
    if !cross_team_files.is_empty() {
        out.push_str(&format!(
            "Cross-team files ({} files requiring multi-team coordination):\n\n",
            cross_team_files.len()
        ));
        for file in &cross_team_files {
            let team_summary: Vec<String> = file
                .teams
                .iter()
                .filter(|t| t.ownership_pct >= 10.0)
                .map(|t| format!("{} ({:.0}%)", t.team, t.ownership_pct))
                .collect();
            out.push_str(&format!(
                "  {:<50} teams: {}\n",
                file.path,
                team_summary.join(", ")
            ));
        }
        out.push('\n');
    }

    // Cross-team dependency edges (limit to first 20)
    if !result.cross_team_edges.is_empty() {
        out.push_str(&format!(
            "Cross-team dependency edges ({}):\n\n",
            result.cross_team_edges.len()
        ));
        for (i, edge) in result.cross_team_edges.iter().enumerate() {
            if i >= 20 {
                out.push_str(&format!(
                    "  ... and {} more\n",
                    result.cross_team_edges.len() - 20
                ));
                break;
            }
            out.push_str(&format!(
                "  {} ({}) -> {} ({})\n",
                edge.from_file, edge.from_team, edge.to_file, edge.to_team
            ));
        }
        out.push('\n');
    }

    // Summary
    out.push_str(&format!(
        "Summary: {} files analyzed, {} teams found, {} cross-team files, {} cross-team edges\n",
        result.summary.files_analyzed,
        result.summary.teams_found,
        result.summary.cross_team_files,
        result.summary.cross_team_edges,
    ));

    if result.summary.cross_team_files == 0 && result.summary.cross_team_edges == 0 {
        out.push_str("No cross-team coordination issues detected.\n");
    }

    out
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
    scope: Option<&str>,
) -> Result<String> {
    let db = ensure_index(project_path, no_index)?;
    let graph = build_file_graph(&db, project_path)?;
    let graph = maybe_filter_type_only(graph, runtime_only);
    let graph = maybe_filter_paths(graph, path_glob, project_path)?;
    let graph = maybe_filter_scope(graph, scope)?;

    let (graph, focus_id) = if let Some(focus_path) = focus {
        let fid = resolve_file_id(&graph, project_path, focus_path)?;
        let subgraph = extract_subgraph(&graph, fid, depth);
        (subgraph, Some(fid))
    } else {
        (graph, None)
    };

    // When --format json/compact is explicitly requested, always output JSON
    match format {
        OutputFormat::Json | OutputFormat::Compact | OutputFormat::Csv => {
            let result = build_graph_json(&graph, focus_id);
            Ok(format_json(&result, format))
        }
        _ => match graph_format {
            "dot" => Ok(generate_dot(&graph, focus_id)),
            "svg" => generate_svg(&graph, focus_id),
            "html" => Ok(generate_html(&graph, focus_id)),
            "json" => {
                let result = build_graph_json(&graph, focus_id);
                Ok(format_json(&result, &OutputFormat::Json))
            }
            _ => Ok(generate_dot(&graph, focus_id)),
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
fn generate_dot(graph: &FileGraph, focus_id: Option<FileId>) -> String {
    let mut out = String::new();
    out.push_str("digraph dependencies {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box, style=filled, fillcolor=\"#e8e8e8\"];\n");
    out.push('\n');

    let mut files: Vec<_> = graph.all_files().collect();
    files.sort_by_key(|(id, _)| **id);

    for (_, info) in &files {
        let rel_path = display_path(&info.path);
        let color = if Some(info.id) == focus_id {
            "#8888ff"
        } else if info.is_entry_point {
            "#a8d8a8"
        } else {
            "#e8e8e8"
        };
        out.push_str(&format!("  \"{}\" [fillcolor=\"{}\"];\n", rel_path, color));
    }
    out.push('\n');

    let mut edges: Vec<(String, String)> = Vec::new();
    for (_, info) in &files {
        if let Some(import_edges) = graph.import_edges(info.id) {
            for edge in import_edges {
                if let Some(target) = graph.get_file(edge.to) {
                    let from = display_path(&info.path);
                    let to = display_path(&target.path);
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
fn generate_svg(graph: &FileGraph, focus_id: Option<FileId>) -> Result<String> {
    let dot = generate_dot(graph, focus_id);

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
fn build_graph_json(graph: &FileGraph, focus_id: Option<FileId>) -> serde_json::Value {
    let mut nodes = Vec::new();
    let mut edges_out = Vec::new();
    let mut files: Vec<_> = graph.all_files().collect();
    files.sort_by_key(|(id, _)| **id);

    for (_, info) in &files {
        let rel = display_path(&info.path);
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
                    let from = display_path(&info.path);
                    let to = display_path(&target.path);
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
fn generate_html(graph: &FileGraph, focus_id: Option<FileId>) -> String {
    let graph_json = build_graph_json(graph, focus_id);
    let json_str = serde_json::to_string(&graph_json).unwrap_or_default();
    HTML_TEMPLATE.replace(
        "/*GRAPH_DATA*/",
        &format!("const graphData = {};", json_str),
    )
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

/// Format any serializable analysis result as JSON.
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

/// Result of the enrich command.
pub struct EnrichResult {
    pub files_enriched: usize,
    pub files_skipped: usize,
    pub symbols_matched: usize,
    pub references_added: usize,
}

/// Resolve a SCIP document's relative path to a DB FileRecord.
fn resolve_scip_file(
    db: &crate::db::Database,
    project_path: &Path,
    rel_path: &str,
) -> Option<crate::model::FileRecord> {
    let abs_path = project_path.join(rel_path);
    let abs_path_str = abs_path.to_string_lossy().to_string();

    db.get_file_by_path(&abs_path_str)
        .unwrap_or(None)
        .or_else(|| {
            db.all_files()
                .unwrap_or_default()
                .into_iter()
                .find(|f| f.path.ends_with(rel_path))
        })
}

/// Match a SCIP definition to a tree-sitter symbol by name + line proximity.
fn match_scip_def_to_tree_sitter(
    scip_def: &crate::scip::ScipDefinition,
    ts_symbols: &[crate::model::Symbol],
) -> Option<crate::model::SymbolId> {
    let scip_line = scip_def.line as usize + 1; // convert 0-based to 1-based
    ts_symbols
        .iter()
        .filter(|s| s.name == scip_def.name)
        .min_by_key(|s| (s.line_span.start.line as isize - scip_line as isize).unsigned_abs())
        .filter(|s| {
            let dist = (s.line_span.start.line as isize - scip_line as isize).unsigned_abs();
            dist <= 5
        })
        .map(|s| s.id)
}

/// Run the `enrich` command: import SCIP index data into the existing DB.
///
/// Uses a two-pass approach:
/// - Pass 1: Build a global mapping from SCIP symbol strings to existing tree-sitter SymbolIds
/// - Pass 2: Insert SCIP references remapped to tree-sitter SymbolIds
///
/// This avoids creating duplicate symbol rows (the root cause of inflated dead-symbol counts).
pub fn run_enrich(project_path: &Path, scip_files: &[String]) -> Result<EnrichResult> {
    let db = ensure_index(project_path, false)?;

    let mut total_files_enriched = 0;
    let mut total_files_skipped = 0;
    let mut total_symbols_matched = 0;
    let mut total_refs = 0;

    for scip_file in scip_files {
        let scip_path = std::path::Path::new(scip_file);
        if !scip_path.exists() {
            anyhow::bail!("SCIP file not found: {}", scip_file);
        }

        let scip_index = crate::scip::read_scip_index(scip_path)
            .with_context(|| format!("failed to read SCIP file: {}", scip_file))?;

        db.begin_transaction()?;

        // Pass 1: Build global scip_symbol_string -> tree_sitter SymbolId mapping
        let mut scip_to_ts: std::collections::HashMap<String, crate::model::SymbolId> =
            std::collections::HashMap::new();
        let mut skipped_files: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for (doc_idx, doc) in scip_index.documents.iter().enumerate() {
            let rel_path = doc.relative_path.to_string_lossy();
            let file_record = match resolve_scip_file(&db, project_path, &rel_path) {
                Some(f) => f,
                None => {
                    skipped_files.insert(doc_idx);
                    total_files_skipped += 1;
                    continue;
                }
            };

            let ts_symbols = db.get_tree_sitter_symbols_by_file(file_record.id)?;

            for def in &doc.definitions {
                if let Some(ts_id) = match_scip_def_to_tree_sitter(def, &ts_symbols) {
                    scip_to_ts.insert(def.symbol.clone(), ts_id);
                    total_symbols_matched += 1;
                }
            }
        }

        // Pass 2: Insert remapped references
        let mut next_ref_id = db.next_reference_id()?;

        for (doc_idx, doc) in scip_index.documents.iter().enumerate() {
            if skipped_files.contains(&doc_idx) {
                continue;
            }

            let rel_path = doc.relative_path.to_string_lossy();
            let file_record = match resolve_scip_file(&db, project_path, &rel_path) {
                Some(f) => f,
                None => continue,
            };

            // Clear any previous SCIP data for this file
            db.clear_scip_data_for_file(file_record.id)?;

            for scip_ref in &doc.references {
                let target_id = match scip_to_ts.get(&scip_ref.symbol) {
                    Some(id) => *id,
                    None => continue,
                };

                let source_id =
                    find_enclosing_definition(&doc.definitions, &scip_to_ts, scip_ref.line);
                let source_id = match source_id {
                    Some(id) => id,
                    None => continue,
                };

                if source_id == target_id {
                    continue;
                }

                let ref_kind = match scip_ref.role {
                    crate::scip::ScipRole::Import => crate::model::RefKind::Import,
                    crate::scip::ScipRole::Reference => crate::model::RefKind::Call,
                    crate::scip::ScipRole::Definition => continue,
                };

                let reference = crate::model::Reference {
                    id: crate::model::ReferenceId(next_ref_id),
                    source: source_id,
                    target: target_id,
                    kind: ref_kind,
                    file: file_record.id,
                    span: crate::model::Span { start: 0, end: 0 },
                    line_span: crate::model::LineSpan {
                        start: crate::model::Position {
                            line: scip_ref.line as usize + 1,
                            column: scip_ref.column as usize,
                        },
                        end: crate::model::Position {
                            line: scip_ref.end_line as usize + 1,
                            column: scip_ref.end_column as usize,
                        },
                    },
                    target_name: None,
                };

                db.insert_scip_reference(&reference)?;
                next_ref_id += 1;
                total_refs += 1;
            }

            total_files_enriched += 1;
        }

        // Store enrichment timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        db.set_metadata("scip_enriched_at", &timestamp.to_string())?;

        if let Some(tool) = &scip_index.tool_name {
            db.set_metadata("scip_tool", tool)?;
        }

        db.commit_transaction()?;
    }

    Ok(EnrichResult {
        files_enriched: total_files_enriched,
        files_skipped: total_files_skipped,
        symbols_matched: total_symbols_matched,
        references_added: total_refs,
    })
}

/// Find the definition whose range encloses the given line.
fn find_enclosing_definition(
    definitions: &[crate::scip::ScipDefinition],
    scip_to_ts: &std::collections::HashMap<String, crate::model::SymbolId>,
    line: u32,
) -> Option<crate::model::SymbolId> {
    let mut best: Option<&crate::scip::ScipDefinition> = None;
    for def in definitions {
        if def.line <= line {
            match best {
                Some(prev) if def.line >= prev.line => best = Some(def),
                None => best = Some(def),
                _ => {}
            }
        }
    }
    best.and_then(|d| scip_to_ts.get(&d.symbol).copied())
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
                    is_scip_derived: false,
                },
                DepNode {
                    file_id: FileId(3),
                    path: PathBuf::from("src/lib.ts"),
                    depth: 2,
                    imported_names: vec![],
                    is_scip_derived: false,
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
                unresolved_ratio: 0.0,
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
                unresolved_ratio: 0.0,
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
                suppressed: 0,
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
                suppressed: 0,
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
                suppressed: 0,
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
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/a.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(3),
            path: PathBuf::from("src/b.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(4),
            path: PathBuf::from("src/c.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });

        // main imports a (normal edge)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        // a <-> b: real cycle
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["b".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 2,
        });
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        // main -> c via mod declaration (should NOT count as cycle edge)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(4),
            imported_names: vec!["c".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            is_scip_derived: false,
            line: 3,
        });
        // c -> main via mod declaration (would be false cycle without filtering)
        graph.add_import(FileImport {
            from: FileId(4),
            to: FileId(1),
            imported_names: vec!["main".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            is_scip_derived: false,
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
            cycles_result.summary.files_in_cycles, summary_cycles.summary.files_in_cycles,
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
            suppressions: std::collections::HashMap::new(),
            source_set: None,
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

        let external_count = graph.external_import_count();
        let unresolved_count = graph.truly_unresolved_count();

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

        // has_unresolved_imports should only return true for truly unresolved, not external
        assert!(
            graph.has_unresolved_imports(FileId(1)),
            "File with FileNotFound/DynamicPath imports should have unresolved imports"
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
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(crate::model::file_graph::FileInfo {
            id: FileId(2),
            path: PathBuf::from("/project/src/utils.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(crate::model::file_graph::FileInfo {
            id: FileId(3),
            path: PathBuf::from("/project/src/db.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["helper".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["query".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 2,
        });
        (graph, root)
    }

    #[test]
    fn test_generate_dot_basic() {
        let (graph, root) = make_test_graph();
        set_display_root(Some(root));
        let dot = generate_dot(&graph, None);

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
        set_display_root(Some(root));
        let dot = generate_dot(&graph, None);

        // main.ts is entry point -> green
        assert!(dot.contains("\"src/main.ts\" [fillcolor=\"#a8d8a8\"]"));
        // utils.ts is not entry point -> default gray
        assert!(dot.contains("\"src/utils.ts\" [fillcolor=\"#e8e8e8\"]"));
    }

    #[test]
    fn test_generate_dot_focus_coloring() {
        let (graph, root) = make_test_graph();
        set_display_root(Some(root));
        let dot = generate_dot(&graph, Some(FileId(2)));

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
                suppressions: std::collections::HashMap::new(),
                source_set: None,
            });
        }
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["b".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        graph.add_import(crate::model::file_graph::FileImport {
            from: FileId(3),
            to: FileId(4),
            imported_names: vec!["c".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
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
        set_display_root(Some(root));
        let json = build_graph_json(&graph, None);

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
        set_display_root(Some(root));
        let json = build_graph_json(&graph, Some(FileId(2)));

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
        set_display_root(Some(root));
        let html = generate_html(&graph, None);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("const graphData ="));
        assert!(html.contains("src/main.ts"));
        assert!(html.contains("src/utils.ts"));
        assert!(html.contains("requestAnimationFrame"));
    }

    #[test]
    fn test_run_lint_no_rules_returns_message() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("rules.toml");
        std::fs::write(
            &config_path,
            "[entry_points]\npatterns = [\"**/Main.java\"]\n",
        )
        .unwrap();

        // Text format
        let (output, has_errors) = run_lint(
            tmp.path(),
            Some(config_path.to_str().unwrap()),
            None,
            "info",
            &OutputFormat::Text,
            true, // no_index: skip indexing (early return before DB access)
            None,
            false,
            None,
        )
        .unwrap();
        assert_eq!(output, "No lint rules configured");
        assert!(!has_errors);

        // JSON format
        let (json_output, has_errors) = run_lint(
            tmp.path(),
            Some(config_path.to_str().unwrap()),
            None,
            "info",
            &OutputFormat::Json,
            true,
            None,
            false,
            None,
        )
        .unwrap();
        assert!(!has_errors);
        let parsed: serde_json::Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed["message"], "No lint rules configured");
        assert_eq!(parsed["violations"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["summary"]["total"], 0);
    }

    #[test]
    fn test_enrich_imports_scip_data() {
        use scip::types::{
            symbol_information::Kind, Document, Index, Occurrence, SymbolInformation, SymbolRole,
        };

        // Create a temp project with a Rust file
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("main.rs"),
            "fn main() {\n    greet();\n}\n\nfn greet() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();

        // Index the project first
        let config = crate::discovery::DiscoveryConfig::default();
        crate::cli::index::run_index(tmp.path(), &config, false).unwrap();

        // Build a SCIP index with definitions and references
        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/main.rs".to_string();
        doc.language = "Rust".to_string();

        // Definition: main function at line 0
        let mut main_occ = Occurrence::new();
        main_occ.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        main_occ.range = vec![0, 3, 7]; // line 0, col 3-7
        main_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(main_occ);

        // Definition: greet function at line 4
        let mut greet_occ = Occurrence::new();
        greet_occ.symbol = "rust-analyzer cargo test 0.1.0 greet().".to_string();
        greet_occ.range = vec![4, 3, 8]; // line 4, col 3-8
        greet_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(greet_occ);

        // Reference: main calls greet at line 1
        let mut call_occ = Occurrence::new();
        call_occ.symbol = "rust-analyzer cargo test 0.1.0 greet().".to_string();
        call_occ.range = vec![1, 4, 9]; // line 1, col 4-9
        call_occ.symbol_roles = 0; // plain reference
        doc.occurrences.push(call_occ);

        // Symbol info
        let mut main_info = SymbolInformation::new();
        main_info.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        main_info.kind = protobuf::EnumOrUnknown::new(Kind::Function);
        doc.symbols.push(main_info);

        let mut greet_info = SymbolInformation::new();
        greet_info.symbol = "rust-analyzer cargo test 0.1.0 greet().".to_string();
        greet_info.kind = protobuf::EnumOrUnknown::new(Kind::Function);
        doc.symbols.push(greet_info);

        index.documents.push(doc);

        // Write SCIP file
        let scip_path = tmp.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).unwrap();

        // Run enrich
        let result = run_enrich(tmp.path(), &[scip_path.to_string_lossy().to_string()]).unwrap();

        assert_eq!(result.files_enriched, 1);
        assert_eq!(result.files_skipped, 0);
        assert_eq!(result.symbols_matched, 2); // main + greet
        assert_eq!(result.references_added, 1); // main -> greet call

        // Verify DB has SCIP metadata
        let db_path = tmp.path().join(".statik").join("index.db");
        let db = crate::db::Database::open(&db_path).unwrap();
        let enriched_at = db.get_metadata("scip_enriched_at").unwrap();
        assert!(enriched_at.is_some());
    }

    #[test]
    fn test_enrich_skips_unknown_files() {
        use scip::types::{Document, Index, Occurrence, SymbolRole};

        // Create a temp project with a Rust file
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();

        let config = crate::discovery::DiscoveryConfig::default();
        crate::cli::index::run_index(tmp.path(), &config, false).unwrap();

        // Build SCIP index referring to a file NOT in the project
        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/nonexistent.rs".to_string();
        doc.language = "Rust".to_string();
        let mut occ = Occurrence::new();
        occ.symbol = "rust-analyzer cargo test 0.1.0 foo().".to_string();
        occ.range = vec![0, 0, 3];
        occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(occ);
        index.documents.push(doc);

        let scip_path = tmp.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).unwrap();

        let result = run_enrich(tmp.path(), &[scip_path.to_string_lossy().to_string()]).unwrap();

        assert_eq!(result.files_enriched, 0);
        assert_eq!(result.files_skipped, 1);
        assert_eq!(result.symbols_matched, 0);
    }

    #[test]
    fn test_enrich_idempotent() {
        use scip::types::{
            symbol_information::Kind, Document, Index, Occurrence, SymbolInformation, SymbolRole,
        };

        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();

        let config = crate::discovery::DiscoveryConfig::default();
        crate::cli::index::run_index(tmp.path(), &config, false).unwrap();

        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/main.rs".to_string();
        doc.language = "Rust".to_string();
        let mut occ = Occurrence::new();
        occ.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        occ.range = vec![0, 3, 7];
        occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(occ);
        let mut sym = SymbolInformation::new();
        sym.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        sym.kind = protobuf::EnumOrUnknown::new(Kind::Function);
        doc.symbols.push(sym);
        index.documents.push(doc);

        let scip_path = tmp.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).unwrap();

        // Enrich twice
        let scip_file = scip_path.to_string_lossy().to_string();
        let result1 = run_enrich(tmp.path(), &[scip_file.clone()]).unwrap();
        let result2 = run_enrich(tmp.path(), &[scip_file]).unwrap();

        // Second run should produce same results (clears previous SCIP data first)
        assert_eq!(result1.files_enriched, result2.files_enriched);
        assert_eq!(result1.symbols_matched, result2.symbols_matched);
    }

    #[test]
    fn test_enrich_does_not_increase_dead_symbols() {
        use scip::types::{
            symbol_information::Kind, Document, Index, Occurrence, SymbolInformation, SymbolRole,
        };

        // Create a project with two files where main calls helper::greet
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("main.rs"),
            "mod helper;\n\nfn main() {\n    helper::greet();\n}\n",
        )
        .unwrap();
        std::fs::write(
            src_dir.join("helper.rs"),
            "pub fn greet() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();

        // Index the project
        let config = crate::discovery::DiscoveryConfig::default();
        crate::cli::index::run_index(tmp.path(), &config, false).unwrap();

        // Count dead symbols before enrichment
        let before_json = run_dead_code(
            tmp.path(),
            "symbols",
            &OutputFormat::Json,
            true,
            false,
            None,
            None,
            None,
        )
        .unwrap();
        let before: serde_json::Value = serde_json::from_str(&before_json).unwrap();
        let dead_before = before["summary"]["dead_symbols"].as_u64().unwrap();

        // Build a SCIP index with cross-file references
        let mut index = Index::new();

        // Document for main.rs
        let mut main_doc = Document::new();
        main_doc.relative_path = "src/main.rs".to_string();
        main_doc.language = "Rust".to_string();

        // Definition: main function at line 2
        let mut main_occ = Occurrence::new();
        main_occ.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        main_occ.range = vec![2, 3, 7]; // line 2, col 3-7
        main_occ.symbol_roles = SymbolRole::Definition as i32;
        main_doc.occurrences.push(main_occ);

        // Reference: main calls helper::greet at line 3
        let mut call_occ = Occurrence::new();
        call_occ.symbol = "rust-analyzer cargo test 0.1.0 helper/greet().".to_string();
        call_occ.range = vec![3, 12, 17]; // line 3, col 12-17
        call_occ.symbol_roles = 0; // plain reference
        main_doc.occurrences.push(call_occ);

        let mut main_info = SymbolInformation::new();
        main_info.symbol = "rust-analyzer cargo test 0.1.0 main().".to_string();
        main_info.kind = protobuf::EnumOrUnknown::new(Kind::Function);
        main_doc.symbols.push(main_info);

        index.documents.push(main_doc);

        // Document for helper.rs
        let mut helper_doc = Document::new();
        helper_doc.relative_path = "src/helper.rs".to_string();
        helper_doc.language = "Rust".to_string();

        // Definition: greet function at line 0
        let mut greet_occ = Occurrence::new();
        greet_occ.symbol = "rust-analyzer cargo test 0.1.0 helper/greet().".to_string();
        greet_occ.range = vec![0, 7, 12]; // line 0, col 7-12
        greet_occ.symbol_roles = SymbolRole::Definition as i32;
        helper_doc.occurrences.push(greet_occ);

        let mut greet_info = SymbolInformation::new();
        greet_info.symbol = "rust-analyzer cargo test 0.1.0 helper/greet().".to_string();
        greet_info.kind = protobuf::EnumOrUnknown::new(Kind::Function);
        helper_doc.symbols.push(greet_info);

        index.documents.push(helper_doc);

        // Write SCIP file and enrich
        let scip_path = tmp.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).unwrap();

        let enrich_result =
            run_enrich(tmp.path(), &[scip_path.to_string_lossy().to_string()]).unwrap();
        assert!(
            enrich_result.symbols_matched >= 2,
            "Should match at least main and greet, got {}",
            enrich_result.symbols_matched
        );

        // Count dead symbols after enrichment
        let after_json = run_dead_code(
            tmp.path(),
            "symbols",
            &OutputFormat::Json,
            true,
            false,
            None,
            None,
            None,
        )
        .unwrap();
        let after: serde_json::Value = serde_json::from_str(&after_json).unwrap();
        let dead_after = after["summary"]["dead_symbols"].as_u64().unwrap();

        assert!(
            dead_after <= dead_before,
            "Enrichment should not increase dead symbols: before={}, after={}",
            dead_before,
            dead_after
        );
    }
}
