use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level lint configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintConfig {
    pub rules: Vec<RuleDefinition>,
}

/// A single lint rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDefinition {
    pub id: String,
    pub severity: Severity,
    pub description: String,
    pub rationale: Option<String>,
    pub fix_direction: Option<String>,
    #[serde(flatten)]
    pub rule: RuleKind,
}

/// The kind of lint rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Boundary(BoundaryRuleConfig),
    Layer(LayerRuleConfig),
    Containment(ContainmentRuleConfig),
    ImportRestriction(ImportRestrictionRuleConfig),
    FanLimit(FanLimitRuleConfig),
}

/// Configuration for a boundary rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryRuleConfig {
    pub from: Vec<String>,
    pub deny: Vec<String>,
    #[serde(default)]
    pub except: Option<Vec<String>>,
}

/// Configuration for a layer hierarchy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRuleConfig {
    pub layers: Vec<LayerDefinition>,
}

/// A single layer in a layer hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDefinition {
    pub name: String,
    pub patterns: Vec<String>,
}

/// Configuration for a module containment rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainmentRuleConfig {
    pub module: Vec<String>,
    pub public_api: Vec<String>,
}

/// Configuration for an import restriction rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRestrictionRuleConfig {
    pub target: Vec<String>,
    #[serde(default)]
    pub require_type_only: bool,
    #[serde(default)]
    pub forbidden_names: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_names: Option<Vec<String>>,
}

/// Configuration for a fan-in/fan-out limit rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanLimitRuleConfig {
    pub pattern: Vec<String>,
    #[serde(default)]
    pub max_fan_in: Option<u32>,
    #[serde(default)]
    pub max_fan_out: Option<u32>,
}

/// Severity level for a lint rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Info => write!(f, "info"),
        }
    }
}

/// Default config file names, searched in order.
const CONFIG_FILENAMES: &[&str] = &[".statik/rules.toml", "statik.toml"];

/// Find the config file for a project.
///
/// If `config_override` is provided, use that path directly.
/// Otherwise, search for config files in the project root.
pub fn find_config_path(project_root: &Path, config_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(override_path) = config_override {
        if override_path.exists() {
            return Some(override_path.to_path_buf());
        }
        return None;
    }

    for filename in CONFIG_FILENAMES {
        let path = project_root.join(filename);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// User-configurable entry point definitions.
///
/// These are checked IN ADDITION to the built-in entry point heuristics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryPointConfig {
    /// Glob patterns matching entry point files (e.g., `"**/Bootstrap.java"`).
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Annotation names that mark entry points (e.g., `"Scheduled"`).
    #[serde(default)]
    pub annotations: Vec<String>,
}

/// Wrapper for deserializing the optional `[entry_points]` section.
#[derive(Debug, Deserialize)]
struct ConfigWithEntryPoints {
    #[serde(default)]
    entry_points: Option<EntryPointConfig>,
}

/// Load and parse a lint config from a TOML file.
pub fn load_config(path: &Path) -> Result<LintConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    parse_config(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Parse a lint config from a TOML string.
pub fn parse_config(toml_str: &str) -> Result<LintConfig> {
    let config: LintConfig = toml::from_str(toml_str)?;
    Ok(config)
}

/// Load entry point config from a project, returning defaults if no config exists.
pub fn load_entry_point_config(project_root: &Path) -> EntryPointConfig {
    let path = match find_config_path(project_root, None) {
        Some(p) => p,
        None => return EntryPointConfig::default(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return EntryPointConfig::default(),
    };
    match toml::from_str::<ConfigWithEntryPoints>(&content) {
        Ok(wrapper) => wrapper.entry_points.unwrap_or_default(),
        Err(_) => EntryPointConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_config() {
        let toml = r#"
[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI layer must not import from database layer"
rationale = "The UI should go through the service layer"
fix_direction = "Import from src/services/ instead"

[rules.boundary]
from = ["src/ui/**", "src/components/**"]
deny = ["src/db/**"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        let rule = &config.rules[0];
        assert_eq!(rule.id, "no-ui-to-db");
        assert_eq!(rule.severity, Severity::Error);
        assert_eq!(
            rule.description,
            "UI layer must not import from database layer"
        );
        assert_eq!(
            rule.rationale.as_deref(),
            Some("The UI should go through the service layer")
        );
        assert_eq!(
            rule.fix_direction.as_deref(),
            Some("Import from src/services/ instead")
        );

        match &rule.rule {
            RuleKind::Boundary(b) => {
                assert_eq!(b.from, vec!["src/ui/**", "src/components/**"]);
                assert_eq!(b.deny, vec!["src/db/**"]);
                assert!(b.except.is_none());
            }
            other => panic!("Expected Boundary rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_config_with_except() {
        let toml = r#"
[[rules]]
id = "no-cross-feature"
severity = "warning"
description = "Features should not import from each other"

[rules.boundary]
from = ["src/features/auth/**"]
deny = ["src/features/billing/**"]
except = ["src/features/billing/types.ts"]
"#;

        let config = parse_config(toml).unwrap();
        let rule = &config.rules[0];
        match &rule.rule {
            RuleKind::Boundary(b) => {
                assert_eq!(
                    b.except.as_deref(),
                    Some(vec!["src/features/billing/types.ts".to_string()].as_slice())
                );
            }
            other => panic!("Expected Boundary rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_config_required_fields_only() {
        let toml = r#"
[[rules]]
id = "minimal-rule"
severity = "info"
description = "A minimal rule"

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]
"#;

        let config = parse_config(toml).unwrap();
        let rule = &config.rules[0];
        assert_eq!(rule.id, "minimal-rule");
        assert_eq!(rule.severity, Severity::Info);
        assert!(rule.rationale.is_none());
        assert!(rule.fix_direction.is_none());
    }

    #[test]
    fn test_parse_multiple_rules() {
        let toml = r#"
[[rules]]
id = "rule-1"
severity = "error"
description = "First rule"

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]

[[rules]]
id = "rule-2"
severity = "warning"
description = "Second rule"

[rules.boundary]
from = ["src/c/**"]
deny = ["src/d/**"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 2);
        assert_eq!(config.rules[0].id, "rule-1");
        assert_eq!(config.rules[1].id, "rule-2");
    }

    #[test]
    fn test_parse_empty_rules() {
        let toml = r#"
rules = []
"#;

        let config = parse_config(toml).unwrap();
        assert!(config.rules.is_empty());
    }

    #[test]
    fn test_parse_missing_required_field() {
        let toml = r#"
[[rules]]
id = "bad-rule"
severity = "error"
# missing description

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]
"#;

        let result = parse_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_severity() {
        let toml = r#"
[[rules]]
id = "bad-severity"
severity = "critical"
description = "Invalid severity level"

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]
"#;

        let result = parse_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_boundary_section() {
        let toml = r#"
[[rules]]
id = "no-boundary"
severity = "error"
description = "No boundary config"
"#;

        let result = parse_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_severity_levels() {
        for (level, expected) in [
            ("error", Severity::Error),
            ("warning", Severity::Warning),
            ("info", Severity::Info),
        ] {
            let toml = format!(
                r#"
[[rules]]
id = "test"
severity = "{}"
description = "test"

[rules.boundary]
from = ["a"]
deny = ["b"]
"#,
                level
            );
            let config = parse_config(&toml).unwrap();
            assert_eq!(config.rules[0].severity, expected);
        }
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Error.to_string(), "error");
        assert_eq!(Severity::Warning.to_string(), "warning");
        assert_eq!(Severity::Info.to_string(), "info");
    }

    #[test]
    fn test_find_config_with_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("custom.toml");
        std::fs::write(&config_path, "rules = []").unwrap();

        let found = find_config_path(dir.path(), Some(&config_path));
        assert_eq!(found, Some(config_path));
    }

    #[test]
    fn test_find_config_override_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent.toml");
        let found = find_config_path(dir.path(), Some(&missing));
        assert!(found.is_none());
    }

    #[test]
    fn test_find_config_statik_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let statik_dir = dir.path().join(".statik");
        std::fs::create_dir_all(&statik_dir).unwrap();
        let config_path = statik_dir.join("rules.toml");
        std::fs::write(&config_path, "rules = []").unwrap();

        let found = find_config_path(dir.path(), None);
        assert_eq!(found, Some(config_path));
    }

    #[test]
    fn test_find_config_root_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("statik.toml");
        std::fs::write(&config_path, "rules = []").unwrap();

        let found = find_config_path(dir.path(), None);
        assert_eq!(found, Some(config_path));
    }

    #[test]
    fn test_find_config_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let found = find_config_path(dir.path(), None);
        assert!(found.is_none());
    }

    #[test]
    fn test_find_config_prefers_statik_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let statik_dir = dir.path().join(".statik");
        std::fs::create_dir_all(&statik_dir).unwrap();
        std::fs::write(statik_dir.join("rules.toml"), "rules = []").unwrap();
        std::fs::write(dir.path().join("statik.toml"), "rules = []").unwrap();

        let found = find_config_path(dir.path(), None);
        // Should prefer .statik/rules.toml
        assert_eq!(found, Some(statik_dir.join("rules.toml")));
    }

    #[test]
    fn test_parse_layer_rule() {
        let toml = r#"
[[rules]]
id = "clean-layers"
severity = "error"
description = "Dependencies must flow top-down"

[rules.layer]
layers = [
  { name = "presentation", patterns = ["src/ui/**"] },
  { name = "service", patterns = ["src/services/**"] },
  { name = "data", patterns = ["src/db/**"] },
]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::Layer(l) => {
                assert_eq!(l.layers.len(), 3);
                assert_eq!(l.layers[0].name, "presentation");
                assert_eq!(l.layers[1].name, "service");
                assert_eq!(l.layers[2].name, "data");
                assert_eq!(l.layers[0].patterns, vec!["src/ui/**"]);
            }
            other => panic!("Expected Layer rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_containment_rule() {
        let toml = r#"
[[rules]]
id = "auth-encapsulation"
severity = "error"
description = "Auth module must be accessed through its public API"

[rules.containment]
module = ["src/auth/**"]
public_api = ["src/auth/index.ts"]
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::Containment(c) => {
                assert_eq!(c.module, vec!["src/auth/**"]);
                assert_eq!(c.public_api, vec!["src/auth/index.ts"]);
            }
            other => panic!("Expected Containment rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_import_restriction_rule() {
        let toml = r#"
[[rules]]
id = "types-type-only"
severity = "warning"
description = "Imports from types/ must be type-only"

[rules.import_restriction]
target = ["src/types/**"]
require_type_only = true
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::ImportRestriction(r) => {
                assert_eq!(r.target, vec!["src/types/**"]);
                assert!(r.require_type_only);
                assert!(r.forbidden_names.is_none());
                assert!(r.allowed_names.is_none());
            }
            other => panic!("Expected ImportRestriction rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_import_restriction_with_names() {
        let toml = r#"
[[rules]]
id = "no-internals"
severity = "error"
description = "Cannot import internal functions"

[rules.import_restriction]
target = ["src/internal/**"]
forbidden_names = ["getSecret", "internalHelper"]
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::ImportRestriction(r) => {
                assert!(!r.require_type_only);
                assert_eq!(
                    r.forbidden_names.as_deref(),
                    Some(["getSecret".to_string(), "internalHelper".to_string()].as_slice())
                );
            }
            other => panic!("Expected ImportRestriction rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fan_limit_rule() {
        let toml = r#"
[[rules]]
id = "no-god-modules"
severity = "warning"
description = "Files should not have too many dependencies"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_out = 20
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::FanLimit(f) => {
                assert_eq!(f.pattern, vec!["src/**"]);
                assert_eq!(f.max_fan_out, Some(20));
                assert!(f.max_fan_in.is_none());
            }
            other => panic!("Expected FanLimit rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_fan_limit_both_directions() {
        let toml = r#"
[[rules]]
id = "limits"
severity = "info"
description = "Fan limits"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_in = 10
max_fan_out = 15
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::FanLimit(f) => {
                assert_eq!(f.max_fan_in, Some(10));
                assert_eq!(f.max_fan_out, Some(15));
            }
            other => panic!("Expected FanLimit rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_mixed_rule_types() {
        let toml = r#"
[[rules]]
id = "boundary"
severity = "error"
description = "Boundary rule"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]

[[rules]]
id = "layers"
severity = "error"
description = "Layer rule"

[rules.layer]
layers = [
  { name = "ui", patterns = ["src/ui/**"] },
  { name = "db", patterns = ["src/db/**"] },
]

[[rules]]
id = "contain"
severity = "warning"
description = "Containment rule"

[rules.containment]
module = ["src/auth/**"]
public_api = ["src/auth/index.ts"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 3);
        assert!(matches!(&config.rules[0].rule, RuleKind::Boundary(_)));
        assert!(matches!(&config.rules[1].rule, RuleKind::Layer(_)));
        assert!(matches!(&config.rules[2].rule, RuleKind::Containment(_)));
    }

    // =========================================================================
    // Entry point config
    // =========================================================================

    #[test]
    fn test_parse_entry_points_config() {
        let toml = r#"
rules = []

[entry_points]
patterns = ["**/Bootstrap.java", "**/Main.java"]
annotations = ["Scheduled", "MyCustomEntryPoint"]
"#;
        // LintConfig parsing should still work (ignores unknown sections)
        let lint = parse_config(toml).unwrap();
        assert!(lint.rules.is_empty());

        // Entry point config should parse correctly
        let wrapper: ConfigWithEntryPoints = toml::from_str(toml).unwrap();
        let ep = wrapper.entry_points.unwrap();
        assert_eq!(ep.patterns, vec!["**/Bootstrap.java", "**/Main.java"]);
        assert_eq!(ep.annotations, vec!["Scheduled", "MyCustomEntryPoint"]);
    }

    #[test]
    fn test_parse_entry_points_with_rules() {
        let toml = r#"
[[rules]]
id = "test"
severity = "error"
description = "test rule"

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]

[entry_points]
patterns = ["**/Startup.java"]
annotations = ["Cron"]
"#;
        let lint = parse_config(toml).unwrap();
        assert_eq!(lint.rules.len(), 1);

        let wrapper: ConfigWithEntryPoints = toml::from_str(toml).unwrap();
        let ep = wrapper.entry_points.unwrap();
        assert_eq!(ep.patterns, vec!["**/Startup.java"]);
        assert_eq!(ep.annotations, vec!["Cron"]);
    }

    #[test]
    fn test_parse_no_entry_points_section() {
        let toml = r#"
rules = []
"#;
        let wrapper: ConfigWithEntryPoints = toml::from_str(toml).unwrap();
        assert!(wrapper.entry_points.is_none());
    }

    #[test]
    fn test_parse_empty_entry_points() {
        let toml = r#"
rules = []

[entry_points]
"#;
        let wrapper: ConfigWithEntryPoints = toml::from_str(toml).unwrap();
        let ep = wrapper.entry_points.unwrap();
        assert!(ep.patterns.is_empty());
        assert!(ep.annotations.is_empty());
    }

    #[test]
    fn test_load_entry_point_config_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let ep = load_entry_point_config(dir.path());
        assert!(ep.patterns.is_empty());
        assert!(ep.annotations.is_empty());
    }

    #[test]
    fn test_load_entry_point_config_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let statik_dir = dir.path().join(".statik");
        std::fs::create_dir_all(&statik_dir).unwrap();
        std::fs::write(
            statik_dir.join("rules.toml"),
            r#"
rules = []

[entry_points]
patterns = ["**/Batch.java"]
annotations = ["Scheduled"]
"#,
        )
        .unwrap();

        let ep = load_entry_point_config(dir.path());
        assert_eq!(ep.patterns, vec!["**/Batch.java"]);
        assert_eq!(ep.annotations, vec!["Scheduled"]);
    }
}
