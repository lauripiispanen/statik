use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::model::file_graph::FileGraph;
use crate::model::FileId;

use super::impact::analyze_impact;
use super::ownership::{HalfLifeMode, OwnershipScore};

/// Ownership info for a person across multiple files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerScore {
    pub author_name: String,
    pub author_email: String,
    /// Total ownership weight across all affected files (sum of per-file scores, NOT re-normalized to 100).
    pub total_weight: f64,
    /// Number of affected files this person owns (has any ownership score on).
    pub files_owned: usize,
}

/// A single file in the blast radius, with its owners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedFileOwnership {
    pub path: String,
    pub depth: usize,
    pub owners: Vec<OwnershipScore>,
}

/// Result of the `who` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoResult {
    pub command: String,
    pub target_file: String,
    pub direct_owners: Vec<OwnershipScore>,
    pub downstream_owners: Vec<ReviewerScore>,
    pub suggested_reviewers: Vec<ReviewerScore>,
    pub affected_files: Vec<AffectedFileOwnership>,
    pub summary: WhoSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoSummary {
    pub total_affected_files: usize,
    pub unique_downstream_owners: usize,
    pub suggested_reviewer_count: usize,
}

/// Compute ownership for a single file path, respecting the half-life mode.
fn compute_ownership_for_file(
    db: &Database,
    rel_path: &str,
    half_life_days: f64,
    now_timestamp: i64,
    mode: HalfLifeMode,
) -> Result<Vec<OwnershipScore>> {
    let commits = db.get_commits_for_file(rel_path)?;
    let owners = match mode {
        HalfLifeMode::Fixed => {
            super::ownership::compute_ownership(&commits, now_timestamp, half_life_days)
        }
        HalfLifeMode::Adaptive => {
            super::ownership::compute_ownership_adaptive(&commits, now_timestamp, half_life_days)
        }
    };
    Ok(owners)
}

/// Compute the greedy set-cover of suggested reviewers.
///
/// Picks the person covering the most uncovered files, repeats until all
/// affected files with owners are covered. A person "covers" a file if they
/// have any non-zero ownership score on it.
fn greedy_set_cover(
    affected_files: &[AffectedFileOwnership],
) -> Vec<ReviewerScore> {
    // Build a mapping: author_email -> set of file indices they cover
    let mut author_files: HashMap<String, (String, HashSet<usize>)> = HashMap::new();
    for (i, file) in affected_files.iter().enumerate() {
        for owner in &file.owners {
            if owner.score > 0.0 {
                let entry = author_files
                    .entry(owner.author_email.clone())
                    .or_insert_with(|| (owner.author_name.clone(), HashSet::new()));
                entry.1.insert(i);
            }
        }
    }

    // Files that have at least one owner
    let files_with_owners: HashSet<usize> = author_files
        .values()
        .flat_map(|(_name, files)| files.iter().copied())
        .collect();

    let mut uncovered = files_with_owners;
    let mut result = Vec::new();

    while !uncovered.is_empty() {
        // Find the author covering the most uncovered files
        let best = author_files
            .iter()
            .max_by_key(|(_email, (_name, files))| {
                files.intersection(&uncovered).count()
            });

        match best {
            Some((email, (name, files))) => {
                let covered_count = files.intersection(&uncovered).count();
                if covered_count == 0 {
                    break;
                }
                result.push(ReviewerScore {
                    author_name: name.clone(),
                    author_email: email.clone(),
                    total_weight: 0.0, // will be filled in later
                    files_owned: files.len(),
                });
                // Remove covered files
                for f in files {
                    uncovered.remove(f);
                }
                // Remove this author from consideration
                let email_clone = email.clone();
                author_files.remove(&email_clone);
            }
            None => break,
        }
    }

    result
}

/// Analyze who should review a change to the given file.
///
/// Combines blast radius analysis (impact) with ownership data (git history)
/// to suggest a minimal reviewer set.
#[allow(clippy::too_many_arguments)]
pub fn analyze_who(
    db: &Database,
    graph: &FileGraph,
    target: FileId,
    max_depth: Option<usize>,
    half_life_days: f64,
    project_root: &Path,
    half_life_mode: HalfLifeMode,
    top_per_file: usize,
) -> Result<WhoResult> {
    let now_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Get the target file's relative path
    let target_info = graph
        .files
        .get(&target)
        .ok_or_else(|| anyhow::anyhow!("File not found in graph"))?;
    let target_abs = &target_info.path;
    let target_rel = target_abs
        .strip_prefix(project_root)
        .unwrap_or(target_abs)
        .to_string_lossy()
        .to_string();

    // 1. Direct owners of the target file
    let direct_owners = compute_ownership_for_file(
        db,
        &target_rel,
        half_life_days,
        now_timestamp,
        half_life_mode,
    )?;

    // 2. Blast radius: get affected files
    let impact = analyze_impact(graph, target, max_depth);
    let affected = match &impact {
        Some(result) => &result.affected,
        None => {
            // Target file not in graph or no dependents
            return Ok(WhoResult {
                command: "who".to_string(),
                target_file: target_rel,
                direct_owners,
                downstream_owners: Vec::new(),
                suggested_reviewers: Vec::new(),
                affected_files: Vec::new(),
                summary: WhoSummary {
                    total_affected_files: 0,
                    unique_downstream_owners: 0,
                    suggested_reviewer_count: 0,
                },
            });
        }
    };

    // 3. For each affected file, compute ownership
    let mut affected_files = Vec::new();
    for af in affected {
        let rel_path = af
            .path
            .strip_prefix(project_root)
            .unwrap_or(&af.path)
            .to_string_lossy()
            .to_string();
        let mut owners = compute_ownership_for_file(
            db,
            &rel_path,
            half_life_days,
            now_timestamp,
            half_life_mode,
        )?;
        owners.truncate(top_per_file);
        affected_files.push(AffectedFileOwnership {
            path: rel_path,
            depth: af.depth,
            owners,
        });
    }

    // 4. Aggregate downstream owners: sum raw scores per author across all affected files
    let mut downstream_map: HashMap<String, (String, f64, usize)> = HashMap::new();
    for file in &affected_files {
        for owner in &file.owners {
            let entry = downstream_map
                .entry(owner.author_email.clone())
                .or_insert_with(|| (owner.author_name.clone(), 0.0, 0));
            entry.1 += owner.score;
            entry.2 += 1;
        }
    }

    let mut downstream_owners: Vec<ReviewerScore> = downstream_map
        .into_iter()
        .map(|(email, (name, weight, files))| ReviewerScore {
            author_name: name,
            author_email: email,
            total_weight: weight,
            files_owned: files,
        })
        .collect();
    downstream_owners.sort_by(|a, b| {
        b.total_weight
            .partial_cmp(&a.total_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let unique_downstream_owners = downstream_owners.len();

    // 5. Greedy set-cover for suggested reviewers
    let mut suggested_reviewers = greedy_set_cover(&affected_files);
    // Fill in total_weight from downstream data
    let weight_lookup: HashMap<&str, f64> = downstream_owners
        .iter()
        .map(|r| (r.author_email.as_str(), r.total_weight))
        .collect();
    for reviewer in &mut suggested_reviewers {
        reviewer.total_weight = weight_lookup
            .get(reviewer.author_email.as_str())
            .copied()
            .unwrap_or(0.0);
    }

    let total_affected_files = affected_files.len();
    let suggested_reviewer_count = suggested_reviewers.len();

    Ok(WhoResult {
        command: "who".to_string(),
        target_file: target_rel,
        direct_owners,
        downstream_owners,
        suggested_reviewers,
        affected_files,
        summary: WhoSummary {
            total_affected_files,
            unique_downstream_owners,
            suggested_reviewer_count,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::file_graph::{FileImport, FileInfo};
    use crate::model::Language;
    use std::path::PathBuf;

    fn make_file(id: u64, path: &str) -> FileInfo {
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(path),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        }
    }

    fn make_edge(from: u64, to: u64) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: vec!["x".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        }
    }

    fn make_ownership(name: &str, email: &str, score: f64) -> OwnershipScore {
        OwnershipScore {
            author_name: name.to_string(),
            author_email: email.to_string(),
            score,
        }
    }

    // --- Set-cover algorithm tests ---

    #[test]
    fn test_set_cover_single_person_covers_all() {
        let affected = vec![
            AffectedFileOwnership {
                path: "a.ts".to_string(),
                depth: 1,
                owners: vec![make_ownership("Alice", "alice@x.com", 80.0)],
            },
            AffectedFileOwnership {
                path: "b.ts".to_string(),
                depth: 1,
                owners: vec![make_ownership("Alice", "alice@x.com", 60.0)],
            },
        ];
        let result = greedy_set_cover(&affected);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].author_email, "alice@x.com");
    }

    #[test]
    fn test_set_cover_two_people_needed() {
        // Person A owns files 0,1; Person B owns file 2
        let affected = vec![
            AffectedFileOwnership {
                path: "a.ts".to_string(),
                depth: 1,
                owners: vec![make_ownership("Alice", "alice@x.com", 80.0)],
            },
            AffectedFileOwnership {
                path: "b.ts".to_string(),
                depth: 1,
                owners: vec![make_ownership("Alice", "alice@x.com", 60.0)],
            },
            AffectedFileOwnership {
                path: "c.ts".to_string(),
                depth: 2,
                owners: vec![make_ownership("Bob", "bob@x.com", 90.0)],
            },
        ];
        let result = greedy_set_cover(&affected);
        assert_eq!(result.len(), 2);
        // Alice should be first (covers 2 files), Bob second (covers 1)
        assert_eq!(result[0].author_email, "alice@x.com");
        assert_eq!(result[1].author_email, "bob@x.com");
    }

    #[test]
    fn test_set_cover_empty_affected() {
        let affected: Vec<AffectedFileOwnership> = vec![];
        let result = greedy_set_cover(&affected);
        assert!(result.is_empty());
    }

    #[test]
    fn test_set_cover_no_owners() {
        let affected = vec![AffectedFileOwnership {
            path: "orphan.ts".to_string(),
            depth: 1,
            owners: vec![],
        }];
        let result = greedy_set_cover(&affected);
        assert!(result.is_empty());
    }

    #[test]
    fn test_set_cover_overlapping_ownership() {
        // Alice owns files 0,1,2. Bob owns files 1,2,3. Charlie owns file 3.
        // Alice covers 3 files, Bob covers 3, but after picking Alice (0,1,2),
        // only file 3 remains. Bob covers it.
        let affected = vec![
            AffectedFileOwnership {
                path: "a.ts".to_string(),
                depth: 1,
                owners: vec![make_ownership("Alice", "alice@x.com", 50.0)],
            },
            AffectedFileOwnership {
                path: "b.ts".to_string(),
                depth: 1,
                owners: vec![
                    make_ownership("Alice", "alice@x.com", 40.0),
                    make_ownership("Bob", "bob@x.com", 60.0),
                ],
            },
            AffectedFileOwnership {
                path: "c.ts".to_string(),
                depth: 2,
                owners: vec![
                    make_ownership("Alice", "alice@x.com", 30.0),
                    make_ownership("Bob", "bob@x.com", 70.0),
                ],
            },
            AffectedFileOwnership {
                path: "d.ts".to_string(),
                depth: 2,
                owners: vec![
                    make_ownership("Bob", "bob@x.com", 50.0),
                    make_ownership("Charlie", "charlie@x.com", 50.0),
                ],
            },
        ];
        let result = greedy_set_cover(&affected);
        // Both Alice and Bob cover 3 files. Depending on HashMap ordering,
        // either could be picked first. After one is picked, the other covers
        // the remaining file(s).
        assert!(result.len() <= 3);
        // All files with owners should be covered
        let all_emails: HashSet<&str> = result.iter().map(|r| r.author_email.as_str()).collect();
        // We need at least 2 people (Alice/Bob cover 0-2, someone covers 3)
        assert!(result.len() >= 2);
        // File 3 requires Bob or Charlie
        assert!(all_emails.contains("bob@x.com") || all_emails.contains("charlie@x.com"));
    }

    // --- Impact integration tests (using in-memory graph, no DB) ---

    #[test]
    fn test_impact_no_dependents_returns_empty_affected() {
        // When a file has no dependents, affected_files should be empty
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "leaf.ts"));

        let impact = analyze_impact(&graph, FileId(1), None);
        assert!(impact.is_some());
        assert!(impact.unwrap().affected.is_empty());
    }

    #[test]
    fn test_impact_with_dependents() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "core.ts"));
        graph.add_file(make_file(2, "a.ts"));
        graph.add_file(make_file(3, "b.ts"));
        graph.add_import(make_edge(2, 1));
        graph.add_import(make_edge(3, 1));

        let impact = analyze_impact(&graph, FileId(1), None).unwrap();
        assert_eq!(impact.affected.len(), 2);
    }

    // --- WhoResult construction tests ---

    #[test]
    fn test_reviewer_score_serialization() {
        let score = ReviewerScore {
            author_name: "Alice".to_string(),
            author_email: "alice@x.com".to_string(),
            total_weight: 150.0,
            files_owned: 3,
        };
        let json = serde_json::to_string(&score).unwrap();
        assert!(json.contains("\"author_name\":\"Alice\""));
        assert!(json.contains("\"total_weight\":150.0"));
    }

    #[test]
    fn test_who_result_serialization() {
        let result = WhoResult {
            command: "who".to_string(),
            target_file: "src/core.ts".to_string(),
            direct_owners: vec![make_ownership("Alice", "alice@x.com", 80.0)],
            downstream_owners: vec![ReviewerScore {
                author_name: "Bob".to_string(),
                author_email: "bob@x.com".to_string(),
                total_weight: 120.0,
                files_owned: 2,
            }],
            suggested_reviewers: vec![ReviewerScore {
                author_name: "Alice".to_string(),
                author_email: "alice@x.com".to_string(),
                total_weight: 80.0,
                files_owned: 1,
            }],
            affected_files: vec![],
            summary: WhoSummary {
                total_affected_files: 2,
                unique_downstream_owners: 1,
                suggested_reviewer_count: 1,
            },
        };
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"command\": \"who\""));
        assert!(json.contains("\"target_file\": \"src/core.ts\""));
    }

    #[test]
    fn test_who_summary_fields() {
        let summary = WhoSummary {
            total_affected_files: 5,
            unique_downstream_owners: 3,
            suggested_reviewer_count: 2,
        };
        assert_eq!(summary.total_affected_files, 5);
        assert_eq!(summary.unique_downstream_owners, 3);
        assert_eq!(summary.suggested_reviewer_count, 2);
    }
}
