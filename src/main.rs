use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use statik::cli::commands;
use statik::cli::index::run_index;
use statik::cli::output::format_index_summary;
use statik::cli::{Cli, Commands, OutputFormat};
use statik::discovery::DiscoveryConfig;
use statik::model::Language;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine project path (used by most commands)
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path_glob = cli.path_filter.as_deref();

    let is_csv = matches!(cli.format, OutputFormat::Csv);
    let has_jq = cli.jq.is_some();
    let needs_json_post =
        cli.count || cli.sort.is_some() || cli.limit.is_some() || is_csv || has_jq;

    // When --count, --sort, --limit, --jq, or --format csv is used, force JSON format for post-processing
    let format = if needs_json_post {
        &OutputFormat::Json
    } else {
        &cli.format
    };

    let command_name = command_name(&cli.command);
    let post_opts = PostProcessOpts {
        count: cli.count,
        limit: cli.limit,
        sort: cli.sort.as_deref(),
        reverse: cli.reverse,
        jq: cli.jq.as_deref(),
        original_format: &cli.format,
    };

    match cli.command {
        Commands::Index { ref path } => {
            let index_path = PathBuf::from(path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(path));

            let config = build_discovery_config(&cli);
            let result = run_index(&index_path, &config)?;

            let output = format_index_summary(
                result.files_indexed + result.files_unchanged,
                result.symbols_extracted,
                result.references_found,
                result.duration_ms,
                format,
            );
            emit_output(&output, command_name, &post_opts);

            if !result.parse_errors.is_empty() {
                eprintln!("\nParse errors:");
                for err in &result.parse_errors {
                    eprintln!("  {}", err);
                }
            }
        }

        Commands::Deps {
            ref file,
            transitive,
            ref direction,
            ref between,
        } => {
            if let Some(between_globs) = between {
                let output = commands::run_deps_between(
                    &project_path,
                    &between_globs[0],
                    &between_globs[1],
                    format,
                    cli.no_index,
                    cli.runtime_only,
                    path_glob,
                )?;
                emit_output(&output, command_name, &post_opts);
            } else {
                let file_path = file.as_deref().unwrap_or_else(|| {
                    eprintln!("Error: deps requires a file path or --between");
                    std::process::exit(2);
                });
                let output = commands::run_deps(
                    &project_path,
                    file_path,
                    transitive,
                    direction,
                    cli.max_depth,
                    format,
                    cli.no_index,
                    cli.runtime_only,
                    path_glob,
                )?;
                emit_output(&output, command_name, &post_opts);
            }
        }

        Commands::Exports { ref path } => {
            let output =
                commands::run_exports(&project_path, path, format, cli.no_index, path_glob)?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::DeadCode { ref scope } => {
            let output = commands::run_dead_code(
                &project_path,
                scope,
                format,
                cli.no_index,
                cli.runtime_only,
                path_glob,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Cycles => {
            let output = commands::run_cycles(
                &project_path,
                format,
                cli.no_index,
                cli.runtime_only,
                path_glob,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Impact { ref path } => {
            let output = commands::run_impact(
                &project_path,
                path,
                cli.max_depth,
                format,
                cli.no_index,
                cli.runtime_only,
                path_glob,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Summary { by_directory } => {
            let output = commands::run_summary(
                &project_path,
                format,
                cli.no_index,
                path_glob,
                by_directory,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Lint {
            ref config,
            ref rule,
            ref severity_threshold,
            freeze,
            update_baseline,
        } => {
            let (output, has_errors) = commands::run_lint(
                &project_path,
                config.as_deref(),
                rule.as_deref(),
                severity_threshold,
                format,
                cli.no_index,
                path_glob,
                freeze || update_baseline,
            )?;
            if needs_json_post {
                emit_output(&output, command_name, &post_opts);
            } else {
                println!("{}", output);
                if has_errors {
                    std::process::exit(1);
                }
            }
        }

        Commands::Diff { ref before } => {
            let output = commands::run_diff(&project_path, before, format, cli.no_index)?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Symbols { ref file, ref kind } => {
            let output = commands::run_symbols(
                &project_path,
                file.as_deref(),
                kind.as_deref(),
                format,
                cli.no_index,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::References {
            ref symbol,
            ref kind,
            ref file,
        } => {
            let output = commands::run_references(
                &project_path,
                symbol,
                kind.as_deref(),
                file.as_deref(),
                format,
                cli.no_index,
            )?;
            emit_output(&output, command_name, &post_opts);
        }

        Commands::Callers {
            ref symbol,
            ref file,
        } => {
            let output = commands::run_callers(
                &project_path,
                symbol,
                file.as_deref(),
                format,
                cli.no_index,
            )?;
            emit_output(&output, command_name, &post_opts);
        }
    }

    Ok(())
}

/// Get the command name for count extraction.
fn command_name(cmd: &Commands) -> &'static str {
    match cmd {
        Commands::Index { .. } => "index",
        Commands::Deps { .. } => "deps",
        Commands::Exports { .. } => "exports",
        Commands::DeadCode { .. } => "dead-code",
        Commands::Cycles => "cycles",
        Commands::Impact { .. } => "impact",
        Commands::Summary { .. } => "summary",
        Commands::Lint { .. } => "lint",
        Commands::Diff { .. } => "diff",
        Commands::Symbols { .. } => "symbols",
        Commands::References { .. } => "references",
        Commands::Callers { .. } => "callers",
    }
}

/// Returns true if the command detects problems (nonzero count should exit 1).
fn is_problem_command(command: &str) -> bool {
    matches!(command, "dead-code" | "cycles" | "lint")
}

/// Extract count from JSON output based on command type.
fn extract_count(json: &serde_json::Value, command: &str) -> u64 {
    let num = |v: &serde_json::Value| v.as_u64();
    let arr_len = |v: &serde_json::Value| v.as_array().map(|a| a.len() as u64);
    let summary = json.get("summary");

    match command {
        "dead-code" => {
            let dead_files = summary
                .and_then(|s| s.get("dead_files").and_then(num))
                .or_else(|| json.get("dead_files").and_then(arr_len))
                .unwrap_or(0);
            let dead_exports = summary
                .and_then(|s| s.get("dead_exports").and_then(num))
                .or_else(|| json.get("dead_exports").and_then(arr_len))
                .unwrap_or(0);
            let dead_symbols = summary
                .and_then(|s| s.get("dead_symbols").and_then(num))
                .or_else(|| json.get("dead_symbols").and_then(arr_len))
                .unwrap_or(0);
            dead_files + dead_exports + dead_symbols
        }
        "cycles" => summary
            .and_then(|s| s.get("cycle_count").and_then(num))
            .or_else(|| json.get("cycles").and_then(arr_len))
            .unwrap_or(0),
        "lint" => summary
            .and_then(|s| s.get("total_violations").and_then(num))
            .unwrap_or(0),
        "deps" => {
            // --between mode has "count" and "edges"
            if let Some(count) = json.get("count").and_then(num) {
                return count;
            }
            let imports = json.get("imports").and_then(arr_len).unwrap_or(0);
            let imported_by = json.get("imported_by").and_then(arr_len).unwrap_or(0);
            imports + imported_by
        }
        "impact" => summary
            .and_then(|s| s.get("total_affected").and_then(num))
            .unwrap_or(0),
        "exports" => summary
            .and_then(|s| s.get("total").and_then(num))
            .unwrap_or(0),
        "symbols" | "references" | "callers" => {
            json.get("count").and_then(num).unwrap_or(0)
        }
        "diff" => {
            let breaking = summary
                .and_then(|s| s.get("breaking_changes").and_then(num))
                .unwrap_or(0);
            let expanding = summary
                .and_then(|s| s.get("expanding_changes").and_then(num))
                .unwrap_or(0);
            breaking + expanding
        }
        "index" => json.get("files_indexed").and_then(num).unwrap_or(0),
        "summary" => json
            .get("files")
            .and_then(|f| f.get("total").and_then(num))
            .unwrap_or(0),
        _ => 0,
    }
}

struct PostProcessOpts<'a> {
    count: bool,
    limit: Option<usize>,
    sort: Option<&'a str>,
    reverse: bool,
    jq: Option<&'a str>,
    original_format: &'a OutputFormat,
}

/// Print output, applying --count, --sort, --limit, --reverse, --jq if requested.
fn emit_output(output: &str, command: &str, opts: &PostProcessOpts) {
    let is_csv = matches!(opts.original_format, OutputFormat::Csv);
    let has_jq = opts.jq.is_some();
    let is_json_output = matches!(
        opts.original_format,
        OutputFormat::Json | OutputFormat::Compact | OutputFormat::Csv
    );
    let needs_json = opts.count || opts.sort.is_some() || opts.limit.is_some() || is_csv
        || is_json_output || has_jq;

    if !needs_json {
        println!("{}", output);
        return;
    }

    let mut json: serde_json::Value = match serde_json::from_str(output) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Error: could not parse output as JSON for post-processing");
            std::process::exit(2);
        }
    };

    // Enrich path fields with directory/filename/extension for JSON output
    if is_json_output || has_jq {
        enrich_path_fields(&mut json);
    }

    // Apply sort and limit to primary arrays in the JSON
    if opts.sort.is_some() || opts.limit.is_some() {
        apply_sort_limit(&mut json, command, opts.sort, opts.limit, opts.reverse);
    }

    if opts.count {
        let count = extract_count(&json, command);
        println!("{}", count);
        // Only exit 1 for problem-detection commands where nonzero means issues found.
        // For informational commands (exports, symbols, deps, etc.), nonzero is normal.
        if count > 0 && is_problem_command(command) {
            std::process::exit(1);
        }
        return;
    }

    // Apply --jq filter if specified
    if let Some(jq_expr) = opts.jq {
        match apply_jq_filter(&json, jq_expr) {
            Ok(results) => {
                for result in &results {
                    println!("{}", serde_json::to_string_pretty(result).unwrap_or_default());
                }
            }
            Err(e) => {
                eprintln!("Error: jq filter failed: {}", e);
                std::process::exit(2);
            }
        }
        return;
    }

    // Re-format output based on original requested format
    match opts.original_format {
        OutputFormat::Csv => {
            println!("{}", json_to_csv(&json, command));
        }
        OutputFormat::Text => {
            // For text mode with sort/limit, render arrays as readable text
            println!("{}", json_to_text(&json, command));
        }
        OutputFormat::Compact => {
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
        }
    }
}

/// Known primary array field names per command.
fn primary_arrays(command: &str) -> Vec<&'static str> {
    match command {
        "dead-code" => vec!["dead_files", "dead_exports", "dead_symbols"],
        "cycles" => vec!["cycles"],
        "lint" => vec!["violations"],
        "deps" => vec!["imports", "imported_by", "edges"],
        "impact" => vec!["affected"],
        "exports" => vec!["exports"],
        "symbols" => vec!["symbols"],
        "references" => vec!["references"],
        "callers" => vec!["callers"],
        "diff" => vec!["changes"],
        "summary" => vec!["directories"],
        _ => vec![],
    }
}

/// Sort and truncate primary arrays in JSON output.
fn apply_sort_limit(
    json: &mut serde_json::Value,
    command: &str,
    sort_field: Option<&str>,
    limit: Option<usize>,
    reverse: bool,
) {
    let arrays = primary_arrays(command);

    for array_name in arrays {
        if let Some(arr) = json.get_mut(array_name).and_then(|v| v.as_array_mut()) {
            // Sort
            if let Some(field) = sort_field {
                arr.sort_by(|a, b| {
                    let va = a.get(field);
                    let vb = b.get(field);
                    compare_json_values(va, vb)
                });
                if reverse {
                    arr.reverse();
                }
            } else if reverse {
                arr.reverse();
            }

            // Limit
            if let Some(n) = limit {
                arr.truncate(n);
            }
        }
    }
}

fn compare_json_values(
    a: Option<&serde_json::Value>,
    b: Option<&serde_json::Value>,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            // Compare numbers
            if let (Some(na), Some(nb)) = (a.as_f64(), b.as_f64()) {
                return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
            }
            // Compare strings
            if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
                return sa.cmp(sb);
            }
            // Compare booleans
            if let (Some(ba), Some(bb)) = (a.as_bool(), b.as_bool()) {
                return ba.cmp(&bb);
            }
            std::cmp::Ordering::Equal
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Convert JSON output to a readable text format. Extracts primary arrays
/// and renders each entry as a compact text line, similar to the native text
/// formatters but working from the post-processed JSON.
fn json_to_text(json: &serde_json::Value, command: &str) -> String {
    let arrays = primary_arrays(command);
    let mut out = String::new();

    // Try to print a summary line if present
    if let Some(summary) = json.get("summary") {
        if let Some(obj) = summary.as_object() {
            let parts: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect();
            out.push_str(&parts.join(", "));
            out.push('\n');
        }
    }

    for array_name in &arrays {
        if let Some(arr) = json.get(*array_name).and_then(|v| v.as_array()) {
            if arr.is_empty() {
                continue;
            }
            // Section header
            out.push_str(&format!("\n{}:\n", array_name));
            for item in arr {
                if let Some(obj) = item.as_object() {
                    // Build a compact one-line representation
                    let line = format_text_entry(obj, command, array_name);
                    out.push_str(&format!("  {}\n", line));
                } else {
                    // Non-object entries (e.g., cycle arrays)
                    out.push_str(&format!("  {}\n", item));
                }
            }
        }
    }

    // If nothing was printed from arrays, fall back to pretty JSON
    if out.trim().is_empty() {
        return serde_json::to_string_pretty(json).unwrap_or_default();
    }

    out
}

/// Format a single JSON object entry as a compact text line.
fn format_text_entry(
    obj: &serde_json::Map<String, serde_json::Value>,
    _command: &str,
    _array_name: &str,
) -> String {
    // Use "path" as the primary field if present, then add key details
    let mut parts = Vec::new();

    if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
        parts.push(path.to_string());
    }

    // Add other interesting fields
    for key in &[
        "export_name",
        "name",
        "rule_id",
        "source_file",
        "target_file",
        "kind",
        "confidence",
        "severity",
        "description",
        "depth",
        "line",
        "fan_in",
        "fan_out",
        "instability",
        "external_ratio",
        "files",
        "exports",
        "dead_exports",
    ] {
        if let Some(val) = obj.get(*key) {
            if val.is_null() {
                continue;
            }
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => {
                    let items: Vec<String> = a
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    items.join(", ")
                }
                _ => format!("{}", val),
            };
            // Skip path since we already printed it
            if *key == "path" {
                continue;
            }
            parts.push(format!("{}: {}", key, s));
        }
    }

    parts.join("  ")
}

/// Convert JSON output to CSV. Finds the primary array for the command and
/// flattens each object into a CSV row with a header.
fn json_to_csv(json: &serde_json::Value, command: &str) -> String {
    use std::collections::BTreeSet;

    let arrays = primary_arrays(command);

    // Collect all rows from all primary arrays
    let mut rows: Vec<&serde_json::Value> = Vec::new();
    for array_name in &arrays {
        if let Some(arr) = json.get(*array_name).and_then(|v| v.as_array()) {
            rows.extend(arr.iter());
        }
    }

    if rows.is_empty() {
        return String::new();
    }

    // Collect all unique keys (sorted for deterministic output)
    let mut keys = BTreeSet::new();
    for row in &rows {
        if let Some(obj) = row.as_object() {
            for key in obj.keys() {
                keys.insert(key.clone());
            }
        }
    }

    let key_list: Vec<String> = keys.into_iter().collect();
    let mut out = String::new();

    // Header
    out.push_str(&key_list.join(","));
    out.push('\n');

    // Rows
    for row in &rows {
        let fields: Vec<String> = key_list
            .iter()
            .map(|k| {
                let val = row.get(k);
                csv_escape_value(val)
            })
            .collect();
        out.push_str(&fields.join(","));
        out.push('\n');
    }

    // Remove trailing newline
    if out.ends_with('\n') {
        out.pop();
    }

    out
}

/// Escape a JSON value for CSV output.
fn csv_escape_value(val: Option<&serde_json::Value>) -> String {
    match val {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::String(s)) => {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        Some(serde_json::Value::Array(arr)) => {
            let joined: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            let s = joined.join(";");
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s
            }
        }
        Some(serde_json::Value::Object(_)) => {
            let s = val.unwrap().to_string();
            format!("\"{}\"", s.replace('"', "\"\""))
        }
    }
}

/// Apply a jq filter expression to a JSON value, returning all output values.
fn apply_jq_filter(
    json: &serde_json::Value,
    expr: &str,
) -> std::result::Result<Vec<serde_json::Value>, String> {
    use jaq_core::{Ctx, Definitions, Val};

    // Start with core filters (length, keys, sort, has, contains, etc.)
    let mut defs = Definitions::core();

    // Load the full jaq standard library (map, select, to_entries, unique_by, etc.)
    let mut errs = Vec::new();
    for def in jaq_std::std() {
        defs.insert(def, &mut errs);
    }
    if !errs.is_empty() {
        return Err(format!("failed to load jq standard library: {:?}", errs));
    }

    // Parse the jq expression
    let (parsed, parse_errs) = jaq_core::parse::parse(expr, jaq_core::parse::main());
    if !parse_errs.is_empty() {
        return Err(format!(
            "parse error in jq expression: {:?}",
            parse_errs
                .iter()
                .map(|e| format!("{}", e))
                .collect::<Vec<_>>()
        ));
    }
    let parsed = match parsed {
        Some(f) => f,
        None => return Err("empty jq expression".to_string()),
    };

    // Compile the filter
    let mut compile_errs = Vec::new();
    let filter = defs.finish(parsed, Vec::new(), &mut compile_errs);
    if !compile_errs.is_empty() {
        return Err(format!(
            "compile error in jq expression ({} errors)",
            compile_errs.len()
        ));
    }

    // Run the filter
    let out = filter.run(Ctx::new(), Val::from(json.clone()));

    let mut results = Vec::new();
    for item in out {
        match item {
            Ok(val) => {
                let v: serde_json::Value = val.into();
                results.push(v);
            }
            Err(e) => {
                return Err(format!("runtime error: {}", e));
            }
        }
    }

    Ok(results)
}

/// Enrich JSON objects by adding directory/filename/extension/language alongside path fields.
/// Walks the entire JSON tree and for any object with a path-like string field,
/// adds derived fields if not already present.
fn enrich_path_fields(json: &mut serde_json::Value) {
    match json {
        serde_json::Value::Object(map) => {
            let path_fields: Vec<String> = [
                "path",
                "source_file",
                "target_file",
                "file",
                "from",
                "to",
                "target_path",
            ]
            .iter()
            .filter(|k| map.contains_key(**k))
            .map(|k| k.to_string())
            .collect();

            for key in &path_fields {
                if let Some(path_str) =
                    map.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
                {
                    let p = std::path::Path::new(&path_str);
                    let dir_key = format!("{}_directory", key);
                    let filename_key = format!("{}_filename", key);
                    let ext_key = format!("{}_extension", key);
                    let lang_key = format!("{}_language", key);

                    if !map.contains_key(&dir_key) {
                        let dir = p
                            .parent()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        map.insert(dir_key, serde_json::Value::String(dir));
                    }
                    if !map.contains_key(&filename_key) {
                        let filename = p
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        map.insert(filename_key, serde_json::Value::String(filename));
                    }
                    if !map.contains_key(&ext_key) {
                        let ext = p
                            .extension()
                            .map(|e| e.to_string_lossy().to_string())
                            .unwrap_or_default();
                        map.insert(ext_key.clone(), serde_json::Value::String(ext));
                    }
                    if !map.contains_key(&lang_key) {
                        let ext = map
                            .get(&ext_key)
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let lang = extension_to_language(ext);
                        if !lang.is_empty() {
                            map.insert(lang_key, serde_json::Value::String(lang.to_string()));
                        }
                    }
                }
            }

            // Recurse into values
            for val in map.values_mut() {
                enrich_path_fields(val);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                enrich_path_fields(item);
            }
        }
        _ => {}
    }
}

/// Map file extension to language name for enrichment.
fn extension_to_language(ext: &str) -> &'static str {
    match ext {
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "java" => "Java",
        "rs" => "Rust",
        _ => "",
    }
}

fn build_discovery_config(cli: &Cli) -> DiscoveryConfig {
    let languages = cli.lang.as_ref().and_then(|l| {
        Language::from_extension(match l.to_lowercase().as_str() {
            "typescript" | "ts" => "ts",
            "javascript" | "js" => "js",
            "python" | "py" => "py",
            "rust" | "rs" => "rs",
            "java" => "java",
            _ => return None,
        })
    });

    DiscoveryConfig {
        include: cli.include.clone(),
        exclude: cli.exclude.clone(),
        languages: languages.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // compare_json_values tests
    // =========================================================================

    #[test]
    fn test_compare_numbers() {
        let a = json!(1);
        let b = json!(2);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_json_values(Some(&b), Some(&a)),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_json_values(Some(&a), Some(&a)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_compare_strings() {
        let a = json!("alpha");
        let b = json!("beta");
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_json_values(Some(&b), Some(&a)),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_compare_booleans() {
        let f = json!(false);
        let t = json!(true);
        assert_eq!(
            compare_json_values(Some(&f), Some(&t)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_none_values() {
        let a = json!(1);
        assert_eq!(
            compare_json_values(None, None),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_json_values(Some(&a), None),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_json_values(None, Some(&a)),
            std::cmp::Ordering::Greater
        );
    }

    // =========================================================================
    // primary_arrays tests
    // =========================================================================

    #[test]
    fn test_primary_arrays_known_commands() {
        assert_eq!(
            primary_arrays("dead-code"),
            vec!["dead_files", "dead_exports", "dead_symbols"]
        );
        assert_eq!(primary_arrays("cycles"), vec!["cycles"]);
        assert_eq!(primary_arrays("lint"), vec!["violations"]);
        assert_eq!(primary_arrays("deps"), vec!["imports", "imported_by", "edges"]);
        assert_eq!(primary_arrays("impact"), vec!["affected"]);
        assert_eq!(primary_arrays("exports"), vec!["exports"]);
        assert_eq!(primary_arrays("symbols"), vec!["symbols"]);
        assert_eq!(primary_arrays("references"), vec!["references"]);
        assert_eq!(primary_arrays("callers"), vec!["callers"]);
        assert_eq!(primary_arrays("diff"), vec!["changes"]);
    }

    #[test]
    fn test_primary_arrays_unknown_command() {
        let result: Vec<&str> = primary_arrays("unknown");
        assert!(result.is_empty());
    }

    // =========================================================================
    // extract_count tests
    // =========================================================================

    #[test]
    fn test_extract_count_dead_code() {
        let json = json!({
            "dead_files": [{"path": "a.ts"}, {"path": "b.ts"}],
            "dead_exports": [{"name": "foo"}],
            "summary": {"dead_files": 2, "dead_exports": 1}
        });
        assert_eq!(extract_count(&json, "dead-code"), 3);
    }

    #[test]
    fn test_extract_count_cycles() {
        let json = json!({"summary": {"cycle_count": 5}});
        assert_eq!(extract_count(&json, "cycles"), 5);
    }

    #[test]
    fn test_extract_count_lint() {
        let json = json!({"summary": {"total_violations": 7}});
        assert_eq!(extract_count(&json, "lint"), 7);
    }

    #[test]
    fn test_extract_count_deps() {
        let json = json!({
            "imports": [{"path": "a.ts"}, {"path": "b.ts"}],
            "imported_by": [{"path": "c.ts"}]
        });
        assert_eq!(extract_count(&json, "deps"), 3);
    }

    #[test]
    fn test_extract_count_symbols() {
        let json = json!({"count": 42});
        assert_eq!(extract_count(&json, "symbols"), 42);
    }

    #[test]
    fn test_extract_count_references() {
        let json = json!({"count": 10});
        assert_eq!(extract_count(&json, "references"), 10);
    }

    // =========================================================================
    // apply_sort_limit tests
    // =========================================================================

    #[test]
    fn test_sort_by_string_field() {
        let mut json = json!({
            "dead_files": [
                {"path": "z.ts", "confidence": "high"},
                {"path": "a.ts", "confidence": "low"},
                {"path": "m.ts", "confidence": "medium"}
            ]
        });
        apply_sort_limit(&mut json, "dead-code", Some("path"), None, false);
        let arr = json["dead_files"].as_array().unwrap();
        assert_eq!(arr[0]["path"], "a.ts");
        assert_eq!(arr[1]["path"], "m.ts");
        assert_eq!(arr[2]["path"], "z.ts");
    }

    #[test]
    fn test_sort_by_numeric_field() {
        let mut json = json!({
            "affected": [
                {"path": "a.ts", "depth": 3},
                {"path": "b.ts", "depth": 1},
                {"path": "c.ts", "depth": 2}
            ]
        });
        apply_sort_limit(&mut json, "impact", Some("depth"), None, false);
        let arr = json["affected"].as_array().unwrap();
        assert_eq!(arr[0]["depth"], 1);
        assert_eq!(arr[1]["depth"], 2);
        assert_eq!(arr[2]["depth"], 3);
    }

    #[test]
    fn test_sort_reverse() {
        let mut json = json!({
            "symbols": [
                {"name": "alpha"},
                {"name": "beta"},
                {"name": "gamma"}
            ]
        });
        apply_sort_limit(&mut json, "symbols", Some("name"), None, true);
        let arr = json["symbols"].as_array().unwrap();
        assert_eq!(arr[0]["name"], "gamma");
        assert_eq!(arr[1]["name"], "beta");
        assert_eq!(arr[2]["name"], "alpha");
    }

    #[test]
    fn test_limit_truncates() {
        let mut json = json!({
            "violations": [
                {"rule_id": "r1"},
                {"rule_id": "r2"},
                {"rule_id": "r3"},
                {"rule_id": "r4"},
                {"rule_id": "r5"}
            ]
        });
        apply_sort_limit(&mut json, "lint", None, Some(3), false);
        let arr = json["violations"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["rule_id"], "r1");
        assert_eq!(arr[2]["rule_id"], "r3");
    }

    #[test]
    fn test_sort_then_limit() {
        let mut json = json!({
            "symbols": [
                {"name": "gamma", "line": 30},
                {"name": "alpha", "line": 10},
                {"name": "beta", "line": 20},
                {"name": "delta", "line": 40}
            ]
        });
        apply_sort_limit(&mut json, "symbols", Some("name"), Some(2), false);
        let arr = json["symbols"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "alpha");
        assert_eq!(arr[1]["name"], "beta");
    }

    #[test]
    fn test_reverse_without_sort() {
        let mut json = json!({
            "callers": [
                {"caller": "a"},
                {"caller": "b"},
                {"caller": "c"}
            ]
        });
        apply_sort_limit(&mut json, "callers", None, None, true);
        let arr = json["callers"].as_array().unwrap();
        assert_eq!(arr[0]["caller"], "c");
        assert_eq!(arr[1]["caller"], "b");
        assert_eq!(arr[2]["caller"], "a");
    }

    #[test]
    fn test_limit_larger_than_array() {
        let mut json = json!({
            "references": [
                {"source": "a"},
                {"source": "b"}
            ]
        });
        apply_sort_limit(&mut json, "references", None, Some(100), false);
        let arr = json["references"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_sort_multiple_arrays_in_command() {
        let mut json = json!({
            "imports": [
                {"path": "z.ts", "depth": 1},
                {"path": "a.ts", "depth": 1}
            ],
            "imported_by": [
                {"path": "y.ts", "depth": 1},
                {"path": "b.ts", "depth": 1}
            ]
        });
        apply_sort_limit(&mut json, "deps", Some("path"), None, false);
        let imports = json["imports"].as_array().unwrap();
        assert_eq!(imports[0]["path"], "a.ts");
        assert_eq!(imports[1]["path"], "z.ts");
        let imported_by = json["imported_by"].as_array().unwrap();
        assert_eq!(imported_by[0]["path"], "b.ts");
        assert_eq!(imported_by[1]["path"], "y.ts");
    }

    #[test]
    fn test_sort_with_missing_field() {
        let mut json = json!({
            "symbols": [
                {"name": "beta"},
                {"name": "alpha"},
                {}
            ]
        });
        apply_sort_limit(&mut json, "symbols", Some("name"), None, false);
        let arr = json["symbols"].as_array().unwrap();
        // Items with the field come first (Less), items without come last (Greater)
        assert_eq!(arr[0]["name"], "alpha");
        assert_eq!(arr[1]["name"], "beta");
    }

    #[test]
    fn test_no_op_on_empty_array() {
        let mut json = json!({"cycles": []});
        apply_sort_limit(&mut json, "cycles", Some("length"), Some(5), true);
        let arr = json["cycles"].as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_no_op_on_unknown_command() {
        let mut json = json!({"data": [{"x": 1}]});
        let original = json.clone();
        apply_sort_limit(&mut json, "unknown", Some("x"), Some(1), false);
        assert_eq!(json, original);
    }

    // =========================================================================
    // json_to_csv tests
    // =========================================================================

    #[test]
    fn test_csv_basic() {
        let json = json!({
            "dead_files": [
                {"path": "a.ts", "confidence": "high"},
                {"path": "b.ts", "confidence": "low"}
            ]
        });
        let csv = json_to_csv(&json, "dead-code");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "confidence,path");
        assert_eq!(lines[1], "high,a.ts");
        assert_eq!(lines[2], "low,b.ts");
    }

    #[test]
    fn test_csv_empty_array() {
        let json = json!({"cycles": []});
        let csv = json_to_csv(&json, "cycles");
        assert!(csv.is_empty());
    }

    #[test]
    fn test_csv_escape_commas() {
        let json = json!({
            "violations": [
                {"rule_id": "r1", "description": "has, comma"}
            ]
        });
        let csv = json_to_csv(&json, "lint");
        assert!(csv.contains("\"has, comma\""));
    }

    #[test]
    fn test_csv_escape_quotes() {
        let json = json!({
            "symbols": [
                {"name": "say\"hello\"", "kind": "function"}
            ]
        });
        let csv = json_to_csv(&json, "symbols");
        assert!(csv.contains("\"say\"\"hello\"\"\""));
    }

    #[test]
    fn test_csv_arrays_joined_with_semicolons() {
        let json = json!({
            "violations": [
                {"rule_id": "r1", "imported_names": ["foo", "bar"]}
            ]
        });
        let csv = json_to_csv(&json, "lint");
        assert!(csv.contains("foo;bar"));
    }

    #[test]
    fn test_csv_multiple_arrays() {
        let json = json!({
            "dead_files": [{"path": "a.ts"}],
            "dead_exports": [{"path": "b.ts", "export_name": "foo"}]
        });
        let csv = json_to_csv(&json, "dead-code");
        let lines: Vec<&str> = csv.lines().collect();
        // Header should include all fields from both arrays
        assert!(lines[0].contains("path"));
        // Should have 2 data rows (1 from each array)
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_csv_null_and_missing_values() {
        let json = json!({
            "symbols": [
                {"name": "alpha", "kind": "function"},
                {"name": "beta"}
            ]
        });
        let csv = json_to_csv(&json, "symbols");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "kind,name");
        assert_eq!(lines[1], "function,alpha");
        // Missing 'kind' should be empty
        assert_eq!(lines[2], ",beta");
    }

    #[test]
    fn test_csv_boolean_and_number_values() {
        let json = json!({
            "exports": [
                {"name": "foo", "is_used": true, "line": 42}
            ]
        });
        let csv = json_to_csv(&json, "exports");
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[1].contains("true"));
        assert!(lines[1].contains("42"));
    }

    // =========================================================================
    // csv_escape_value tests
    // =========================================================================

    #[test]
    fn test_csv_escape_null() {
        assert_eq!(csv_escape_value(None), "");
        assert_eq!(
            csv_escape_value(Some(&serde_json::Value::Null)),
            ""
        );
    }

    #[test]
    fn test_csv_escape_simple_string() {
        let v = json!("hello");
        assert_eq!(csv_escape_value(Some(&v)), "hello");
    }

    #[test]
    fn test_csv_escape_string_with_comma() {
        let v = json!("hello, world");
        assert_eq!(csv_escape_value(Some(&v)), "\"hello, world\"");
    }

    #[test]
    fn test_csv_escape_number() {
        let v = json!(42);
        assert_eq!(csv_escape_value(Some(&v)), "42");
    }

    // =========================================================================
    // apply_jq_filter tests
    // =========================================================================

    #[test]
    fn test_jq_identity() {
        let input = json!({"a": 1, "b": 2});
        let results = apply_jq_filter(&input, ".").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["a"], 1);
        assert_eq!(results[0]["b"], 2);
    }

    #[test]
    fn test_jq_field_access() {
        let input = json!({"name": "test", "count": 42});
        let results = apply_jq_filter(&input, ".name").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "test");
    }

    #[test]
    fn test_jq_array_iteration() {
        let input = json!({"items": ["a", "b", "c"]});
        let results = apply_jq_filter(&input, ".items[]").unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], "a");
        assert_eq!(results[1], "b");
        assert_eq!(results[2], "c");
    }

    #[test]
    fn test_jq_if_then_filter() {
        let input = json!({
            "dead_files": [
                {"path": "a.ts", "confidence": "high"},
                {"path": "b.ts", "confidence": "low"},
                {"path": "c.ts", "confidence": "high"}
            ]
        });
        // Use if/then/else/empty instead of select (which requires std lib)
        let results = apply_jq_filter(
            &input,
            r#"[.dead_files[] | if .confidence == "high" then . else empty end]"#,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        let arr = results[0].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["path"], "a.ts");
        assert_eq!(arr[1]["path"], "c.ts");
    }

    #[test]
    fn test_jq_map_paths() {
        let input = json!({
            "dead_files": [
                {"path": "src/a.ts"},
                {"path": "src/b.ts"}
            ]
        });
        let results = apply_jq_filter(&input, "[.dead_files[].path]").unwrap();
        assert_eq!(results.len(), 1);
        let arr = results[0].as_array().unwrap();
        assert_eq!(arr[0], "src/a.ts");
        assert_eq!(arr[1], "src/b.ts");
    }

    #[test]
    fn test_jq_invalid_expression() {
        let input = json!({"a": 1});
        let result = apply_jq_filter(&input, "[[[invalid syntax");
        assert!(result.is_err());
    }

    // =========================================================================
    // extension_to_language tests
    // =========================================================================

    #[test]
    fn test_extension_to_language_typescript() {
        assert_eq!(extension_to_language("ts"), "TypeScript");
        assert_eq!(extension_to_language("tsx"), "TypeScript");
        assert_eq!(extension_to_language("mts"), "TypeScript");
        assert_eq!(extension_to_language("cts"), "TypeScript");
    }

    #[test]
    fn test_extension_to_language_javascript() {
        assert_eq!(extension_to_language("js"), "JavaScript");
        assert_eq!(extension_to_language("jsx"), "JavaScript");
        assert_eq!(extension_to_language("mjs"), "JavaScript");
        assert_eq!(extension_to_language("cjs"), "JavaScript");
    }

    #[test]
    fn test_extension_to_language_java() {
        assert_eq!(extension_to_language("java"), "Java");
    }

    #[test]
    fn test_extension_to_language_rust() {
        assert_eq!(extension_to_language("rs"), "Rust");
    }

    #[test]
    fn test_extension_to_language_unknown() {
        assert_eq!(extension_to_language("py"), "");
        assert_eq!(extension_to_language(""), "");
    }

    // =========================================================================
    // enrich_path_fields tests (including language enrichment)
    // =========================================================================

    #[test]
    fn test_enrich_path_adds_language() {
        let mut json = json!({
            "dead_files": [
                {"path": "src/utils.ts"},
                {"path": "src/Main.java"},
                {"path": "src/lib.rs"}
            ]
        });
        enrich_path_fields(&mut json);

        let files = json["dead_files"].as_array().unwrap();
        assert_eq!(files[0]["path_language"], "TypeScript");
        assert_eq!(files[1]["path_language"], "Java");
        assert_eq!(files[2]["path_language"], "Rust");
    }

    #[test]
    fn test_enrich_path_adds_directory_filename_extension() {
        let mut json = json!({
            "exports": [
                {"path": "src/utils/format.ts"}
            ]
        });
        enrich_path_fields(&mut json);

        let item = &json["exports"][0];
        assert_eq!(item["path_directory"], "src/utils");
        assert_eq!(item["path_filename"], "format.ts");
        assert_eq!(item["path_extension"], "ts");
        assert_eq!(item["path_language"], "TypeScript");
    }

    #[test]
    fn test_enrich_path_preserves_existing_fields() {
        let mut json = json!({
            "exports": [
                {"path": "src/a.ts", "path_language": "custom"}
            ]
        });
        enrich_path_fields(&mut json);

        let item = &json["exports"][0];
        assert_eq!(item["path_language"], "custom");
    }

    // =========================================================================
    // is_problem_command tests
    // =========================================================================

    #[test]
    fn test_problem_commands_exit_1() {
        assert!(is_problem_command("dead-code"));
        assert!(is_problem_command("cycles"));
        assert!(is_problem_command("lint"));
    }

    #[test]
    fn test_informational_commands_exit_0() {
        assert!(!is_problem_command("deps"));
        assert!(!is_problem_command("exports"));
        assert!(!is_problem_command("symbols"));
        assert!(!is_problem_command("references"));
        assert!(!is_problem_command("callers"));
        assert!(!is_problem_command("summary"));
        assert!(!is_problem_command("impact"));
        assert!(!is_problem_command("diff"));
        assert!(!is_problem_command("index"));
    }

    // =========================================================================
    // extract_count dead_symbols test
    // =========================================================================

    #[test]
    fn test_extract_count_dead_code_symbols() {
        let json = json!({
            "dead_symbols": [{"name": "foo"}, {"name": "bar"}, {"name": "baz"}],
            "summary": {"dead_symbols": 3}
        });
        assert_eq!(extract_count(&json, "dead-code"), 3);
    }

    #[test]
    fn test_extract_count_dead_code_all_scopes() {
        let json = json!({
            "dead_files": [{"path": "a.ts"}],
            "dead_exports": [{"name": "x"}],
            "dead_symbols": [{"name": "y"}, {"name": "z"}],
            "summary": {"dead_files": 1, "dead_exports": 1, "dead_symbols": 2}
        });
        assert_eq!(extract_count(&json, "dead-code"), 4);
    }
}
