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
    scores.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scores
}

/// Half-life mode for ownership scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfLifeMode {
    /// Fixed half-life (the `half_life_days` parameter is used directly).
    Fixed,
    /// Adaptive half-life: scales with file age so old-file creators retain
    /// meaningful ownership.
    ///
    /// `effective_half_life = max(half_life_days, file_age_days * 0.25)`
    Adaptive,
}

/// Compute ownership with adaptive half-life that scales with file age.
///
/// For young files (age < 4 * half_life_days), this behaves identically to
/// `compute_ownership`. For older files the half-life grows proportionally,
/// preventing the original creator's contribution from decaying to zero.
pub fn compute_ownership_adaptive(
    commits: &[CommitRecord],
    now_timestamp: i64,
    base_half_life_days: f64,
) -> Vec<OwnershipScore> {
    if commits.is_empty() {
        return Vec::new();
    }

    // File age = time since earliest commit
    let earliest_timestamp = commits
        .iter()
        .map(|c| c.timestamp)
        .min()
        .unwrap_or(now_timestamp);
    let file_age_days = ((now_timestamp - earliest_timestamp) as f64) / 86400.0;

    // Adaptive half-life: scale with file age, minimum = base_half_life_days
    let half_life = (file_age_days * 0.25).max(base_half_life_days);

    compute_ownership(commits, now_timestamp, half_life)
}

/// Dispatch to the correct ownership computation based on `HalfLifeMode`.
fn compute_ownership_with_mode(
    commits: &[CommitRecord],
    now_timestamp: i64,
    half_life_days: f64,
    mode: HalfLifeMode,
) -> Vec<OwnershipScore> {
    match mode {
        HalfLifeMode::Fixed => compute_ownership(commits, now_timestamp, half_life_days),
        HalfLifeMode::Adaptive => {
            compute_ownership_adaptive(commits, now_timestamp, half_life_days)
        }
    }
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
    Ok(compute_ownership(
        &all_commits,
        now_timestamp,
        half_life_days,
    ))
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
    mode: HalfLifeMode,
) -> anyhow::Result<OwnersResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!("No commit history found. Run `statik index --with-history` first.");
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
        let mut owners = compute_ownership_with_mode(commits, now_timestamp, half_life_days, mode);

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

/// Per-author bus factor entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorBusFactorEntry {
    pub author_name: String,
    pub author_email: String,
    /// Number of files where this person is sole owner (above sole-owner threshold).
    pub sole_owned_files: usize,
    /// Total files this person has touched.
    pub total_files: usize,
    /// Sum of fan_in for their sole-owned files (total blast radius).
    pub total_blast_radius: usize,
    /// Top directories they solely own (up to 5).
    pub key_areas: Vec<String>,
}

/// Result of the `bus-factor --by-author` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorBusFactorResult {
    pub command: String,
    pub authors: Vec<AuthorBusFactorEntry>,
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
    project_root: &std::path::Path,
    mode: HalfLifeMode,
) -> anyhow::Result<BusFactorResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!("No commit history found. Run `statik index --with-history` first.");
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

    // Build relative-path-to-FileId lookup from the file graph.
    // File graph stores absolute paths; commit history stores relative paths.
    // By stripping the project root, we can match them directly.
    let mut rel_path_to_id: std::collections::HashMap<String, crate::model::FileId> =
        std::collections::HashMap::new();
    for (_, info) in graph.all_files() {
        if let Ok(rel) = info.path.strip_prefix(project_root) {
            rel_path_to_id.insert(rel.to_string_lossy().to_string(), info.id);
        }
    }

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
        let owners = compute_ownership_with_mode(commits, now_timestamp, half_life_days, mode);

        if owners.is_empty() {
            continue;
        }

        let bus_factor = compute_bus_factor(&owners, threshold);
        let fan_in = rel_path_to_id
            .get(path)
            .map(|id| graph.direct_importers(*id).len())
            .unwrap_or(0);
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

/// Sole-owner threshold: 80% ownership.
const SOLE_OWNER_THRESHOLD: f64 = 80.0;

/// Compute per-person bus factor view.
///
/// For each author, counts:
/// - Files they solely own (primary owner with > 80% ownership)
/// - Total files they have touched
/// - Total blast radius (sum of fan_in for sole-owned files)
/// - Key areas (top directories by sole-owned file count)
pub fn compute_bus_factor_by_author(
    db: &Database,
    graph: &crate::model::file_graph::FileGraph,
    project_root: &std::path::Path,
    glob_pattern: Option<&str>,
    half_life_days: f64,
    mode: HalfLifeMode,
) -> anyhow::Result<AuthorBusFactorResult> {
    let all_file_commits = db
        .get_all_file_commits()
        .context("Failed to load commit history")?;

    if all_file_commits.is_empty() {
        let count = db.commit_count()?;
        if count == 0 {
            anyhow::bail!("No commit history found. Run `statik index --with-history` first.");
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

    // Build relative-path-to-FileId lookup
    let mut rel_path_to_id: std::collections::HashMap<String, crate::model::FileId> =
        std::collections::HashMap::new();
    for (_, info) in graph.all_files() {
        if let Ok(rel) = info.path.strip_prefix(project_root) {
            rel_path_to_id.insert(rel.to_string_lossy().to_string(), info.id);
        }
    }

    let now_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Track per-author data
    // Key: author_email
    struct AuthorData {
        name: String,
        sole_owned: Vec<(String, usize)>, // (path, fan_in)
        total_files: usize,
    }
    let mut authors: std::collections::HashMap<String, AuthorData> =
        std::collections::HashMap::new();

    for (path, commits) in &by_file {
        if let Some(ref matcher) = glob_matcher {
            if !matcher.is_match(path) {
                continue;
            }
        }

        let owners = compute_ownership_with_mode(commits, now_timestamp, half_life_days, mode);
        if owners.is_empty() {
            continue;
        }

        // Track total_files for all authors who touched this file
        for owner in &owners {
            let entry = authors
                .entry(owner.author_email.clone())
                .or_insert_with(|| AuthorData {
                    name: owner.author_name.clone(),
                    sole_owned: Vec::new(),
                    total_files: 0,
                });
            entry.total_files += 1;
        }

        // Check if primary owner is a sole owner (> 80% ownership)
        if owners[0].score > SOLE_OWNER_THRESHOLD {
            let fan_in = rel_path_to_id
                .get(path)
                .map(|id| graph.direct_importers(*id).len())
                .unwrap_or(0);

            let entry = authors
                .entry(owners[0].author_email.clone())
                .or_insert_with(|| AuthorData {
                    name: owners[0].author_name.clone(),
                    sole_owned: Vec::new(),
                    total_files: 0,
                });
            entry.sole_owned.push((path.clone(), fan_in));
        }
    }

    // Build result entries
    let mut result_authors: Vec<AuthorBusFactorEntry> = authors
        .into_iter()
        .filter(|(_, data)| !data.sole_owned.is_empty())
        .map(|(email, data)| {
            let sole_owned_files = data.sole_owned.len();
            let total_blast_radius: usize = data.sole_owned.iter().map(|(_, fi)| fi).sum();

            // Extract key areas: parent directory of each sole-owned file, top 5 by count
            let mut dir_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (path, _) in &data.sole_owned {
                let dir = std::path::Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !dir.is_empty() {
                    *dir_counts.entry(dir).or_default() += 1;
                }
            }
            let mut sorted_dirs: Vec<(String, usize)> = dir_counts.into_iter().collect();
            sorted_dirs.sort_by(|a, b| b.1.cmp(&a.1));
            let key_areas: Vec<String> = sorted_dirs
                .into_iter()
                .take(5)
                .map(|(dir, _)| dir)
                .collect();

            AuthorBusFactorEntry {
                author_name: data.name,
                author_email: email,
                sole_owned_files,
                total_files: data.total_files,
                total_blast_radius,
                key_areas,
            }
        })
        .collect();

    // Sort by sole_owned_files descending
    result_authors.sort_by(|a, b| b.sole_owned_files.cmp(&a.sole_owned_files));

    let count = result_authors.len();
    Ok(AuthorBusFactorResult {
        command: "bus-factor".to_string(),
        authors: result_authors,
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
        let commits = vec![make_commit(
            "sha1",
            "Alice",
            "alice@example.com",
            1700000000,
            10,
            5,
        )];
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
        assert!(
            alice.score > bob.score,
            "Recent commits should score higher"
        );
        assert!(
            alice.score > 70.0,
            "Alice should dominate with recent commit"
        );
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
            make_commit(
                "sha3",
                "Charlie",
                "charlie@example.com",
                now - 86400 * 30,
                8,
                2,
            ),
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
        db.insert_file_commit("src/main.rs", "sha1", 20, 5).unwrap();
        db.insert_file_commit("src/main.rs", "sha2", 10, 2).unwrap();

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

    /// Regression test for 10.4b: fan_in was always 0 on multi-module projects.
    ///
    /// The bug: FileGraph stores absolute paths (e.g. /home/user/project/module/src/Foo.java)
    /// but git log returns relative paths (e.g. module/src/Foo.java). The old suffix-matching
    /// approach failed on deeply nested multi-module paths. This test reproduces the exact
    /// scenario from the external evaluation: a utility class imported by many files across
    /// multiple Gradle modules, where fan_in was reported as 0 despite having 200+ importers.
    #[test]
    fn test_bus_factor_fan_in_multimodule_deep_paths() {
        use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
        use crate::model::{FileId, Language};
        use std::path::PathBuf;

        let db = Database::in_memory().unwrap();
        let now = 1700000000;

        // Simulate a multi-module Gradle project with a deeply nested project root
        let project_root = PathBuf::from("/home/user/workspace/mycompany-platform");

        // The core utility file (the one that had 200+ importers but fan_in=0)
        let core_util = "core/src/main/java/com/mycompany/platform/core/util/StringUtils.java";
        // Files across different modules that import the utility
        let api_controller =
            "api-gateway/src/main/java/com/mycompany/platform/api/UserController.java";
        let auth_service =
            "auth-service/src/main/java/com/mycompany/platform/auth/AuthService.java";
        let data_repo = "data-layer/src/main/java/com/mycompany/platform/data/UserRepository.java";
        let web_handler =
            "web-frontend/src/main/java/com/mycompany/platform/web/RequestHandler.java";

        // Commit history uses relative paths (as git log returns)
        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        for rel_path in &[
            core_util,
            api_controller,
            auth_service,
            data_repo,
            web_handler,
        ] {
            db.insert_file_commit(rel_path, "sha1", 50, 0).unwrap();
        }

        // File graph uses absolute paths
        let mut graph = FileGraph::new();
        let files = vec![
            (FileId(1), core_util),
            (FileId(2), api_controller),
            (FileId(3), auth_service),
            (FileId(4), data_repo),
            (FileId(5), web_handler),
        ];
        for (id, rel) in &files {
            graph.add_file(FileInfo {
                id: *id,
                path: project_root.join(rel),
                language: Language::Java,
                exports: vec![],
                is_entry_point: false,
                suppressions: std::collections::HashMap::new(),
                source_set: None,
            });
        }

        // All 4 module files import the core utility -> fan_in = 4
        for importer_id in [FileId(2), FileId(3), FileId(4), FileId(5)] {
            graph.add_import(FileImport {
                from: importer_id,
                to: FileId(1),
                imported_names: vec!["StringUtils".to_string()],
                is_type_only: false,
                is_mod_declaration: false,
                is_scip_derived: false,
                line: 1,
            });
        }

        let result = compute_bus_factor_analysis(
            &db,
            &graph,
            None,
            0.1,
            180.0,
            &project_root,
            HalfLifeMode::Fixed,
        )
        .unwrap();

        // The core utility should have fan_in = 4
        let util_entry = result
            .files
            .iter()
            .find(|e| e.path.contains("StringUtils.java"))
            .expect("StringUtils.java should be in results");
        assert_eq!(
            util_entry.fan_in, 4,
            "StringUtils.java should have fan_in=4 (imported by 4 modules), got {}",
            util_entry.fan_in
        );
        assert!(
            util_entry.risk_score > 0.0,
            "risk_score should be > 0 when fan_in > 0 and bus_factor = 1, got {}",
            util_entry.risk_score
        );

        // Files that import but are NOT imported should have fan_in = 0
        let controller_entry = result
            .files
            .iter()
            .find(|e| e.path.contains("UserController.java"))
            .expect("UserController.java should be in results");
        assert_eq!(controller_entry.fan_in, 0, "Leaf file should have fan_in=0");

        // Verify ALL files have results (none lost due to path matching)
        assert_eq!(
            result.files.len(),
            5,
            "All 5 files should appear in results"
        );
    }

    /// Regression test: fan_in path matching must handle project roots that are
    /// themselves deeply nested (e.g. /home/user/work/clients/acme/backend).
    /// The old suffix-matching would sometimes match the wrong file when paths
    /// shared common suffixes across different modules.
    #[test]
    fn test_bus_factor_fan_in_no_false_matches() {
        use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
        use crate::model::{FileId, Language};
        use std::path::PathBuf;

        let db = Database::in_memory().unwrap();
        let now = 1700000000;
        let project_root = PathBuf::from("/project");

        // Two files with the same filename in different modules
        let foo_core = "core/src/main/java/com/example/Config.java";
        let foo_web = "web/src/main/java/com/example/Config.java";

        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_file_commit(foo_core, "sha1", 80, 0).unwrap();
        db.insert_file_commit(foo_web, "sha1", 40, 0).unwrap();

        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: project_root.join(foo_core),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: project_root.join(foo_web),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });

        // Only web/Config is imported (by a hypothetical consumer)
        // but core/Config has no importers
        graph.add_file(FileInfo {
            id: FileId(3),
            path: project_root.join("web/src/main/java/com/example/App.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        db.insert_file_commit("web/src/main/java/com/example/App.java", "sha1", 20, 0)
            .unwrap();
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(2),
            imported_names: vec!["Config".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });

        let result = compute_bus_factor_analysis(
            &db,
            &graph,
            None,
            0.1,
            180.0,
            &project_root,
            HalfLifeMode::Fixed,
        )
        .unwrap();

        // core/Config should have fan_in = 0 (nobody imports it)
        let core_entry = result
            .files
            .iter()
            .find(|e| e.path.contains("core") && e.path.contains("Config.java"))
            .expect("core/Config.java should be in results");
        assert_eq!(
            core_entry.fan_in, 0,
            "core/Config should have fan_in=0, not inherit web/Config's fan_in"
        );

        // web/Config should have fan_in = 1
        let web_entry = result
            .files
            .iter()
            .find(|e| e.path.contains("web") && e.path.contains("Config.java"))
            .expect("web/Config.java should be in results");
        assert_eq!(web_entry.fan_in, 1, "web/Config should have fan_in=1");
    }

    #[test]
    fn test_adaptive_half_life_old_file_creator_retains_ownership() {
        let now = 1700000000;
        let eight_years_ago = now - 8 * 365 * 86400;
        let one_year_ago = now - 365 * 86400;

        // Alice created the file 8 years ago with substantial work
        // Bob made a trivial edit 1 year ago
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", eight_years_ago, 90, 0),
            make_commit("sha2", "Bob", "bob@example.com", one_year_ago, 3, 2),
        ];

        // With fixed half-life (180 days), Alice's contribution decays to near-zero
        let fixed_scores = compute_ownership(&commits, now, 180.0);
        let alice_fixed = fixed_scores
            .iter()
            .find(|s| s.author_name == "Alice")
            .unwrap();
        assert!(
            alice_fixed.score < 1.0,
            "With fixed 180d half-life, Alice should be < 1%, got {:.2}%",
            alice_fixed.score
        );

        // With adaptive half-life, Alice retains meaningful ownership
        let adaptive_scores = compute_ownership_adaptive(&commits, now, 180.0);
        let alice_adaptive = adaptive_scores
            .iter()
            .find(|s| s.author_name == "Alice")
            .unwrap();
        assert!(
            alice_adaptive.score > 10.0,
            "With adaptive half-life, Alice should retain > 10% ownership, got {:.2}%",
            alice_adaptive.score
        );
    }

    #[test]
    fn test_adaptive_young_file_same_as_fixed() {
        let now = 1700000000;
        let thirty_days_ago = now - 30 * 86400;

        // File younger than 4 * 180 days -> adaptive = fixed
        let commits = vec![
            make_commit("sha1", "Alice", "alice@example.com", thirty_days_ago, 50, 0),
            make_commit("sha2", "Bob", "bob@example.com", now, 10, 0),
        ];

        let fixed_scores = compute_ownership(&commits, now, 180.0);
        let adaptive_scores = compute_ownership_adaptive(&commits, now, 180.0);

        // File age = 30 days, adaptive half-life = max(180, 30*0.25) = 180
        // So they should be identical
        assert_eq!(fixed_scores.len(), adaptive_scores.len());
        for (f, a) in fixed_scores.iter().zip(adaptive_scores.iter()) {
            assert!(
                (f.score - a.score).abs() < 0.01,
                "Young file: fixed ({:.2}%) and adaptive ({:.2}%) should match",
                f.score,
                a.score
            );
        }
    }

    #[test]
    fn test_adaptive_vs_fixed_difference() {
        let now = 1700000000;
        let four_years_ago = now - 4 * 365 * 86400;
        let six_months_ago = now - 180 * 86400;

        // File created 4 years ago, recent edit
        let commits = vec![
            make_commit(
                "sha1",
                "Creator",
                "creator@example.com",
                four_years_ago,
                80,
                0,
            ),
            make_commit(
                "sha2",
                "Tweaker",
                "tweaker@example.com",
                six_months_ago,
                5,
                0,
            ),
        ];

        let fixed_scores = compute_ownership(&commits, now, 180.0);
        let adaptive_scores = compute_ownership_adaptive(&commits, now, 180.0);

        let creator_fixed = fixed_scores
            .iter()
            .find(|s| s.author_name == "Creator")
            .unwrap();
        let creator_adaptive = adaptive_scores
            .iter()
            .find(|s| s.author_name == "Creator")
            .unwrap();

        // Adaptive should give creator significantly more than fixed
        assert!(
            creator_adaptive.score > creator_fixed.score,
            "Adaptive should give creator more ownership: adaptive={:.2}% vs fixed={:.2}%",
            creator_adaptive.score,
            creator_fixed.score
        );
    }

    /// Regression test for 10.7: verify that the full bus-factor pipeline actually
    /// uses adaptive mode when passed, producing different primary_owner scores
    /// than fixed mode on an old file.
    #[test]
    fn test_bus_factor_analysis_uses_adaptive_mode() {
        use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
        use crate::model::{FileId, Language};
        use std::path::PathBuf;

        let db = Database::in_memory().unwrap();
        let now = 1700000000;
        let eight_years_ago = now - 8 * 365 * 86400;
        let one_year_ago = now - 365 * 86400;
        let project_root = PathBuf::from("/project");

        // Creator made the file 8 years ago, tweaker edited recently
        db.insert_commit("sha1", "Creator", "creator@example.com", eight_years_ago)
            .unwrap();
        db.insert_commit("sha2", "Tweaker", "tweaker@example.com", one_year_ago)
            .unwrap();
        db.insert_file_commit("src/old_util.rs", "sha1", 90, 0)
            .unwrap();
        db.insert_file_commit("src/old_util.rs", "sha2", 3, 2)
            .unwrap();

        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: project_root.join("src/old_util.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: project_root.join("src/consumer.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        db.insert_file_commit("src/consumer.rs", "sha2", 20, 0)
            .unwrap();
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(1),
            imported_names: vec!["old_util".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });

        let fixed_result = compute_bus_factor_analysis(
            &db,
            &graph,
            None,
            0.1,
            180.0,
            &project_root,
            HalfLifeMode::Fixed,
        )
        .unwrap();
        let adaptive_result = compute_bus_factor_analysis(
            &db,
            &graph,
            None,
            0.1,
            180.0,
            &project_root,
            HalfLifeMode::Adaptive,
        )
        .unwrap();

        let fixed_entry = fixed_result
            .files
            .iter()
            .find(|e| e.path.contains("old_util"))
            .unwrap();
        let adaptive_entry = adaptive_result
            .files
            .iter()
            .find(|e| e.path.contains("old_util"))
            .unwrap();

        // With fixed mode on an 8-year-old file, Tweaker should be primary owner
        assert_eq!(
            fixed_entry.primary_owner.author_name, "Tweaker",
            "Fixed mode: Tweaker should be primary owner of old file"
        );

        // With adaptive mode, Creator should retain enough ownership to be primary
        assert_eq!(
            adaptive_entry.primary_owner.author_name, "Creator",
            "Adaptive mode: Creator should be primary owner of old file, got {}",
            adaptive_entry.primary_owner.author_name,
        );
    }

    /// Regression test for 10.7: verify that compute_owners also respects the mode.
    #[test]
    fn test_compute_owners_uses_adaptive_mode() {
        let db = Database::in_memory().unwrap();
        let now = 1700000000;
        let five_years_ago = now - 5 * 365 * 86400;
        let three_months_ago = now - 90 * 86400;

        db.insert_commit("sha1", "Original", "original@example.com", five_years_ago)
            .unwrap();
        db.insert_commit("sha2", "Recent", "recent@example.com", three_months_ago)
            .unwrap();
        db.insert_file_commit("src/old_module.rs", "sha1", 100, 0)
            .unwrap();
        db.insert_file_commit("src/old_module.rs", "sha2", 5, 0)
            .unwrap();

        let fixed_result = compute_owners(&db, Some("**"), 10, 180.0, HalfLifeMode::Fixed).unwrap();
        let adaptive_result =
            compute_owners(&db, Some("**"), 10, 180.0, HalfLifeMode::Adaptive).unwrap();

        let fixed_file = &fixed_result.files[0];
        let adaptive_file = &adaptive_result.files[0];

        let orig_fixed = fixed_file
            .owners
            .iter()
            .find(|o| o.author_name == "Original")
            .unwrap();
        let orig_adaptive = adaptive_file
            .owners
            .iter()
            .find(|o| o.author_name == "Original")
            .unwrap();

        assert!(
            orig_adaptive.score > orig_fixed.score,
            "Adaptive mode should give Original more ownership than fixed: adaptive={:.2}% vs fixed={:.2}%",
            orig_adaptive.score,
            orig_fixed.score
        );
    }

    #[test]
    fn test_bus_factor_by_author() {
        use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
        use crate::model::{FileId, Language};
        use std::path::PathBuf;

        let db = Database::in_memory().unwrap();
        let now = 1700000000;
        let project_root = PathBuf::from("/project");

        // Alice is sole owner of 2 files, Bob is sole owner of 1 file
        // Charlie shared ownership with Alice on one file
        db.insert_commit("sha1", "Alice", "alice@example.com", now)
            .unwrap();
        db.insert_commit("sha2", "Bob", "bob@example.com", now)
            .unwrap();
        db.insert_commit("sha3", "Charlie", "charlie@example.com", now)
            .unwrap();

        // File 1: Alice sole owner (100% ownership)
        db.insert_file_commit("src/core/auth.rs", "sha1", 100, 0)
            .unwrap();
        // File 2: Alice sole owner (100% ownership)
        db.insert_file_commit("src/core/db.rs", "sha1", 80, 0)
            .unwrap();
        // File 3: Bob sole owner (100% ownership)
        db.insert_file_commit("src/api/handler.rs", "sha2", 60, 0)
            .unwrap();
        // File 4: Alice + Charlie shared (equal ownership)
        db.insert_file_commit("src/models/user.rs", "sha1", 50, 0)
            .unwrap();
        db.insert_file_commit("src/models/user.rs", "sha3", 50, 0)
            .unwrap();

        // Build file graph with import edges
        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("/project/src/core/auth.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("/project/src/core/db.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(3),
            path: PathBuf::from("/project/src/api/handler.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });
        graph.add_file(FileInfo {
            id: FileId(4),
            path: PathBuf::from("/project/src/models/user.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        });

        // handler imports auth (auth has fan_in = 1)
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(1),
            imported_names: vec!["auth".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 1,
        });
        // handler imports db (db has fan_in = 1)
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(2),
            imported_names: vec!["db".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            is_scip_derived: false,
            line: 2,
        });

        let result = compute_bus_factor_by_author(
            &db,
            &graph,
            &project_root,
            None,
            180.0,
            HalfLifeMode::Fixed,
        )
        .unwrap();

        // Alice should have 2 sole-owned files, Bob should have 1
        let alice = result
            .authors
            .iter()
            .find(|a| a.author_email == "alice@example.com")
            .expect("Alice should be in results");
        assert_eq!(
            alice.sole_owned_files, 2,
            "Alice should solely own 2 files, got {}",
            alice.sole_owned_files
        );
        assert_eq!(
            alice.total_blast_radius, 2,
            "Alice's blast radius should be 2 (auth fan_in=1 + db fan_in=1), got {}",
            alice.total_blast_radius
        );
        assert!(
            alice.key_areas.contains(&"src/core".to_string()),
            "Alice's key areas should include src/core, got {:?}",
            alice.key_areas
        );

        // Alice touched 3 files total: auth.rs, db.rs, and user.rs (shared)
        assert_eq!(
            alice.total_files, 3,
            "Alice should have touched 3 files total, got {}",
            alice.total_files
        );

        let bob = result
            .authors
            .iter()
            .find(|a| a.author_email == "bob@example.com")
            .expect("Bob should be in results");
        assert_eq!(bob.sole_owned_files, 1);
        assert_eq!(
            bob.total_files, 1,
            "Bob should have touched 1 file total, got {}",
            bob.total_files
        );
        assert_eq!(
            bob.total_blast_radius, 0,
            "Bob's handler.rs has no importers, blast radius should be 0, got {}",
            bob.total_blast_radius
        );
        assert!(
            bob.key_areas.contains(&"src/api".to_string()),
            "Bob's key areas should include src/api, got {:?}",
            bob.key_areas
        );

        // Charlie should NOT appear (no sole-owned files, shared user.rs with Alice)
        let charlie = result
            .authors
            .iter()
            .find(|a| a.author_email == "charlie@example.com");
        assert!(
            charlie.is_none(),
            "Charlie should not appear (no sole-owned files)"
        );

        // Result should be sorted by sole_owned_files descending
        assert_eq!(result.authors[0].author_email, "alice@example.com");
        assert_eq!(result.authors[1].author_email, "bob@example.com");

        // Verify count
        assert_eq!(
            result.count, 2,
            "Only Alice and Bob should appear (not Charlie)"
        );
    }
}
