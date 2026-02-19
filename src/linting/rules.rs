use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::analysis::Confidence;
use crate::model::file_graph::FileGraph;

use super::config::{LintConfig, RuleKind, Severity};
use super::matcher::{to_relative, FileMatcher};

/// A single lint violation found during rule evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintViolation {
    pub rule_id: String,
    pub severity: Severity,
    pub description: String,
    pub rationale: Option<String>,
    pub source_file: PathBuf,
    pub target_file: PathBuf,
    pub imported_names: Vec<String>,
    pub line: usize,
    pub confidence: Confidence,
    pub fix_direction: Option<String>,
}

/// Summary of lint results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintSummary {
    pub total_violations: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub rules_evaluated: usize,
}

/// Result of running lint rules against a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    pub violations: Vec<LintViolation>,
    pub rules_evaluated: usize,
    pub summary: LintSummary,
}

/// Evaluate all lint rules in a config against a FileGraph.
pub fn evaluate_rules(
    config: &LintConfig,
    graph: &FileGraph,
    project_root: &Path,
) -> Result<LintResult> {
    let mut all_violations = Vec::new();

    for rule_def in &config.rules {
        match &rule_def.rule {
            RuleKind::Boundary(boundary) => {
                let from_matcher = FileMatcher::new(&boundary.from)?;
                let deny_matcher = FileMatcher::new(&boundary.deny)?;
                let except_matcher = match &boundary.except {
                    Some(patterns) if !patterns.is_empty() => Some(FileMatcher::new(patterns)?),
                    _ => None,
                };

                // Iterate all import edges
                for (source_id, edges) in graph.imports.iter() {
                    let source_info = match graph.files.get(source_id) {
                        Some(info) => info,
                        None => continue,
                    };
                    let source_rel = to_relative(&source_info.path, project_root);

                    if !from_matcher.matches(source_rel) {
                        continue;
                    }

                    for edge in edges {
                        let target_info = match graph.files.get(&edge.to) {
                            Some(info) => info,
                            None => continue,
                        };
                        let target_rel = to_relative(&target_info.path, project_root);

                        if !deny_matcher.matches(target_rel) {
                            continue;
                        }

                        // Check exceptions
                        if let Some(ref except) = except_matcher {
                            if except.matches(target_rel) {
                                continue;
                            }
                        }

                        all_violations.push(LintViolation {
                            rule_id: rule_def.id.clone(),
                            severity: rule_def.severity,
                            description: rule_def.description.clone(),
                            rationale: rule_def.rationale.clone(),
                            source_file: source_rel.to_path_buf(),
                            target_file: target_rel.to_path_buf(),
                            imported_names: edge.imported_names.clone(),
                            line: edge.line,
                            confidence: Confidence::Certain,
                            fix_direction: rule_def.fix_direction.clone(),
                        });
                    }
                }
            }
        }
    }

    // Sort violations by severity (errors first), then by file path
    all_violations.sort_by(|a, b| {
        severity_order(a.severity)
            .cmp(&severity_order(b.severity))
            .then(a.source_file.cmp(&b.source_file))
            .then(a.line.cmp(&b.line))
    });

    let errors = all_violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    let warnings = all_violations
        .iter()
        .filter(|v| v.severity == Severity::Warning)
        .count();
    let infos = all_violations
        .iter()
        .filter(|v| v.severity == Severity::Info)
        .count();

    Ok(LintResult {
        summary: LintSummary {
            total_violations: all_violations.len(),
            errors,
            warnings,
            infos,
            rules_evaluated: config.rules.len(),
        },
        violations: all_violations,
        rules_evaluated: config.rules.len(),
    })
}

fn severity_order(s: Severity) -> u8 {
    match s {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linting::config::{BoundaryRuleConfig, RuleDefinition};
    use crate::model::file_graph::{FileImport, FileInfo};
    use crate::model::{FileId, Language};

    fn make_file(id: u64, path: &str) -> FileInfo {
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(format!("/project/{}", path)),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        }
    }

    fn make_edge(from: u64, to: u64, names: &[&str], line: usize) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: names.iter().map(|s| s.to_string()).collect(),
            is_type_only: false,
            line,
        }
    }

    fn make_boundary_rule(
        id: &str,
        severity: Severity,
        from: &[&str],
        deny: &[&str],
    ) -> RuleDefinition {
        RuleDefinition {
            id: id.to_string(),
            severity,
            description: format!("Rule: {}", id),
            rationale: None,
            fix_direction: None,
            rule: RuleKind::Boundary(BoundaryRuleConfig {
                from: from.iter().map(|s| s.to_string()).collect(),
                deny: deny.iter().map(|s| s.to_string()).collect(),
                except: None,
            }),
        }
    }

    #[test]
    fn test_basic_violation() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 2, &["getConnection"], 5));

        let config = LintConfig {
            rules: vec![make_boundary_rule(
                "no-ui-to-db",
                Severity::Error,
                &["src/ui/**"],
                &["src/db/**"],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].rule_id, "no-ui-to-db");
        assert_eq!(result.violations[0].severity, Severity::Error);
        assert_eq!(
            result.violations[0].source_file,
            PathBuf::from("src/ui/Button.ts")
        );
        assert_eq!(
            result.violations[0].target_file,
            PathBuf::from("src/db/connection.ts")
        );
        assert_eq!(result.violations[0].line, 5);
        assert_eq!(result.summary.errors, 1);
    }

    #[test]
    fn test_no_violation_when_not_matching() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/services/api.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 2, &["getConnection"], 3));

        let config = LintConfig {
            rules: vec![make_boundary_rule(
                "no-ui-to-db",
                Severity::Error,
                &["src/ui/**"],
                &["src/db/**"],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_exception_prevents_violation() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/types.ts"));
        graph.add_import(make_edge(1, 2, &["DbType"], 10));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-ui-to-db".to_string(),
                severity: Severity::Error,
                description: "No UI to DB".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::Boundary(BoundaryRuleConfig {
                    from: vec!["src/ui/**".to_string()],
                    deny: vec!["src/db/**".to_string()],
                    except: Some(vec!["src/db/types.ts".to_string()]),
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_multiple_violations() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/ui/Header.ts"));
        graph.add_file(make_file(3, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 3, &["getConnection"], 5));
        graph.add_import(make_edge(2, 3, &["getConnection"], 3));

        let config = LintConfig {
            rules: vec![make_boundary_rule(
                "no-ui-to-db",
                Severity::Error,
                &["src/ui/**"],
                &["src/db/**"],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 2);
    }

    #[test]
    fn test_empty_rules() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 2, &["x"], 1));

        let config = LintConfig { rules: vec![] };
        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
        assert_eq!(result.rules_evaluated, 0);
    }

    #[test]
    fn test_empty_from_matches_nothing() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 2, &["x"], 1));

        let config = LintConfig {
            rules: vec![make_boundary_rule("no-match", Severity::Error, &[], &["src/db/**"])],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_summary_counts() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_file(make_file(3, "src/api/endpoint.ts"));
        graph.add_import(make_edge(1, 2, &["x"], 1));
        graph.add_import(make_edge(1, 3, &["y"], 2));

        let config = LintConfig {
            rules: vec![
                make_boundary_rule("no-ui-to-db", Severity::Error, &["src/ui/**"], &["src/db/**"]),
                make_boundary_rule(
                    "no-ui-to-api",
                    Severity::Warning,
                    &["src/ui/**"],
                    &["src/api/**"],
                ),
            ],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.summary.total_violations, 2);
        assert_eq!(result.summary.errors, 1);
        assert_eq!(result.summary.warnings, 1);
        assert_eq!(result.summary.infos, 0);
        assert_eq!(result.summary.rules_evaluated, 2);
    }

    #[test]
    fn test_violations_sorted_by_severity() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_file(make_file(3, "src/api/endpoint.ts"));
        graph.add_import(make_edge(1, 2, &["x"], 1));
        graph.add_import(make_edge(1, 3, &["y"], 2));

        let config = LintConfig {
            rules: vec![
                make_boundary_rule(
                    "no-ui-to-api",
                    Severity::Warning,
                    &["src/ui/**"],
                    &["src/api/**"],
                ),
                make_boundary_rule("no-ui-to-db", Severity::Error, &["src/ui/**"], &["src/db/**"]),
            ],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 2);
        // Error should come first
        assert_eq!(result.violations[0].severity, Severity::Error);
        assert_eq!(result.violations[1].severity, Severity::Warning);
    }

    #[test]
    fn test_rationale_and_fix_direction() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/connection.ts"));
        graph.add_import(make_edge(1, 2, &["x"], 1));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-ui-to-db".to_string(),
                severity: Severity::Error,
                description: "No UI to DB".to_string(),
                rationale: Some("UI should use services".to_string()),
                fix_direction: Some("Use the service layer".to_string()),
                rule: RuleKind::Boundary(BoundaryRuleConfig {
                    from: vec!["src/ui/**".to_string()],
                    deny: vec!["src/db/**".to_string()],
                    except: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert_eq!(
            result.violations[0].rationale.as_deref(),
            Some("UI should use services")
        );
        assert_eq!(
            result.violations[0].fix_direction.as_deref(),
            Some("Use the service layer")
        );
    }
}
