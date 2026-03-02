use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::model::file_graph::FileGraph;

use super::ownership::{compute_owners, HalfLifeMode};

/// Team membership for a single author.
#[derive(Debug, Clone)]
pub struct TeamMember {
    pub email: String,
    pub team: String,
}

/// Team config: team name -> list of email glob patterns.
pub type TeamConfig = HashMap<String, Vec<String>>;

/// Resolve which team an email belongs to.
///
/// Tries explicit patterns first (supports `*` wildcard prefix matching).
/// Falls back to email domain when no config exists.
pub fn resolve_team(email: &str, team_config: &TeamConfig) -> String {
    for (team_name, patterns) in team_config {
        for pattern in patterns {
            if email_matches_pattern(email, pattern) {
                return team_name.clone();
            }
        }
    }
    // Fallback: use email domain as team name
    email_domain(email)
}

/// Check if an email matches a glob-like pattern.
/// Supports `*@domain.com` (prefix wildcard) and exact matches.
fn email_matches_pattern(email: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        email.ends_with(suffix)
    } else {
        email == pattern
    }
}

/// Extract domain from email address.
fn email_domain(email: &str) -> String {
    match email.rsplit_once('@') {
        Some((_, domain)) => domain.to_string(),
        None => "unknown".to_string(),
    }
}

/// Ownership for a single team on a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamOwnership {
    pub team: String,
    /// Aggregate ownership percentage (0.0 to 100.0).
    pub ownership_pct: f64,
}

/// Team coupling result for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTeamCoupling {
    pub path: String,
    /// Teams with ownership and their aggregate ownership percentage.
    pub teams: Vec<TeamOwnership>,
    /// Number of distinct teams with significant ownership (>10%).
    pub team_count: usize,
    /// True if 2+ teams have significant ownership each.
    pub is_cross_team: bool,
}

/// A dependency edge that crosses team boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossTeamEdge {
    pub from_file: String,
    pub to_file: String,
    pub from_team: String,
    pub to_team: String,
}

/// Summary of team coupling analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCouplingSummary {
    pub files_analyzed: usize,
    pub cross_team_files: usize,
    pub cross_team_edges: usize,
    pub teams_found: usize,
}

/// Result of the team-coupling command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCouplingResult {
    pub command: String,
    pub files: Vec<FileTeamCoupling>,
    pub cross_team_edges: Vec<CrossTeamEdge>,
    pub summary: TeamCouplingSummary,
}

/// Parameters for team coupling analysis.
pub struct TeamCouplingParams<'a> {
    pub glob_pattern: Option<&'a str>,
    pub half_life_days: f64,
    pub half_life_mode: HalfLifeMode,
    pub team_config: &'a TeamConfig,
    pub ownership_threshold: f64,
}

/// Compute team coupling analysis.
///
/// For each file, compute ownership, resolve owners to teams, and report
/// which files require cross-team coordination. Also identify dependency
/// edges that cross team boundaries.
pub fn compute_team_coupling(
    db: &Database,
    graph: &FileGraph,
    project_root: &std::path::Path,
    params: &TeamCouplingParams<'_>,
) -> Result<TeamCouplingResult> {
    // Compute full ownership (top 10 owners to capture all significant contributors)
    let owners_result = compute_owners(
        db,
        params.glob_pattern,
        10,
        params.half_life_days,
        params.half_life_mode,
    )?;

    // Determine whether to use config or infer from domains
    let use_domain_inference = params.team_config.is_empty();

    let mut files = Vec::new();

    for file_ownership in &owners_result.files {
        // Aggregate ownership by team
        let mut team_scores: HashMap<String, f64> = HashMap::new();
        for owner in &file_ownership.owners {
            let team = if use_domain_inference {
                email_domain(&owner.author_email)
            } else {
                resolve_team(&owner.author_email, params.team_config)
            };
            *team_scores.entry(team).or_default() += owner.score;
        }

        // Sort teams by ownership descending
        let mut teams: Vec<TeamOwnership> = team_scores
            .into_iter()
            .map(|(team, pct)| TeamOwnership {
                team,
                ownership_pct: pct,
            })
            .collect();
        teams.sort_by(|a, b| {
            b.ownership_pct
                .partial_cmp(&a.ownership_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Count teams with significant ownership
        let significant_teams = teams
            .iter()
            .filter(|t| t.ownership_pct >= params.ownership_threshold * 100.0)
            .count();
        let is_cross_team = significant_teams >= 2;

        files.push(FileTeamCoupling {
            path: file_ownership.path.clone(),
            teams,
            team_count: significant_teams,
            is_cross_team,
        });
    }

    // Find cross-team dependency edges
    let cross_team_edges = find_cross_team_edges(graph, project_root, &files);

    // Count unique teams
    let mut all_teams: std::collections::HashSet<String> = std::collections::HashSet::new();
    for file in &files {
        for team in &file.teams {
            all_teams.insert(team.team.clone());
        }
    }

    let cross_team_file_count = files.iter().filter(|f| f.is_cross_team).count();

    let summary = TeamCouplingSummary {
        files_analyzed: files.len(),
        cross_team_files: cross_team_file_count,
        cross_team_edges: cross_team_edges.len(),
        teams_found: all_teams.len(),
    };

    Ok(TeamCouplingResult {
        command: "team-coupling".to_string(),
        files,
        cross_team_edges,
        summary,
    })
}

/// Find dependency edges that cross team boundaries.
///
/// For each edge in the file graph, determine the primary team of the source
/// and target files. If they differ, it's a cross-team edge.
fn find_cross_team_edges(
    graph: &FileGraph,
    project_root: &std::path::Path,
    file_couplings: &[FileTeamCoupling],
) -> Vec<CrossTeamEdge> {
    // Build a lookup from file path to primary team
    let mut path_to_team: HashMap<String, String> = HashMap::new();
    for fc in file_couplings {
        if let Some(primary) = fc.teams.first() {
            path_to_team.insert(fc.path.clone(), primary.team.clone());
        }
    }

    let mut edges = Vec::new();
    for file_edges in graph.imports.values() {
        for edge in file_edges {
            let from_info = match graph.files.get(&edge.from) {
                Some(f) => f,
                None => continue,
            };
            let to_info = match graph.files.get(&edge.to) {
                Some(f) => f,
                None => continue,
            };

            let from_rel = from_info
                .path
                .strip_prefix(project_root)
                .unwrap_or(&from_info.path)
                .display()
                .to_string();
            let to_rel = to_info
                .path
                .strip_prefix(project_root)
                .unwrap_or(&to_info.path)
                .display()
                .to_string();

            let from_team = path_to_team.get(&from_rel).cloned().unwrap_or_default();
            let to_team = path_to_team.get(&to_rel).cloned().unwrap_or_default();

            if !from_team.is_empty() && !to_team.is_empty() && from_team != to_team {
                edges.push(CrossTeamEdge {
                    from_file: from_rel,
                    to_file: to_rel,
                    from_team,
                    to_team,
                });
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_team_with_config() {
        let mut config = TeamConfig::new();
        config.insert(
            "platform".to_string(),
            vec!["*@platform.example.com".to_string()],
        );
        config.insert(
            "product".to_string(),
            vec![
                "*@product.example.com".to_string(),
                "alice@special.com".to_string(),
            ],
        );

        assert_eq!(
            resolve_team("bob@platform.example.com", &config),
            "platform"
        );
        assert_eq!(
            resolve_team("alice@product.example.com", &config),
            "product"
        );
        assert_eq!(resolve_team("alice@special.com", &config), "product");
    }

    #[test]
    fn test_resolve_team_fallback_to_domain() {
        let config = TeamConfig::new();
        assert_eq!(resolve_team("alice@example.com", &config), "example.com");
    }

    #[test]
    fn test_email_domain() {
        assert_eq!(email_domain("alice@example.com"), "example.com");
        assert_eq!(email_domain("bob@sub.domain.org"), "sub.domain.org");
        assert_eq!(email_domain("nodomain"), "unknown");
    }

    #[test]
    fn test_email_matches_pattern() {
        assert!(email_matches_pattern("alice@example.com", "*@example.com"));
        assert!(email_matches_pattern(
            "alice@example.com",
            "alice@example.com"
        ));
        assert!(!email_matches_pattern("alice@example.com", "*@other.com"));
        assert!(!email_matches_pattern(
            "alice@example.com",
            "bob@example.com"
        ));
    }
}
