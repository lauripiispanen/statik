use serde::Serialize;

use super::OutputFormat;
use crate::model::Symbol;

/// Format a list of symbols for output.
pub fn format_symbols(symbols: &[Symbol], format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(symbols).unwrap_or_default(),
        OutputFormat::Compact => serde_json::to_string(symbols).unwrap_or_default(),
        OutputFormat::Text => {
            let mut output = String::new();
            for s in symbols {
                output.push_str(&format!(
                    "{:<12} {:<40} (line {})\n",
                    s.kind, s.qualified_name, s.line_span.start.line,
                ));
            }
            output
        }
    }
}

/// Format any serializable value as JSON.
pub fn format_json<T: Serialize>(value: &T, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(value).unwrap_or_default(),
        OutputFormat::Compact => serde_json::to_string(value).unwrap_or_default(),
        OutputFormat::Text => serde_json::to_string_pretty(value).unwrap_or_default(),
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
        OutputFormat::Json | OutputFormat::Compact => {
            let summary = serde_json::json!({
                "files_indexed": files,
                "symbols_extracted": symbols,
                "references_found": references,
                "duration_ms": duration_ms,
            });
            if matches!(format, OutputFormat::Json) {
                serde_json::to_string_pretty(&summary).unwrap_or_default()
            } else {
                serde_json::to_string(&summary).unwrap_or_default()
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
