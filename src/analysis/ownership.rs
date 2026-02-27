use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::git::CommitRecord;

/// Ownership score for a single author on a file or set of files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipScore {
    pub author_name: String,
    pub author_email: String,
    /// Ownership percentage (0.0 to 100.0).
    pub score: f64,
}

/// Per-file ownership result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOwnership {
    pub path: String,
    pub owners: Vec<OwnershipScore>,
}

/// Result of the owners command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnersResult {
    pub command: String,
    pub files: Vec<FileOwnership>,
    pub count: usize,
    pub summary: OwnersSummary,
}

/// Compute ownership scores from commit records for a single file.
///
/// Each commit contributes `recency_weight * volume_weight` to its author's score.
/// - Recency weight: `exp(-ln(2) * days_since / half_life_days)`
/// - Volume weight: `lines_added + lines_removed`
///
/// Scores are normalized to sum to 100%.
pub fn compute_ownership(
    commits: &[CommitRecord],
    now_timestamp: i64,
    half_life_days: f64,
) -> Vec<OwnershipScore> {
    if commits.is_empty() {
        return Vec::new();
    }

    let ln2 = std::f64::consts::LN_2;

    // Accumulate raw scores per author (keyed by email)
    let mut author_scores: std::collections::HashMap<String, (String, f64)> =
        std::collections::HashMap::new();

    for commit in commits {
        let days_since = ((now_timestamp - commit.timestamp) as f64) / 86400.0;
        let recency_weight = (-ln2 * days_since / half_life_days).exp();

        // Volume from this commit's file changes
        let volume: u64 = commit
            .files
            .iter()
            .map(|f| f.lines_added + f.lines_removed)
            .sum();

        // Ensure at least 1 for commits that only add/remove files (0 line changes)
        let volume_weight = (volume as f64).max(1.0);
        let raw_score = recency_weight * volume_weight;

        let entry = author_scores
            .entry(commit.author_email.clone())
            .or_insert_with(|| (commit.author_name.clone(), 0.0));
        entry.1 += raw_score;
    }

    // Normalize to percentages
    let total: f64 = author_scores.values().map(|(_, s)| s).sum();
    if total == 0.0 {
        return Vec::new();
    }

    let mut scores: Vec<OwnershipScore> = author_scores
        .into_iter()
        .map(|(email, (name, raw))| OwnershipScore {
            author_name: name,
            author_email: email,
            score: (raw / total) * 100.0,
        })
        .collect();

    // Sort descending by score
    scores.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    scores
}

/// Compute ownership for a single file from the database.
pub fn compute_file_ownership(
    db: &Database,
    file_path: &str,
    half_life_days: f64,
    now_timestamp: i64,
) -> anyhow::Result<Vec<OwnershipScore>> {
    let commits = db.get_commits_for_file(file_path)?;
    Ok(compute_ownership(&commits, now_timestamp, half_life_days))
}

/// Compute aggregate ownership across all files matching a glob pattern.
///
/// Collects all file-commit associations and computes ownership across all of them.
pub fn compute_directory_ownership(
    db: &Database,
    file_paths: &[String],
    half_life_days: f64,
    now_timestamp: i64,
) -> anyhow::Result<Vec<OwnershipScore>> {
    let mut all_commits = Vec::new();
    for path in file_paths {
        let commits = db.get_commits_for_file(path)?;
        all_commits.extend(commits);
    }
    Ok(compute_ownership(&all_commits, now_timestamp, half_life_days))
}

/// Default half-life for ownership scoring (in days).
pub const DEFAULT_HALF_LIFE_DAYS: f64 = 180.0;

/// Summary statistics for the owners command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnersSummary {
    pub files_analyzed: usize,
    pub unique_authors: usize,
}

/// Compute ownership for all files in the database, optionally filtered by glob.
///
/// Returns per-file ownership with `top` owners per file.
pub fn compute_owners(
    db: &Database,
    glob_pattern: Option<&str>,
    top: usize,
    half_life_days: f64,
) -> anyhow::Result<OwnersResult> {
    let all_file_commits = db.get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!(
                "No commit history found. Run `statik index --with-history` first."
            );
        }
    }

    // Group commits by file path
    let mut by_file: std::collections::HashMap<String, Vec<CommitRecord>> =
        std::collections::HashMap::new();
    for (path, commit) in all_file_commits {
        by_file.entry(path).or_default().push(commit);
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

    // Use current time as "now" for recency calculation
    let now_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut all_authors: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut files: Vec<FileOwnership> = Vec::new();

    let mut sorted_paths: Vec<String> = by_file.keys().cloned().collect();
    sorted_paths.sort();

    for path in &sorted_paths {
        if let Some(ref matcher) = glob_matcher {
            if !matcher.is_match(path) {
                continue;
            }
        }

        let commits = &by_file[path];
        let mut owners = compute_ownership(commits, now_timestamp, half_life_days);

        for owner in &owners {
            all_authors.insert(owner.author_email.clone());
        }

        owners.truncate(top);

        files.push(FileOwnership {
            path: path.clone(),
            owners,
        });
    }

    let count = files.len();
    Ok(OwnersResult {
        command: "owners".to_string(),
        files,
        count,
        summary: OwnersSummary {
            files_analyzed: count,
            unique_authors: all_authors.len(),
        },
    })
}

/// Bus factor analysis result for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusFactorEntry {
    pub path: String,
    /// Number of significant contributors (above threshold).
    pub bus_factor: usize,
    /// Primary owner of the file.
    pub primary_owner: OwnershipScore,
    /// Number of files that depend on this file (fan-in).
    pub fan_in: usize,
    /// Risk score: fan_in / bus_factor. Higher = more organizational risk.
    pub risk_score: f64,
}

/// Result of the bus-factor command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusFactorResult {
    pub command: String,
    pub files: Vec<BusFactorEntry>,
    pub count: usize,
}

/// Count how many authors hold at least `threshold` percentage of ownership.
///
/// `threshold` is in the range 0.0-1.0 (e.g., 0.1 = 10%).
pub fn compute_bus_factor(owners: &[OwnershipScore], threshold: f64) -> usize {
    let threshold_pct = threshold * 100.0;
    owners.iter().filter(|o| o.score >= threshold_pct).count()
}

/// Compute bus-factor analysis for all files in the database.
///
/// Cross-references ownership concentration with dependency fan-in
/// to surface organizational risk.
pub fn compute_bus_factor_analysis(
    db: &Database,
    graph: &crate::model::file_graph::FileGraph,
    glob_pattern: Option<&str>,
    threshold: f64,
    half_life_days: f64,
) -> anyhow::Result<BusFactorResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!(
                "No commit history found. Run `statik index --with-history` first."
            );
        }
    }

    // Group commits by file path
    let mut by_file: std::collections::HashMap<String, Vec<CommitRecord>> =
        std::collections::HashMap::new();
    for (path, commit) in all_file_commits {
        by_file.entry(path).or_default().push(commit);
    }

    // Build glob matcher
    let glob_matcher = match glob_pattern {
        Some(pattern) => Some(
            globset::Glob::new(pattern)
                .with_context(|| format!("Invalid glob pattern: {}", pattern))?
                .compile_matcher(),
        ),
        None => None,
    };

    // Build path-to-fan-in lookup from the file graph.
    // File graph uses absolute paths; commit history uses relative paths.
    // We store both the full path and attempt suffix matching for lookups.
    let mut fan_in_by_path: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (_, info) in graph.all_files() {
        let full_path = info.path.to_string_lossy().to_string();
        let importers = graph.direct_importers(info.id);
        let fan_in = importers.len();
        fan_in_by_path.insert(full_path, fan_in);
    }

    // Build a suffix-match lookup: for each commit history path (relative),
    // find the matching file graph entry (absolute) by checking if any
    // absolute path ends with the relative path.
    let fan_in_suffix_lookup = |rel_path: &str| -> usize {
        // Direct match first
        if let Some(&val) = fan_in_by_path.get(rel_path) {
            return val;
        }
        // Suffix match: check if any absolute path ends with /rel_path
        let suffix = format!("/{}", rel_path);
        for (abs_path, &fan_in) in &fan_in_by_path {
            if abs_path.ends_with(&suffix) {
                return fan_in;
            }
        }
        0
    };

    let now_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut entries = Vec::new();

    let mut sorted_paths: Vec<String> = by_file.keys().cloned().collect();
    sorted_paths.sort();

    for path in &sorted_paths {
        if let Some(ref matcher) = glob_matcher {
            if !matcher.is_match(path) {
                continue;
            }
        }

        let commits = &by_file[path];
        let owners = compute_ownership(commits, now_timestamp, half_life_days);

        if owners.is_empty() {
            continue;
        }

        let bus_factor = compute_bus_factor(&owners, threshold);
        let fan_in = fan_in_suffix_lookup(path);
        let risk_score = if bus_factor > 0 {
            fan_in as f64 / bus_factor as f64
        } else {
            fan_in as f64
        };

        entries.push(BusFactorEntry {
            path: path.clone(),
            bus_factor,
            primary_owner: owners[0].clone(),
            fan_in,
            risk_score,
        });
    }

    // Sort by risk_score descending (highest risk first)
    entries.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let count = entries.len();
    Ok(BusFactorResult {
        command: "bus-factor".to_string(),
        files: entries,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{CommitRecord, FileChange};

    fn make_commit(
        sha: &str,
        name: &str,
        email: &str,
        timestamp: i64,
        lines_added: u64,
        lines_removed: u64,
    ) -> CommitRecord {
        CommitRecord {
            sha: sha.to_string(),
            author_name: name.to_string(),
            author_email: email.to_string(),
            timestamp,
            files: vec![FileChange {
                path: "test.rs".to_string(),
                lines_added,
                lines_removed,
            }],
        }
    }

    #[test]
    fn test_empty_commits() {
        let scores = compute_ownership(&[], 1700000000, 180.0);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_single_author() {
        let commits = vec![make_commit("sha1", "Alice", "alice@example.com", 1700000000, 10, 5)];
        let scores = compute_ownership(&commits, 1700000000, 180.0);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].author_name, "Alice");
        assert!((scores[0].score - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_two_authors_equal_recent() {
        // Same timestamp, same volume -> equal ownership
        let now = 1700000000;
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 10, 0),
            make_commit("sha2", "Bob", "bob@example.com", now, 10, 0),
        ];
        let scores = compute_ownership(&commits, now, 180.0);
        assert_eq!(scores.len(), 2);
        assert!((scores[0].score - 50.0).abs() < 0.01);
        assert!((scores[1].score - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_recency_weighting() {
        let now = 1700000000;
        let one_year_ago = now - 365 * 86400;

        // Alice committed recently, Bob committed a year ago, same volume
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 10, 0),
            make_commit("sha2", "Bob", "bob@example.com", one_year_ago, 10, 0),
        ];
        let scores = compute_ownership(&commits, now, 180.0);

        // Alice should score much higher than Bob
        let alice = scores.iter().find(|s| s.author_name == "Alice").unwrap();
        let bob = scores.iter().find(|s| s.author_name == "Bob").unwrap();
        assert!(alice.score > bob.score, "Recent commits should score higher");
        assert!(alice.score > 70.0, "Alice should dominate with recent commit");
    }

    #[test]
    fn test_volume_weighting() {
        let now = 1700000000;

        // Same timestamp, but Alice changed more lines
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 100, 50),
            make_commit("sha2", "Bob", "bob@example.com", now, 5, 0),
        ];
        let scores = compute_ownership(&commits, now, 180.0);

        let alice = scores.iter().find(|s| s.author_name == "Alice").unwrap();
        let bob = scores.iter().find(|s| s.author_name == "Bob").unwrap();
        assert!(alice.score > bob.score, "Higher volume should score higher");
    }

    #[test]
    fn test_scores_sum_to_100() {
        let now = 1700000000;
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 10, 5),
            make_commit("sha2", "Bob", "bob@example.com", now - 86400, 20, 3),
            make_commit("sha3", "Charlie", "charlie@example.com", now - 86400 * 30, 8, 2),
        ];
        let scores = compute_ownership(&commits, now, 180.0);

        let total: f64 = scores.iter().map(|s| s.score).sum();
        assert!(
            (total - 100.0).abs() < 0.01,
            "Scores should sum to 100%, got {}",
            total
        );
    }

    #[test]
    fn test_sorted_descending() {
        let now = 1700000000;
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 5, 0),
            make_commit("sha2", "Bob", "bob@example.com", now, 50, 0),
            make_commit("sha3", "Charlie", "charlie@example.com", now, 20, 0),
        ];
        let scores = compute_ownership(&commits, now, 180.0);

        assert_eq!(scores[0].author_name, "Bob");
        assert_eq!(scores[1].author_name, "Charlie");
        assert_eq!(scores[2].author_name, "Alice");
    }

    #[test]
    fn test_same_author_multiple_commits() {
        let now = 1700000000;
        // Alice has two commits
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 10, 0),
            make_commit("sha2", "Alice", "alice@example.com", now - 86400, 10, 0),
            make_commit("sha3", "Bob", "bob@example.com", now, 10, 0),
        ];
        let scores = compute_ownership(&commits, now, 180.0);

        let alice = scores.iter().find(|s| s.author_name == "Alice").unwrap();
        let bob = scores.iter().find(|s| s.author_name == "Bob").unwrap();
        // Alice has two commits vs Bob's one, so she should score higher
        assert!(alice.score > bob.score);
    }

    #[test]
    fn test_half_life_at_boundary() {
        let now = 1700000000;
        let half_life_days = 180.0;
        let half_life_ago = now - (half_life_days as i64) * 86400;

        // Alice: now, Bob: exactly one half-life ago, same volume
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", now, 10, 0),
            make_commit("sha2", "Bob", "bob@example.com", half_life_ago, 10, 0),
        ];
        let scores = compute_ownership(&commits, now, half_life_days);

        let alice = scores.iter().find(|s| s.author_name == "Alice").unwrap();
        let bob = scores.iter().find(|s| s.author_name == "Bob").unwrap();

        // At one half-life, Bob's weight should be half of Alice's
        // So Alice ~ 66.7%, Bob ~ 33.3%
        assert!(
            (alice.score - 66.67).abs() < 1.0,
            "Alice should be ~66.7%, got {}",
            alice.score
        );
        assert!(
            (bob.score - 33.33).abs() < 1.0,
            "Bob should be ~33.3%, got {}",
            bob.score
        );
    }

    #[test]
    fn test_db_file_ownership() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_commit("sha2", "Bob", "bob@example.com", now - 86400)
            .unwrap();
        db.insert_file_commit("src/main.rs", "sha1", 20, 5)
            .unwrap();
        db.insert_file_commit("src/main.rs", "sha2", 10, 2)
            .unwrap();

        let scores = compute_file_ownership(&db, "src/main.rs", 180.0, now).unwrap();
        assert_eq!(scores.len(), 2);
        // Alice should score higher (more recent, more volume)
        assert_eq!(scores[0].author_name, "Alice");
    }

    // ---- Bus factor tests ----

    #[test]
    fn test_bus_factor_single_owner() {
        let owners = vec![OwnershipScore {
            author_name: "Alice".to_string(),
            author_email: "alice@example.com".to_string(),
            score: 100.0,
        }];
        assert_eq!(compute_bus_factor(&owners, 0.1), 1);
    }

    #[test]
    fn test_bus_factor_two_equal_owners() {
        let owners = vec![
            OwnershipScore {
                author_name: "Alice".to_string(),
                author_email: "alice@example.com".to_string(),
                score: 50.0,
            },
            OwnershipScore {
                author_name: "Bob".to_string(),
                author_email: "bob@example.com".to_string(),
                score: 50.0,
            },
        ];
        assert_eq!(compute_bus_factor(&owners, 0.1), 2);
    }

    #[test]
    fn test_bus_factor_threshold_filters() {
        let owners = vec![
            OwnershipScore {
                author_name: "Alice".to_string(),
                author_email: "alice@example.com".to_string(),
                score: 80.0,
            },
            OwnershipScore {
                author_name: "Bob".to_string(),
                author_email: "bob@example.com".to_string(),
                score: 15.0,
            },
            OwnershipScore {
                author_name: "Charlie".to_string(),
                author_email: "charlie@example.com".to_string(),
                score: 5.0,
            },
        ];
        // At 10% threshold, Alice (80%) and Bob (15%) count, Charlie (5%) doesn't
        assert_eq!(compute_bus_factor(&owners, 0.1), 2);
        // At 20% threshold, only Alice counts
        assert_eq!(compute_bus_factor(&owners, 0.2), 1);
    }

    #[test]
    fn test_bus_factor_empty() {
        assert_eq!(compute_bus_factor(&[], 0.1), 0);
    }
}
