use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::OutputFormat;

thread_local! {
    static PROJECT_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Set the project root for path display. When set, `display_path` will
/// strip this prefix and show project-relative paths.
pub fn set_display_root(root: Option<PathBuf>) {
    PROJECT_ROOT.with(|r| *r.borrow_mut() = root);
}

/// Strip the project root prefix from a path for display.
/// When a project root is set via `set_display_root`, paths are shown
/// relative to it. Otherwise, absolute paths are shown.
pub fn display_path(path: &Path) -> String {
    PROJECT_ROOT.with(|r| {
        if let Some(root) = r.borrow().as_ref() {
            if let Ok(rel) = path.strip_prefix(root) {
                return rel.display().to_string();
            }
        }
        path.display().to_string()
    })
}

/// Serialize a value to JSON, applying path relativization.
pub fn format_json<T: serde::Serialize>(value: &T, format: &OutputFormat) -> String {
    let mut json_value = serde_json::to_value(value).unwrap_or_default();
    relativize_json_paths(&mut json_value);
    match format {
        OutputFormat::Json | OutputFormat::Csv => {
            serde_json::to_string_pretty(&json_value).unwrap_or_default()
        }
        OutputFormat::Compact => serde_json::to_string(&json_value).unwrap_or_default(),
        OutputFormat::Text => unreachable!("text format should be handled by caller"),
    }
}

/// Recursively strip the project root prefix from path-like strings in JSON values.
fn relativize_json_paths(value: &mut serde_json::Value) {
    PROJECT_ROOT.with(|r| {
        if let Some(root) = r.borrow().as_ref() {
            let prefix = format!("{}/", root.display());
            relativize_value(value, &prefix);
        }
    });
}

fn relativize_value(value: &mut serde_json::Value, prefix: &str) {
    match value {
        serde_json::Value::String(s) => {
            if s.starts_with(prefix) {
                *s = s[prefix.len()..].to_string();
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                relativize_value(item, prefix);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                relativize_value(v, prefix);
            }
        }
        _ => {}
    }
}

/// Format an indexing summary.
pub fn format_index_summary(
    files: usize,
    symbols: usize,
    references: usize,
    duration_ms: u128,
    format: &OutputFormat,
) -> String {
    match format {
        OutputFormat::Json | OutputFormat::Compact | OutputFormat::Csv => {
            let summary = serde_json::json!({
                "files_indexed": files,
                "symbols_extracted": symbols,
                "references_found": references,
                "duration_ms": duration_ms,
            });
            if matches!(format, OutputFormat::Compact) {
                serde_json::to_string(&summary).unwrap_or_default()
            } else {
                serde_json::to_string_pretty(&summary).unwrap_or_default()
            }
        }
        OutputFormat::Text => {
            format!(
                "Indexed {} files: {} symbols, {} references ({}ms)",
                files, symbols, references, duration_ms,
            )
        }
    }
}

// --- Text formatters for each command ---

pub fn format_dir_summary_text(result: &impl serde::Serialize) -> String {
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

pub fn format_symbols_text(result: &impl serde::Serialize) -> String {
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

pub fn format_references_text(result: &impl serde::Serialize) -> String {
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

pub fn format_deps_text(result: &crate::analysis::dependencies::DepsResult) -> String {
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
            let suffix = if dep.is_scip_derived {
                " [scip]".to_string()
            } else if dep.imported_names.is_empty() {
                String::new()
            } else {
                format!(" ({})", dep.imported_names.join(", "))
            };
            out.push_str(&format!(
                "{}{}{}\n",
                indent,
                display_path(&dep.path),
                suffix
            ));
        }
        out.push('\n');
    }

    if !result.imported_by.is_empty() {
        out.push_str(&format!("Imported by ({}):\n", result.imported_by.len()));
        for dep in &result.imported_by {
            let indent = "  ".repeat(dep.depth);
            let suffix = if dep.is_scip_derived {
                " [scip]".to_string()
            } else if dep.imported_names.is_empty() {
                String::new()
            } else {
                format!(" ({})", dep.imported_names.join(", "))
            };
            out.push_str(&format!(
                "{}{}{}\n",
                indent,
                display_path(&dep.path),
                suffix
            ));
        }
        out.push('\n');
    }

    if result.imports.is_empty() && result.imported_by.is_empty() {
        out.push_str("No dependencies found.\n");
    }

    out.push_str(&format!("Confidence: {}", result.confidence));
    out
}

pub fn format_dead_code_text(result: &crate::analysis::dead_code::DeadCodeResult) -> String {
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

pub fn format_dead_symbols_text(result: &crate::analysis::dead_code::DeadSymbolResult) -> String {
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

pub fn format_cycles_text(result: &crate::analysis::cycles::CycleResult) -> String {
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

pub fn format_impact_text(result: &crate::analysis::impact::ImpactResult) -> String {
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

pub fn format_exports_text(result: &serde_json::Value) -> String {
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

pub fn format_summary_text(result: &serde_json::Value) -> String {
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
            "Cycles: {} cycles, {} files involved\n",
            count, files_in,
        ));
    }

    if let Some(enrich) = result.get("enrichment") {
        let scip_files = enrich
            .get("scip_enriched_files")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let stale_files = enrich
            .get("scip_stale_files")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ts_files = enrich
            .get("tree_sitter_only_files")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tool = enrich
            .get("scip_tool")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if stale_files > 0 {
            out.push_str(&format!(
                "Enrichment: {} files via SCIP ({}), {} stale, {} tree-sitter only\n",
                scip_files, tool, stale_files, ts_files,
            ));
        } else {
            out.push_str(&format!(
                "Enrichment: {} files via SCIP ({}), {} tree-sitter only\n",
                scip_files, tool, ts_files,
            ));
        }
        out.push_str(&format!(
            "Resolution: {} files SCIP-precise, {} tree-sitter-heuristic",
            scip_files, ts_files,
        ));
    }

    out
}

pub fn format_lint_text(result: &crate::linting::rules::LintResult) -> String {
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

    let mut summary_parts = vec![format!(
        "{} errors, {} warnings across {} rules",
        result.summary.errors, result.summary.warnings, result.summary.rules_evaluated,
    )];
    if result.summary.suppressed > 0 {
        summary_parts.push(format!(
            "{} suppressed by inline comments",
            result.summary.suppressed,
        ));
    }
    out.push_str(&summary_parts.join(", "));
    out.push('\n');

    out
}

pub fn format_diff_text(result: &crate::analysis::diff::DiffResult) -> String {
    use crate::analysis::diff::{ChangeKind, CycleChangeKind, EdgeChangeKind};

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
                    out.push_str(&format!("      imported by: {}\n", display_path(importer),));
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
                out.push_str(&format!(
                    "  ! {} (length {})\n",
                    paths.join(" -> "),
                    c.length
                ));
            }
        }

        if !resolved.is_empty() {
            out.push_str(&format!("\nCycles resolved ({}):\n", resolved.len()));
            for c in &resolved {
                let paths: Vec<String> = c.files.iter().map(|p| display_path(p)).collect();
                out.push_str(&format!(
                    "  * {} (length {})\n",
                    paths.join(" -> "),
                    c.length
                ));
            }
        }

        out.push_str(&format!(
            "  {} introduced, {} resolved\n",
            result.summary.cycles_introduced, result.summary.cycles_resolved,
        ));
    }

    out
}
