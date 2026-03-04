use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::db::Database;
use crate::discovery::{discover_files, DiscoveryConfig};
use crate::model::{FileId, FileRecord, Language, ParseResult};
use crate::parser::ParserRegistry;

/// Parser version stamp. Bump this when parser logic changes in a way that
/// affects stored import/export/symbol data (e.g., new extraction rules).
/// When the stored version differs from this constant, a full re-index is
/// triggered automatically.
pub const PARSER_VERSION: &str = "1";

/// Run the indexing process for a project.
pub fn run_index(
    project_path: &Path,
    config: &DiscoveryConfig,
    force: bool,
) -> Result<IndexResult> {
    let start = Instant::now();

    // Ensure .statik directory exists
    let statik_dir = project_path.join(".statik");
    std::fs::create_dir_all(&statik_dir).context("failed to create .statik directory")?;

    let db_path = statik_dir.join("index.db");

    // If --force, delete the existing database to start fresh
    if force && db_path.exists() {
        std::fs::remove_file(&db_path).context("failed to remove existing index.db")?;
    }

    let db = Database::open(&db_path)?;

    // Check parser version: if it changed, do a full re-index
    let version_changed = match db.get_metadata("parser_version")? {
        Some(stored) => stored != PARSER_VERSION,
        None => false, // First index, no version stored yet
    };
    if version_changed {
        eprintln!("Parser version changed, performing full re-index...");
        db.clear_all_data()?;
    }

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
            files_to_parse.push((existing.id, df.clone(), true));
        } else {
            // New file
            let file_id = FileId(next_file_id);
            next_file_id += 1;
            files_to_parse.push((file_id, df.clone(), false));
        }
    }

    // Detect deleted files
    let deleted_files: Vec<_> = existing_files
        .iter()
        .filter(|(path, _)| !current_paths.contains(path.as_str()))
        .map(|(_, f)| f.id)
        .collect();

    // Parse files in parallel, skipping languages without parsers
    let registry = ParserRegistry::with_defaults();

    // Partition files into parseable and skipped (no parser for language)
    let (parseable, skipped): (Vec<_>, Vec<_>) = files_to_parse
        .iter()
        .partition(|(_, df, _)| registry.parser_for(df.language).is_some());
    let files_skipped_no_parser = skipped.len();

    let parse_results: Vec<(FileId, Language, String, bool, Result<ParseResult>)> = parseable
        .par_iter()
        .map(|(file_id, df, is_existing)| {
            let source = std::fs::read_to_string(&df.path)
                .with_context(|| format!("failed to read {}", df.path.display()));
            match source {
                Ok(source) => {
                    let result = registry.parse(*file_id, &source, &df.path, df.language);
                    (
                        *file_id,
                        df.language,
                        df.path.to_string_lossy().to_string(),
                        *is_existing,
                        result,
                    )
                }
                Err(e) => (
                    *file_id,
                    df.language,
                    df.path.to_string_lossy().to_string(),
                    *is_existing,
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
    let files_to_parse_map: HashMap<FileId, &crate::discovery::DiscoveredFile> =
        files_to_parse.iter().map(|(id, df, _)| (*id, df)).collect();

    // Batch-clear old data for changed files (5 DELETEs total instead of 5*N)
    let changed_ids: Vec<FileId> = parse_results
        .iter()
        .filter(|(_, _, _, is_existing, result)| *is_existing && result.is_ok())
        .map(|(file_id, _, _, _, _)| *file_id)
        .collect();
    db.clear_files_data_batch(&changed_ids)?;

    let mut total_symbols = 0;
    let mut total_references = 0;
    let mut parse_errors = Vec::new();

    for (file_id, language, path_str, _is_existing, result) in &parse_results {
        match result {
            Ok(parse_result) => {

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

                // Insert suppressions
                if !parse_result.suppressions.is_empty() {
                    db.store_suppressions(*file_id, &parse_result.suppressions)?;
                }
            }
            Err(e) => {
                parse_errors.push(format!("{}: {}", path_str, e));
            }
        }
    }

    db.commit_transaction()?;

    // Store current parser version
    db.set_metadata("parser_version", PARSER_VERSION)?;

    let duration = start.elapsed();

    Ok(IndexResult {
        files_indexed: parse_results.len(),
        files_unchanged: unchanged_count,
        files_deleted: deleted_files.len(),
        symbols_extracted: total_symbols,
        references_found: total_references,
        parse_errors,
        files_skipped_no_parser,
        duration_ms: duration.as_millis(),
    })
}

/// Index git commit history into the database.
///
/// If `force` is true, clears existing history and re-indexes everything.
/// Otherwise performs incremental indexing from the last indexed commit.
pub fn run_history_index(
    project_path: &Path,
    db: &Database,
    max_commits: Option<usize>,
    force: bool,
) -> Result<HistoryResult> {
    let start = Instant::now();

    if !crate::git::is_git_repo(project_path) {
        anyhow::bail!("--with-history requires a git repository");
    }

    // Determine since_sha for incremental indexing
    let since_sha = if force {
        db.clear_history()?;
        None
    } else {
        db.get_last_indexed_commit_sha()?
    };

    let commits = crate::git::git_log_numstat(project_path, since_sha.as_deref(), max_commits)?;

    if commits.is_empty() {
        return Ok(HistoryResult {
            commits_indexed: 0,
            file_changes_indexed: 0,
            duration_ms: start.elapsed().as_millis(),
        });
    }

    // The most recent commit SHA (first in the list, git log returns newest first)
    let newest_sha = commits[0].sha.clone();

    db.begin_transaction()?;

    let mut total_file_changes = 0;
    for commit in &commits {
        db.insert_commit(
            &commit.sha,
            &commit.author_name,
            &commit.author_email,
            commit.timestamp,
        )?;

        for file_change in &commit.files {
            db.insert_file_commit(
                &file_change.path,
                &commit.sha,
                file_change.lines_added,
                file_change.lines_removed,
            )?;
            total_file_changes += 1;
        }
    }

    db.set_last_indexed_commit_sha(&newest_sha)?;
    db.commit_transaction()?;

    Ok(HistoryResult {
        commits_indexed: commits.len(),
        file_changes_indexed: total_file_changes,
        duration_ms: start.elapsed().as_millis(),
    })
}

#[derive(Debug)]
pub struct HistoryResult {
    pub commits_indexed: usize,
    pub file_changes_indexed: usize,
    pub duration_ms: u128,
}

#[derive(Debug)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub files_unchanged: usize,
    pub files_deleted: usize,
    pub symbols_extracted: usize,
    pub references_found: usize,
    pub parse_errors: Vec<String>,
    pub files_skipped_no_parser: usize,
    pub duration_ms: u128,
}
