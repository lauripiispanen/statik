use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::analysis::Confidence;
use crate::model::file_graph::FileGraph;

use std::collections::HashSet;

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
            RuleKind::Layer(layer_config) => {
                let layer_matchers: Vec<(String, FileMatcher)> = layer_config
                    .layers
                    .iter()
                    .map(|l| Ok((l.name.clone(), FileMatcher::new(&l.patterns)?)))
                    .collect::<Result<Vec<_>>>()?;

                for (source_id, edges) in graph.imports.iter() {
                    let source_info = match graph.files.get(source_id) {
                        Some(info) => info,
                        None => continue,
                    };
                    let source_rel = to_relative(&source_info.path, project_root);

                    // Find source layer (first match)
                    let source_layer = layer_matchers
                        .iter()
                        .position(|(_, m)| m.matches(source_rel));
                    let source_layer = match source_layer {
                        Some(idx) => idx,
                        None => continue, // not in any layer, skip
                    };

                    for edge in edges {
                        let target_info = match graph.files.get(&edge.to) {
                            Some(info) => info,
                            None => continue,
                        };
                        let target_rel = to_relative(&target_info.path, project_root);

                        let target_layer = layer_matchers
                            .iter()
                            .position(|(_, m)| m.matches(target_rel));
                        let target_layer = match target_layer {
                            Some(idx) => idx,
                            None => continue, // not in any layer, skip
                        };

                        // Top-down: lower index = higher layer. A file can import
                        // from the same layer or from layers below (higher index).
                        // Violation: source is below target (source index > target index).
                        if source_layer > target_layer {
                            all_violations.push(LintViolation {
                                rule_id: rule_def.id.clone(),
                                severity: rule_def.severity,
                                description: format!(
                                    "{} (layer '{}' must not import from layer '{}')",
                                    rule_def.description,
                                    layer_matchers[source_layer].0,
                                    layer_matchers[target_layer].0,
                                ),
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
            RuleKind::Containment(containment) => {
                let module_matcher = FileMatcher::new(&containment.module)?;
                let public_api_matcher = FileMatcher::new(&containment.public_api)?;

                for (source_id, edges) in graph.imports.iter() {
                    let source_info = match graph.files.get(source_id) {
                        Some(info) => info,
                        None => continue,
                    };
                    let source_rel = to_relative(&source_info.path, project_root);

                    // Only check imports from outside the module
                    if module_matcher.matches(source_rel) {
                        continue;
                    }

                    for edge in edges {
                        let target_info = match graph.files.get(&edge.to) {
                            Some(info) => info,
                            None => continue,
                        };
                        let target_rel = to_relative(&target_info.path, project_root);

                        // Only check imports into the module
                        if !module_matcher.matches(target_rel) {
                            continue;
                        }

                        // Allow if target is a public API file
                        if public_api_matcher.matches(target_rel) {
                            continue;
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
            RuleKind::ImportRestriction(restriction) => {
                let target_matcher = FileMatcher::new(&restriction.target)?;

                for (_source_id, edges) in graph.imports.iter() {
                    for edge in edges {
                        let target_info = match graph.files.get(&edge.to) {
                            Some(info) => info,
                            None => continue,
                        };
                        let target_rel = to_relative(&target_info.path, project_root);

                        if !target_matcher.matches(target_rel) {
                            continue;
                        }

                        let source_info = match graph.files.get(&edge.from) {
                            Some(info) => info,
                            None => continue,
                        };
                        let source_rel = to_relative(&source_info.path, project_root);

                        // Check type-only requirement
                        if restriction.require_type_only && !edge.is_type_only {
                            all_violations.push(LintViolation {
                                rule_id: rule_def.id.clone(),
                                severity: rule_def.severity,
                                description: format!(
                                    "{} (import must be type-only)",
                                    rule_def.description,
                                ),
                                rationale: rule_def.rationale.clone(),
                                source_file: source_rel.to_path_buf(),
                                target_file: target_rel.to_path_buf(),
                                imported_names: edge.imported_names.clone(),
                                line: edge.line,
                                confidence: Confidence::Certain,
                                fix_direction: rule_def.fix_direction.clone(),
                            });
                            continue;
                        }

                        // Check forbidden names
                        if let Some(ref forbidden) = restriction.forbidden_names {
                            let forbidden_set: HashSet<&str> =
                                forbidden.iter().map(|s| s.as_str()).collect();
                            let violated_names: Vec<String> = edge
                                .imported_names
                                .iter()
                                .filter(|n| forbidden_set.contains(n.as_str()))
                                .cloned()
                                .collect();
                            if !violated_names.is_empty() {
                                all_violations.push(LintViolation {
                                    rule_id: rule_def.id.clone(),
                                    severity: rule_def.severity,
                                    description: format!(
                                        "{} (forbidden imports: {})",
                                        rule_def.description,
                                        violated_names.join(", "),
                                    ),
                                    rationale: rule_def.rationale.clone(),
                                    source_file: source_rel.to_path_buf(),
                                    target_file: target_rel.to_path_buf(),
                                    imported_names: violated_names,
                                    line: edge.line,
                                    confidence: Confidence::Certain,
                                    fix_direction: rule_def.fix_direction.clone(),
                                });
                            }
                        }

                        // Check allowed names (allowlist mode)
                        if let Some(ref allowed) = restriction.allowed_names {
                            let allowed_set: HashSet<&str> =
                                allowed.iter().map(|s| s.as_str()).collect();
                            let violated_names: Vec<String> = edge
                                .imported_names
                                .iter()
                                .filter(|n| !allowed_set.contains(n.as_str()))
                                .cloned()
                                .collect();
                            if !violated_names.is_empty() {
                                all_violations.push(LintViolation {
                                    rule_id: rule_def.id.clone(),
                                    severity: rule_def.severity,
                                    description: format!(
                                        "{} (imports not in allowlist: {})",
                                        rule_def.description,
                                        violated_names.join(", "),
                                    ),
                                    rationale: rule_def.rationale.clone(),
                                    source_file: source_rel.to_path_buf(),
                                    target_file: target_rel.to_path_buf(),
                                    imported_names: violated_names,
                                    line: edge.line,
                                    confidence: Confidence::Certain,
                                    fix_direction: rule_def.fix_direction.clone(),
                                });
                            }
                        }
                    }
                }
            }
            RuleKind::FanLimit(fan_config) => {
                let pattern_matcher = FileMatcher::new(&fan_config.pattern)?;

                for (file_id, file_info) in graph.files.iter() {
                    let file_rel = to_relative(&file_info.path, project_root);

                    if !pattern_matcher.matches(file_rel) {
                        continue;
                    }

                    // Count fan-out (distinct files this file imports)
                    if let Some(max_out) = fan_config.max_fan_out {
                        let fan_out = graph
                            .import_edges(*file_id)
                            .map(|edges| {
                                edges.iter().map(|e| e.to).collect::<HashSet<_>>().len()
                            })
                            .unwrap_or(0);

                        if fan_out > max_out as usize {
                            all_violations.push(LintViolation {
                                rule_id: rule_def.id.clone(),
                                severity: rule_def.severity,
                                description: format!(
                                    "{} (fan-out {} exceeds limit {})",
                                    rule_def.description, fan_out, max_out,
                                ),
                                rationale: rule_def.rationale.clone(),
                                source_file: file_rel.to_path_buf(),
                                target_file: file_rel.to_path_buf(),
                                imported_names: vec![],
                                line: 0,
                                confidence: Confidence::Certain,
                                fix_direction: rule_def.fix_direction.clone(),
                            });
                        }
                    }

                    // Count fan-in (distinct files that import this file)
                    if let Some(max_in) = fan_config.max_fan_in {
                        let fan_in = graph
                            .imported_by_edges(*file_id)
                            .map(|edges| {
                                edges.iter().map(|e| e.from).collect::<HashSet<_>>().len()
                            })
                            .unwrap_or(0);

                        if fan_in > max_in as usize {
                            all_violations.push(LintViolation {
                                rule_id: rule_def.id.clone(),
                                severity: rule_def.severity,
                                description: format!(
                                    "{} (fan-in {} exceeds limit {})",
                                    rule_def.description, fan_in, max_in,
                                ),
                                rationale: rule_def.rationale.clone(),
                                source_file: file_rel.to_path_buf(),
                                target_file: file_rel.to_path_buf(),
                                imported_names: vec![],
                                line: 0,
                                confidence: Confidence::Certain,
                                fix_direction: rule_def.fix_direction.clone(),
                            });
                        }
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
    use crate::linting::config::{
        BoundaryRuleConfig, ContainmentRuleConfig, FanLimitRuleConfig,
        ImportRestrictionRuleConfig, LayerDefinition, LayerRuleConfig, RuleDefinition,
    };
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

    fn make_type_only_edge(from: u64, to: u64, names: &[&str], line: usize) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: names.iter().map(|s| s.to_string()).collect(),
            is_type_only: true,
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

    // ---- Layer hierarchy rule tests ----

    fn make_layer_rule(id: &str, severity: Severity, layers: &[(&str, &[&str])]) -> RuleDefinition {
        RuleDefinition {
            id: id.to_string(),
            severity,
            description: format!("Rule: {}", id),
            rationale: None,
            fix_direction: None,
            rule: RuleKind::Layer(LayerRuleConfig {
                layers: layers
                    .iter()
                    .map(|(name, patterns)| LayerDefinition {
                        name: name.to_string(),
                        patterns: patterns.iter().map(|s| s.to_string()).collect(),
                    })
                    .collect(),
            }),
        }
    }

    #[test]
    fn test_layer_violation_lower_imports_higher() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/db/repo.ts"));
        graph.add_file(make_file(2, "src/ui/Button.ts"));
        // data layer imports from presentation layer (violation)
        graph.add_import(make_edge(1, 2, &["Button"], 3));

        let config = LintConfig {
            rules: vec![make_layer_rule(
                "clean-layers",
                Severity::Error,
                &[
                    ("presentation", &["src/ui/**"]),
                    ("service", &["src/services/**"]),
                    ("data", &["src/db/**"]),
                ],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].description.contains("data"));
        assert!(result.violations[0].description.contains("presentation"));
    }

    #[test]
    fn test_layer_valid_top_down_dependency() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/services/userService.ts"));
        graph.add_file(make_file(3, "src/db/repo.ts"));
        // presentation -> service (valid top-down)
        graph.add_import(make_edge(1, 2, &["UserService"], 1));
        // service -> data (valid top-down)
        graph.add_import(make_edge(2, 3, &["UserRepo"], 2));

        let config = LintConfig {
            rules: vec![make_layer_rule(
                "clean-layers",
                Severity::Error,
                &[
                    ("presentation", &["src/ui/**"]),
                    ("service", &["src/services/**"]),
                    ("data", &["src/db/**"]),
                ],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_layer_same_layer_import_allowed() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/ui/Header.ts"));
        graph.add_import(make_edge(1, 2, &["Header"], 1));

        let config = LintConfig {
            rules: vec![make_layer_rule(
                "clean-layers",
                Severity::Error,
                &[
                    ("presentation", &["src/ui/**"]),
                    ("data", &["src/db/**"]),
                ],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_layer_file_not_in_any_layer_skipped() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/utils/helpers.ts"));
        graph.add_file(make_file(2, "src/ui/Button.ts"));
        // utils not in any layer — should not produce violations
        graph.add_import(make_edge(1, 2, &["Button"], 1));

        let config = LintConfig {
            rules: vec![make_layer_rule(
                "clean-layers",
                Severity::Error,
                &[
                    ("presentation", &["src/ui/**"]),
                    ("data", &["src/db/**"]),
                ],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_layer_skip_across_layers_allowed() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/ui/Button.ts"));
        graph.add_file(make_file(2, "src/db/repo.ts"));
        // presentation -> data (skipping service) is valid top-down
        graph.add_import(make_edge(1, 2, &["Repo"], 1));

        let config = LintConfig {
            rules: vec![make_layer_rule(
                "clean-layers",
                Severity::Error,
                &[
                    ("presentation", &["src/ui/**"]),
                    ("service", &["src/services/**"]),
                    ("data", &["src/db/**"]),
                ],
            )],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    // ---- Containment rule tests ----

    #[test]
    fn test_containment_violation_direct_internal_import() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/auth/utils.ts"));
        graph.add_file(make_file(3, "src/auth/index.ts"));
        // app.ts imports directly from auth/utils.ts (violation)
        graph.add_import(make_edge(1, 2, &["hashPassword"], 5));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "auth-encapsulation".to_string(),
                severity: Severity::Error,
                description: "Auth module must be accessed through index.ts".to_string(),
                rationale: None,
                fix_direction: Some("Import from src/auth/index.ts instead".to_string()),
                rule: RuleKind::Containment(ContainmentRuleConfig {
                    module: vec!["src/auth/**".to_string()],
                    public_api: vec!["src/auth/index.ts".to_string()],
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert_eq!(
            result.violations[0].target_file,
            PathBuf::from("src/auth/utils.ts")
        );
    }

    #[test]
    fn test_containment_import_through_public_api_allowed() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/auth/index.ts"));
        // app.ts imports from auth/index.ts (allowed)
        graph.add_import(make_edge(1, 2, &["login"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "auth-encapsulation".to_string(),
                severity: Severity::Error,
                description: "Auth must be accessed through index.ts".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::Containment(ContainmentRuleConfig {
                    module: vec!["src/auth/**".to_string()],
                    public_api: vec!["src/auth/index.ts".to_string()],
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_containment_internal_imports_not_checked() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/auth/service.ts"));
        graph.add_file(make_file(2, "src/auth/utils.ts"));
        // internal module import — should not be checked
        graph.add_import(make_edge(1, 2, &["hashPassword"], 2));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "auth-encapsulation".to_string(),
                severity: Severity::Error,
                description: "Auth must be accessed through index.ts".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::Containment(ContainmentRuleConfig {
                    module: vec!["src/auth/**".to_string()],
                    public_api: vec!["src/auth/index.ts".to_string()],
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    // ---- Import restriction rule tests ----

    #[test]
    fn test_import_restriction_require_type_only_violation() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/types/user.ts"));
        // Non-type-only import from types/ (violation)
        graph.add_import(make_edge(1, 2, &["User"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "types-type-only".to_string(),
                severity: Severity::Warning,
                description: "Imports from types/ must be type-only".to_string(),
                rationale: None,
                fix_direction: Some("Use 'import type' instead".to_string()),
                rule: RuleKind::ImportRestriction(ImportRestrictionRuleConfig {
                    target: vec!["src/types/**".to_string()],
                    require_type_only: true,
                    forbidden_names: None,
                    allowed_names: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].description.contains("type-only"));
    }

    #[test]
    fn test_import_restriction_type_only_passes() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/types/user.ts"));
        // Type-only import (valid)
        graph.add_import(make_type_only_edge(1, 2, &["User"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "types-type-only".to_string(),
                severity: Severity::Warning,
                description: "Imports from types/ must be type-only".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::ImportRestriction(ImportRestrictionRuleConfig {
                    target: vec!["src/types/**".to_string()],
                    require_type_only: true,
                    forbidden_names: None,
                    allowed_names: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_import_restriction_forbidden_names() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/internal/secrets.ts"));
        graph.add_import(make_edge(1, 2, &["getSecret", "Config"], 5));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-secrets".to_string(),
                severity: Severity::Error,
                description: "Cannot import secret functions".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::ImportRestriction(ImportRestrictionRuleConfig {
                    target: vec!["src/internal/**".to_string()],
                    require_type_only: false,
                    forbidden_names: Some(vec!["getSecret".to_string()]),
                    allowed_names: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].imported_names, vec!["getSecret"]);
    }

    #[test]
    fn test_import_restriction_allowed_names() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/core/engine.ts"));
        graph.add_import(make_edge(1, 2, &["Engine", "internalHelper"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "core-allowlist".to_string(),
                severity: Severity::Warning,
                description: "Only specific exports from core/ are allowed".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::ImportRestriction(ImportRestrictionRuleConfig {
                    target: vec!["src/core/**".to_string()],
                    require_type_only: false,
                    forbidden_names: None,
                    allowed_names: Some(vec!["Engine".to_string()]),
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].imported_names, vec!["internalHelper"]);
    }

    #[test]
    fn test_import_restriction_no_forbidden_names_matched() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/internal/helpers.ts"));
        graph.add_import(make_edge(1, 2, &["safeHelper"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-secrets".to_string(),
                severity: Severity::Error,
                description: "Cannot import secret functions".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::ImportRestriction(ImportRestrictionRuleConfig {
                    target: vec!["src/internal/**".to_string()],
                    require_type_only: false,
                    forbidden_names: Some(vec!["getSecret".to_string()]),
                    allowed_names: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    // ---- Fan-in/fan-out limit rule tests ----

    #[test]
    fn test_fan_out_exceeds_limit() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/god-module.ts"));
        graph.add_file(make_file(2, "src/dep1.ts"));
        graph.add_file(make_file(3, "src/dep2.ts"));
        graph.add_file(make_file(4, "src/dep3.ts"));
        graph.add_import(make_edge(1, 2, &["a"], 1));
        graph.add_import(make_edge(1, 3, &["b"], 2));
        graph.add_import(make_edge(1, 4, &["c"], 3));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-god-modules".to_string(),
                severity: Severity::Warning,
                description: "Too many dependencies".to_string(),
                rationale: None,
                fix_direction: Some("Split this file into smaller modules".to_string()),
                rule: RuleKind::FanLimit(FanLimitRuleConfig {
                    pattern: vec!["src/**".to_string()],
                    max_fan_in: None,
                    max_fan_out: Some(2),
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].description.contains("fan-out 3"));
        assert!(result.violations[0].description.contains("limit 2"));
    }

    #[test]
    fn test_fan_in_exceeds_limit() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/shared/utils.ts"));
        graph.add_file(make_file(2, "src/a.ts"));
        graph.add_file(make_file(3, "src/b.ts"));
        graph.add_file(make_file(4, "src/c.ts"));
        graph.add_import(make_edge(2, 1, &["helper"], 1));
        graph.add_import(make_edge(3, 1, &["helper"], 1));
        graph.add_import(make_edge(4, 1, &["helper"], 1));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "no-bottlenecks".to_string(),
                severity: Severity::Info,
                description: "Too many dependents".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::FanLimit(FanLimitRuleConfig {
                    pattern: vec!["src/**".to_string()],
                    max_fan_in: Some(2),
                    max_fan_out: None,
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        // Only src/shared/utils.ts should violate (3 importers > limit 2)
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].description.contains("fan-in 3"));
    }

    #[test]
    fn test_fan_within_limits() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "src/utils.ts"));
        graph.add_import(make_edge(1, 2, &["helper"], 1));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "limits".to_string(),
                severity: Severity::Warning,
                description: "Limits".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::FanLimit(FanLimitRuleConfig {
                    pattern: vec!["src/**".to_string()],
                    max_fan_in: Some(5),
                    max_fan_out: Some(5),
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_fan_limit_pattern_filtering() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/app.ts"));
        graph.add_file(make_file(2, "lib/external.ts"));
        graph.add_file(make_file(3, "src/dep1.ts"));
        graph.add_file(make_file(4, "src/dep2.ts"));
        // lib/external.ts has 2 deps but pattern only matches src/**
        graph.add_import(make_edge(2, 3, &["a"], 1));
        graph.add_import(make_edge(2, 4, &["b"], 2));

        let config = LintConfig {
            rules: vec![RuleDefinition {
                id: "limits".to_string(),
                severity: Severity::Warning,
                description: "Limits".to_string(),
                rationale: None,
                fix_direction: None,
                rule: RuleKind::FanLimit(FanLimitRuleConfig {
                    pattern: vec!["src/**".to_string()],
                    max_fan_in: None,
                    max_fan_out: Some(1),
                }),
            }],
        };

        let result = evaluate_rules(&config, &graph, Path::new("/project")).unwrap();
        // lib/external.ts should NOT be checked (doesn't match src/**)
        assert!(result.violations.is_empty());
    }
}
