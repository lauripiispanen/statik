use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::model::file_graph::FileGraph;
use crate::model::FileId;

/// Change frequency result for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnEntry {
    pub path: String,
    pub commit_count: usize,
    pub lines_changed: u64,
    /// Commits per 30-day period within the observation window.
    pub frequency: f64,
}

/// Result of the churn command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnResult {
    pub command: String,
    pub files: Vec<ChurnEntry>,
    pub count: usize,
    pub summary: ChurnSummary,
}

/// Summary statistics for churn analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnSummary {
    pub files_analyzed: usize,
    pub total_commits: usize,
}

/// Co-change result for a pair of files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeEntry {
    pub file_a: String,
    pub file_b: String,
    pub co_change_count: usize,
    pub co_change_ratio: f64,
    pub has_import_edge: bool,
    pub hidden_coupling: bool,
}

/// Result of the co-change analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeResult {
    pub command: String,
    pub pairs: Vec<CoChangeEntry>,
    pub count: usize,
    pub summary: CoChangeSummary,
}

/// Summary statistics for co-change analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeSummary {
    pub pairs_found: usize,
    pub hidden_coupling_count: usize,
}

/// Compute file change frequency (churn) from commit history.
///
/// Returns one entry per file with commit count, total lines changed,
/// and frequency (commits per 30-day period).
pub fn compute_churn(
    db: &Database,
    glob_pattern: Option<&str>,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<ChurnResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!("No commit history found. Run `statik index --with-history` first.");
        }
    }

    // Apply glob filter
    let glob_matcher = match glob_pattern {
        Some(pattern) => Some(
            globset::Glob::new(pattern)
                .with_context(|| format!("Invalid glob pattern: {}", pattern))?
                .compile_matcher(),
        ),
        None => None,
    };

    // Group commits by file path, applying time window filter
    let mut by_file: HashMap<String, Vec<(i64, u64, u64)>> = HashMap::new();
    for (path, commit) in &all_file_commits {
        // Apply glob filter
        if let Some(ref matcher) = glob_matcher {
            if !matcher.is_match(path) {
                continue;
            }
        }

        // Apply time window filter
        if let Some(since_ts) = since {
            if commit.timestamp < since_ts {
                continue;
            }
        }
        if let Some(until_ts) = until {
            if commit.timestamp > until_ts {
                continue;
            }
        }

        for fc in &commit.files {
            by_file.entry(path.clone()).or_default().push((
                commit.timestamp,
                fc.lines_added,
                fc.lines_removed,
            ));
        }
    }

    // Compute the observation window for frequency calculation
    let all_timestamps: Vec<i64> = by_file
        .values()
        .flat_map(|v| v.iter().map(|(ts, _, _)| *ts))
        .collect();

    let window_start = since.unwrap_or_else(|| all_timestamps.iter().copied().min().unwrap_or(0));
    let window_end = until.unwrap_or_else(|| all_timestamps.iter().copied().max().unwrap_or(0));
    let window_days = ((window_end - window_start) as f64 / 86400.0).max(1.0);
    let periods_30d = window_days / 30.0;

    let mut files: Vec<ChurnEntry> = Vec::new();
    let mut total_commits = 0usize;

    let mut sorted_paths: Vec<String> = by_file.keys().cloned().collect();
    sorted_paths.sort();

    for path in &sorted_paths {
        let entries = &by_file[path];
        let commit_count = entries.len();
        let lines_changed: u64 = entries.iter().map(|(_, a, r)| a + r).sum();
        let frequency = commit_count as f64 / periods_30d;

        total_commits += commit_count;

        files.push(ChurnEntry {
            path: path.clone(),
            commit_count,
            lines_changed,
            frequency,
        });
    }

    // Default sort by commit_count descending
    files.sort_by(|a, b| b.commit_count.cmp(&a.commit_count));

    let count = files.len();
    Ok(ChurnResult {
        command: "churn".to_string(),
        files,
        count,
        summary: ChurnSummary {
            files_analyzed: count,
            total_commits,
        },
    })
}

/// Compute co-change analysis: find file pairs that frequently change together.
///
/// For each pair of files appearing in the same commit, counts co-occurrences.
/// `co_change_ratio = co_changes / max(changes_a, changes_b)`.
/// Checks the FileGraph for import edges between the pair.
/// Flags "hidden coupling" when `co_change_ratio > 0.5` and there's no import edge.
pub fn compute_co_changes(
    db: &Database,
    graph: &FileGraph,
    glob_pattern: Option<&str>,
    min_co_changes: usize,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<CoChangeResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!("No commit history found. Run `statik index --with-history` first.");
        }
    }

    let glob_matcher = match glob_pattern {
        Some(pattern) => Some(
            globset::Glob::new(pattern)
                .with_context(|| format!("Invalid glob pattern: {}", pattern))?
                .compile_matcher(),
        ),
        None => None,
    };

    // Group file paths by commit SHA, applying filters
    let mut files_by_commit: HashMap<String, Vec<String>> = HashMap::new();
    let mut file_change_counts: HashMap<String, usize> = HashMap::new();

    for (path, commit) in &all_file_commits {
        if let Some(ref matcher) = glob_matcher {
            if !matcher.is_match(path) {
                continue;
            }
        }
        if let Some(since_ts) = since {
            if commit.timestamp < since_ts {
                continue;
            }
        }
        if let Some(until_ts) = until {
            if commit.timestamp > until_ts {
                continue;
            }
        }

        files_by_commit
            .entry(commit.sha.clone())
            .or_default()
            .push(path.clone());
        *file_change_counts.entry(path.clone()).or_default() += 1;
    }

    // Count co-changes for each pair of files in the same commit
    let mut co_change_counts: HashMap<(String, String), usize> = HashMap::new();
    for files in files_by_commit.values() {
        // Deduplicate files within a commit (a file may appear multiple times)
        let unique: Vec<&String> = {
            let mut seen = std::collections::HashSet::new();
            files.iter().filter(|f| seen.insert(f.as_str())).collect()
        };

        for i in 0..unique.len() {
            for j in (i + 1)..unique.len() {
                let (a, b) = if unique[i] < unique[j] {
                    (unique[i].clone(), unique[j].clone())
                } else {
                    (unique[j].clone(), unique[i].clone())
                };
                *co_change_counts.entry((a, b)).or_default() += 1;
            }
        }
    }

    // Build path -> FileId lookup from the graph
    let path_to_id: HashMap<&str, FileId> = graph
        .files
        .values()
        .map(|f| {
            let path_str = f.path.to_str().unwrap_or("");
            (path_str, f.id)
        })
        .collect();

    // Check if two files have an import edge (in either direction)
    let has_edge = |path_a: &str, path_b: &str| -> bool {
        let id_a = path_to_id.get(path_a).or_else(|| {
            // Try suffix matching for relative paths
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(path_a))
                .map(|f| &f.id)
        });
        let id_b = path_to_id.get(path_b).or_else(|| {
            graph
                .files
                .values()
                .find(|f| f.path.ends_with(path_b))
                .map(|f| &f.id)
        });

        match (id_a, id_b) {
            (Some(&a), Some(&b)) => {
                // Check a -> b
                let a_imports_b = graph
                    .import_edges(a)
                    .map(|edges| edges.iter().any(|e| e.to == b))
                    .unwrap_or(false);
                // Check b -> a
                let b_imports_a = graph
                    .import_edges(b)
                    .map(|edges| edges.iter().any(|e| e.to == a))
                    .unwrap_or(false);
                a_imports_b || b_imports_a
            }
            _ => false,
        }
    };

    let mut pairs: Vec<CoChangeEntry> = Vec::new();
    let mut hidden_coupling_count = 0;

    let mut sorted_pairs: Vec<(&(String, String), &usize)> = co_change_counts.iter().collect();
    sorted_pairs.sort_by(|a, b| b.1.cmp(a.1));

    for ((file_a, file_b), &count) in sorted_pairs {
        if count < min_co_changes {
            continue;
        }

        let changes_a = file_change_counts.get(file_a).copied().unwrap_or(0);
        let changes_b = file_change_counts.get(file_b).copied().unwrap_or(0);
        let max_changes = changes_a.max(changes_b);
        let co_change_ratio = if max_changes > 0 {
            count as f64 / max_changes as f64
        } else {
            0.0
        };

        let has_import_edge = has_edge(file_a, file_b);
        let hidden_coupling = co_change_ratio > 0.5 && !has_import_edge;

        if hidden_coupling {
            hidden_coupling_count += 1;
        }

        pairs.push(CoChangeEntry {
            file_a: file_a.clone(),
            file_b: file_b.clone(),
            co_change_count: count,
            co_change_ratio,
            has_import_edge,
            hidden_coupling,
        });
    }

    let pair_count = pairs.len();
    Ok(CoChangeResult {
        command: "churn".to_string(),
        pairs,
        count: pair_count,
        summary: CoChangeSummary {
            pairs_found: pair_count,
            hidden_coupling_count,
        },
    })
}

/// Parse a date string (YYYY-MM-DD) into a Unix timestamp.
pub fn parse_date_to_timestamp(date_str: &str) -> Result<i64> {
    // Parse as YYYY-MM-DD, assume start of day UTC
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        anyhow::bail!("Invalid date format '{}', expected YYYY-MM-DD", date_str);
    }
    let year: i64 = parts[0]
        .parse()
        .with_context(|| format!("Invalid year in date: {}", date_str))?;
    let month: i64 = parts[1]
        .parse()
        .with_context(|| format!("Invalid month in date: {}", date_str))?;
    let day: i64 = parts[2]
        .parse()
        .with_context(|| format!("Invalid day in date: {}", date_str))?;

    // Simple conversion (not accounting for leap seconds, etc.)
    // Use a rough days-since-epoch calculation
    let days = days_since_epoch(year, month, day);
    Ok(days * 86400)
}

/// Compute days since Unix epoch for a given date.
fn days_since_epoch(year: i64, month: i64, day: i64) -> i64 {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_date_basic() {
        let ts = parse_date_to_timestamp("2023-01-01").unwrap();
        // 2023-01-01 00:00:00 UTC
        assert_eq!(ts, 1672531200);
    }

    #[test]
    fn test_parse_date_invalid() {
        assert!(parse_date_to_timestamp("not-a-date").is_err());
        assert!(parse_date_to_timestamp("2023/01/01").is_err());
    }

    #[test]
    fn test_compute_churn_basic() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_commit("sha2", "Bob", "bob@example.com", now - 86400)
            .unwrap();
        db.insert_commit("sha3", "Alice", "alice@example.com", now - 86400 * 2)
            .unwrap();

        db.insert_file_commit("src/main.rs", "sha1", 10, 2).unwrap();
        db.insert_file_commit("src/main.rs", "sha2", 5, 1).unwrap();
        db.insert_file_commit("src/main.rs", "sha3", 3, 0).unwrap();
        db.insert_file_commit("src/lib.rs", "sha1", 20, 0).unwrap();

        let result = compute_churn(&db, None, None, None).unwrap();
        assert_eq!(result.count, 2);

        // src/main.rs has 3 commits, so it should be first
        assert_eq!(result.files[0].path, "src/main.rs");
        assert_eq!(result.files[0].commit_count, 3);
        assert_eq!(result.files[0].lines_changed, 21); // 10+2 + 5+1 + 3+0

        assert_eq!(result.files[1].path, "src/lib.rs");
        assert_eq!(result.files[1].commit_count, 1);
    }

    #[test]
    fn test_compute_churn_with_glob() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_file_commit("src/main.rs", "sha1", 10, 0).unwrap();
        db.insert_file_commit("tests/test.rs", "sha1", 5, 0)
            .unwrap();

        let result = compute_churn(&db, Some("src/**"), None, None).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.files[0].path, "src/main.rs");
    }

    #[test]
    fn test_compute_churn_with_time_filter() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;
        let old = now - 86400 * 365; // 1 year ago

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_commit("sha2", "Bob", "bob@example.com", old)
            .unwrap();
        db.insert_file_commit("src/main.rs", "sha1", 10, 0).unwrap();
        db.insert_file_commit("src/main.rs", "sha2", 5, 0).unwrap();

        // Only recent commits (since 30 days ago)
        let since = now - 86400 * 30;
        let result = compute_churn(&db, None, Some(since), None).unwrap();
        assert_eq!(result.files[0].commit_count, 1);
    }

    #[test]
    fn test_compute_churn_empty_history() {
        let db = Database::in_memory().unwrap();
        let result = compute_churn(&db, None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_churn_frequency() {
        let db = Database::in_memory().unwrap();
        let day = 86400;

        // 3 commits over 60 days = 1.5 per 30-day period
        db.insert_commit("sha1", "Alice", "alice@example.com", 1000 + 60 * day)
            .unwrap();
        db.insert_commit("sha2", "Alice", "alice@example.com", 1000 + 30 * day)
            .unwrap();
        db.insert_commit("sha3", "Alice", "alice@example.com", 1000)
            .unwrap();
        db.insert_file_commit("f.rs", "sha1", 1, 0).unwrap();
        db.insert_file_commit("f.rs", "sha2", 1, 0).unwrap();
        db.insert_file_commit("f.rs", "sha3", 1, 0).unwrap();

        let result = compute_churn(&db, None, None, None).unwrap();
        assert_eq!(result.files[0].commit_count, 3);
        // 3 commits / (60 days / 30 days per period) = 1.5
        assert!((result.files[0].frequency - 1.5).abs() < 0.01);
    }

    #[test]
    fn test_co_change_basic() {
        use crate::model::file_graph::{FileGraph, FileInfo};
        use crate::model::{FileId, Language};

        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        // Two commits: sha1 touches both files, sha2 touches only main.rs
        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_commit("sha2", "Bob", "bob@example.com", now - 86400)
            .unwrap();
        db.insert_commit("sha3", "Alice", "alice@example.com", now - 86400 * 2)
            .unwrap();
        db.insert_file_commit("src/main.rs", "sha1", 10, 0).unwrap();
        db.insert_file_commit("src/lib.rs", "sha1", 5, 0).unwrap();
        db.insert_file_commit("src/main.rs", "sha2", 3, 0).unwrap();
        db.insert_file_commit("src/main.rs", "sha3", 2, 0).unwrap();
        db.insert_file_commit("src/lib.rs", "sha3", 1, 0).unwrap();

        // Build a minimal graph with no import edges
        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: "src/main.rs".into(),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: true,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: "src/lib.rs".into(),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });

        let result = compute_co_changes(&db, &graph, None, 1, None, None).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.pairs[0].co_change_count, 2); // sha1 and sha3
        assert!(!result.pairs[0].has_import_edge);
        // co_change_ratio = 2 / max(3, 2) = 2/3 = 0.67
        assert!((result.pairs[0].co_change_ratio - 0.67).abs() < 0.01);
        assert!(result.pairs[0].hidden_coupling); // ratio > 0.5 and no edge
    }

    #[test]
    fn test_co_change_min_threshold() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_file_commit("a.rs", "sha1", 1, 0).unwrap();
        db.insert_file_commit("b.rs", "sha1", 1, 0).unwrap();

        let graph = FileGraph::new();

        // min_co_changes = 3, but we only have 1 co-change
        let result = compute_co_changes(&db, &graph, None, 3, None, None).unwrap();
        assert_eq!(result.count, 0);
    }
}
