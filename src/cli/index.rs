use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::db::Database;
use crate::discovery::{discover_files, DiscoveryConfig};
use crate::model::{FileId, FileRecord, Language, ParseResult};
use crate::parser::ParserRegistry;

/// Run the indexing process for a project.
pub fn run_index(project_path: &Path, config: &DiscoveryConfig) -> Result<IndexResult> {
    let start = Instant::now();

    // Ensure .statik directory exists
    let statik_dir = project_path.join(".statik");
    std::fs::create_dir_all(&statik_dir).context("failed to create .statik directory")?;

    let db_path = statik_dir.join("index.db");
    let db = Database::open(&db_path)?;

    // Discover files
    let discovered = discover_files(project_path, config)?;

    // Load existing file records for incremental updates
    let existing_files: HashMap<String, FileRecord> = db
        .all_files()?
        .into_iter()
        .map(|f| (f.path.to_string_lossy().to_string(), f))
        .collect();

    // Determine which files need re-parsing
    let mut files_to_parse = Vec::new();
    let mut unchanged_count = 0;
    let mut next_file_id = existing_files.values().map(|f| f.id.0).max().unwrap_or(0) + 1;

    // Track files that still exist (for detecting deleted files)
    let mut current_paths = std::collections::HashSet::new();

    for df in &discovered {
        let path_str = df.path.to_string_lossy().to_string();
        current_paths.insert(path_str.clone());

        if let Some(existing) = existing_files.get(&path_str) {
            if existing.mtime >= df.mtime {
                unchanged_count += 1;
                continue;
            }
            // File changed - reparse with same ID
            files_to_parse.push((existing.id, df.clone()));
        } else {
            // New file
            let file_id = FileId(next_file_id);
            next_file_id += 1;
            files_to_parse.push((file_id, df.clone()));
        }
    }

    // Detect deleted files
    let deleted_files: Vec<_> = existing_files
        .iter()
        .filter(|(path, _)| !current_paths.contains(path.as_str()))
        .map(|(_, f)| f.id)
        .collect();

    // Parse files in parallel
    let registry = ParserRegistry::with_defaults();

    let parse_results: Vec<(FileId, Language, String, Result<ParseResult>)> = files_to_parse
        .par_iter()
        .map(|(file_id, df)| {
            let source = std::fs::read_to_string(&df.path)
                .with_context(|| format!("failed to read {}", df.path.display()));
            match source {
                Ok(source) => {
                    let result = registry.parse(*file_id, &source, &df.path, df.language);
                    (
                        *file_id,
                        df.language,
                        df.path.to_string_lossy().to_string(),
                        result,
                    )
                }
                Err(e) => (
                    *file_id,
                    df.language,
                    df.path.to_string_lossy().to_string(),
                    Err(e),
                ),
            }
        })
        .collect();

    // Write results to database
    db.begin_transaction()?;

    // Remove deleted files
    for file_id in &deleted_files {
        db.delete_file(*file_id)?;
    }

    // Build FileId -> DiscoveredFile lookup (avoids O(N²) linear scans)
    let files_to_parse_map: HashMap<FileId, &crate::discovery::DiscoveredFile> = files_to_parse
        .iter()
        .map(|(id, df)| (*id, df))
        .collect();

    let mut total_symbols = 0;
    let mut total_references = 0;
    let mut parse_errors = Vec::new();

    for (file_id, language, path_str, result) in &parse_results {
        match result {
            Ok(parse_result) => {
                // Clear old data for this file
                db.clear_file_data(*file_id)?;

                // Upsert file record
                let df = files_to_parse_map[file_id];

                let file_record = FileRecord {
                    id: *file_id,
                    path: df.path.clone(),
                    mtime: df.mtime,
                    language: *language,
                };
                db.upsert_file(&file_record)?;

                // Insert symbols
                for symbol in &parse_result.symbols {
                    db.insert_symbol(symbol)?;
                }
                total_symbols += parse_result.symbols.len();

                // Insert references (only those with resolved targets)
                for reference in &parse_result.references {
                    // Skip unresolved references (placeholder targets)
                    if reference.target.0 < u64::MAX - 1_000_000 {
                        db.insert_reference(reference)?;
                        total_references += 1;
                    }
                }

                // Insert imports
                for import in &parse_result.imports {
                    db.insert_import(import)?;
                }

                // Insert exports
                for export in &parse_result.exports {
                    db.insert_export(export)?;
                }
            }
            Err(e) => {
                parse_errors.push(format!("{}: {}", path_str, e));
            }
        }
    }

    db.commit_transaction()?;

    let duration = start.elapsed();

    Ok(IndexResult {
        files_indexed: parse_results.len(),
        files_unchanged: unchanged_count,
        files_deleted: deleted_files.len(),
        symbols_extracted: total_symbols,
        references_found: total_references,
        parse_errors,
        duration_ms: duration.as_millis(),
    })
}

#[derive(Debug)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub files_unchanged: usize,
    pub files_deleted: usize,
    pub symbols_extracted: usize,
    pub references_found: usize,
    pub parse_errors: Vec<String>,
    pub duration_ms: u128,
}
