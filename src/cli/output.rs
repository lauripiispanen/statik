use super::OutputFormat;

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
