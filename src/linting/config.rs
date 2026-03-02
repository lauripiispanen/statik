use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level lint configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LintConfig {
    #[serde(default)]
    pub rules: Vec<RuleDefinition>,
    #[serde(default)]
    pub tags: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub scope: HashMap<String, ScopeSetConfig>,
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
    CyclePolicy(CyclePolicyRuleConfig),
    StabilityLimit(StabilityLimitRuleConfig),
    NamingBoundary(NamingBoundaryRuleConfig),
    RestrictedConsumer(RestrictedConsumerRuleConfig),
    ExportLimit(ExportLimitRuleConfig),
    CouplingWeight(CouplingWeightRuleConfig),
    Cohesion(CohesionRuleConfig),
    TagBoundary(TagBoundaryRuleConfig),
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

/// Configuration for a cycle policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CyclePolicyRuleConfig {
    #[serde(default)]
    pub max_cycle_length: usize,
    #[serde(default)]
    pub pattern: Option<Vec<String>>,
}

/// Configuration for a stability limit rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityLimitRuleConfig {
    pub pattern: Vec<String>,
    pub max_instability: f64,
}

/// Configuration for a naming boundary rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingBoundaryRuleConfig {
    pub pattern: Vec<String>,
    pub must_match: String,
}

/// Configuration for a restricted consumer rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestrictedConsumerRuleConfig {
    pub target: Vec<String>,
    pub allowed_consumers: Vec<String>,
}

/// Configuration for an export limit rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportLimitRuleConfig {
    pub pattern: Vec<String>,
    pub max_exports: u32,
}

/// Configuration for a coupling weight rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingWeightRuleConfig {
    pub pattern: Vec<String>,
    pub max_names_per_edge: u32,
}

/// Configuration for a cohesion rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CohesionRuleConfig {
    pub pattern: Vec<String>,
    pub max_external_ratio: f64,
}

/// Configuration for a tag-based boundary rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagBoundaryRuleConfig {
    pub from_tag: String,
    pub deny_tags: Vec<String>,
    #[serde(default)]
    pub except_tags: Option<Vec<String>>,
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

/// Configuration for a single scope source set.
///
/// Defined in the `[scope.<name>]` section of the config file.
/// Controls which files belong to this source set and how they
/// are treated during analysis and linting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeSetConfig {
    /// Glob patterns for files included in this source set.
    pub include: Vec<String>,
    /// Glob patterns for files to exclude from this source set.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Role for files in this source set (e.g., "entry_point").
    #[serde(default)]
    pub role: Option<String>,
    /// Whether files in this source set are subject to lint rules.
    /// Defaults to true.
    #[serde(default = "default_true")]
    pub lint: bool,
    /// Whether files in this source set appear in analysis output.
    /// Defaults to true.
    #[serde(default = "default_true")]
    pub analysis: bool,
}

fn default_true() -> bool {
    true
}

impl ScopeSetConfig {
    /// Check if this source set has the given role.
    pub fn has_role(&self, role: &str) -> bool {
        self.role.as_deref() == Some(role)
    }
}

/// Compiled scope configuration with precompiled glob matchers.
///
/// Built from the `[scope]` config section. Provides file classification
/// and role/lint/analysis queries.
#[derive(Debug, Clone)]
pub struct ScopeIndex {
    /// Ordered list of (set_name, include_matcher, exclude_matcher, config).
    sets: Vec<(String, globset::GlobSet, globset::GlobSet, ScopeSetConfig)>,
}

impl ScopeIndex {
    /// Build a ScopeIndex from the scope config map.
    pub fn build(scope: &HashMap<String, ScopeSetConfig>) -> Result<Self> {
        let mut sets = Vec::new();
        // Use sorted keys for deterministic ordering
        let mut keys: Vec<&String> = scope.keys().collect();
        keys.sort();
        for name in keys {
            let config = &scope[name];
            let include = Self::build_globset(&config.include)
                .with_context(|| format!("invalid include patterns in scope.{}", name))?;
            let exclude = Self::build_globset(&config.exclude)
                .with_context(|| format!("invalid exclude patterns in scope.{}", name))?;
            sets.push((name.clone(), include, exclude, config.clone()));
        }
        Ok(Self { sets })
    }

    /// Classify a project-relative file path into a source set name.
    ///
    /// Returns the name of the first matching source set, or `None`
    /// if the file does not belong to any configured source set.
    pub fn classify(&self, rel_path: &Path) -> Option<&str> {
        for (name, include, exclude, _) in &self.sets {
            if include.is_match(rel_path) && !exclude.is_match(rel_path) {
                return Some(name.as_str());
            }
        }
        None
    }

    /// Check if a source set has lint enabled.
    /// Returns true if the source set is not found (default behavior).
    pub fn lint_enabled(&self, set_name: &str) -> bool {
        self.sets
            .iter()
            .find(|(name, _, _, _)| name == set_name)
            .map(|(_, _, _, config)| config.lint)
            .unwrap_or(true)
    }

    /// Check if a source set has analysis enabled.
    /// Returns true if the source set is not found (default behavior).
    pub fn analysis_enabled(&self, set_name: &str) -> bool {
        self.sets
            .iter()
            .find(|(name, _, _, _)| name == set_name)
            .map(|(_, _, _, config)| config.analysis)
            .unwrap_or(true)
    }

    /// Check if a source set has the given role.
    pub fn has_role(&self, set_name: &str, role: &str) -> bool {
        self.sets
            .iter()
            .find(|(name, _, _, _)| name == set_name)
            .map(|(_, _, _, config)| config.has_role(role))
            .unwrap_or(false)
    }

    /// Return names of source sets that have `analysis = false`.
    pub fn analysis_disabled_set_names(&self) -> Vec<&str> {
        self.sets
            .iter()
            .filter(|(_, _, _, config)| !config.analysis)
            .map(|(name, _, _, _)| name.as_str())
            .collect()
    }

    /// Whether any source sets are configured.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    fn build_globset(patterns: &[String]) -> Result<globset::GlobSet> {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(
                globset::Glob::new(pattern)
                    .with_context(|| format!("invalid glob pattern: {}", pattern))?,
            );
        }
        Ok(builder.build()?)
    }
}

/// Load scope config from a project, returning empty map if no config exists.
pub fn load_scope_config(project_root: &Path) -> HashMap<String, ScopeSetConfig> {
    let path = match find_config_path(project_root, None) {
        Some(p) => p,
        None => return HashMap::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    match toml::from_str::<LintConfig>(&content) {
        Ok(config) => config.scope,
        Err(_) => HashMap::new(),
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

/// Java-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JavaConfig {
    /// Explicit source root directories relative to the project root
    /// (e.g., `["server/core/src/main/java"]`).
    #[serde(default)]
    pub source_roots: Vec<String>,
}

/// Wrapper for deserializing the optional `[java]` section.
#[derive(Debug, Deserialize)]
struct ConfigWithJava {
    #[serde(default)]
    java: Option<JavaConfig>,
}

/// Load Java config from a project, returning None if no config or no `[java]` section.
pub fn load_java_config(project_root: &Path) -> Option<JavaConfig> {
    let path = find_config_path(project_root, None)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let wrapper: ConfigWithJava = toml::from_str(&content).ok()?;
    wrapper.java.filter(|c| !c.source_roots.is_empty())
}

/// Wrapper for deserializing the optional `[teams]` section.
#[derive(Debug, Deserialize)]
struct ConfigWithTeams {
    #[serde(default)]
    teams: HashMap<String, Vec<String>>,
}

/// Load team config from a project, returning empty map if no config or no `[teams]` section.
///
/// Teams are defined as: `team_name = ["email_pattern1", "email_pattern2"]`
/// Email patterns support `*` wildcard prefix (e.g., `*@platform.example.com`).
pub fn load_team_config(project_root: &Path) -> HashMap<String, Vec<String>> {
    let path = match find_config_path(project_root, None) {
        Some(p) => p,
        None => return HashMap::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    match toml::from_str::<ConfigWithTeams>(&content) {
        Ok(wrapper) => wrapper.teams,
        Err(_) => HashMap::new(),
    }
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
    /// Glob patterns matching files where ALL symbols should be considered alive.
    /// Use for test fixtures, test utilities, and other directories where every
    /// symbol is considered live infrastructure (e.g., `"test-fixtures/**"`).
    #[serde(default, alias = "seed_all_patterns")]
    pub always_alive: Vec<String>,
}

/// Wrapper for deserializing the optional `[entry_points]` section.
#[derive(Debug, Deserialize)]
struct ConfigWithEntryPoints {
    #[serde(default)]
    entry_points: Option<EntryPointConfig>,
}

/// Wrapper for deserializing the optional `[[source_sets]]` section.
#[derive(Debug, Deserialize)]
struct ConfigWithSourceSets {
    #[serde(default)]
    source_sets: Vec<crate::resolver::source_sets::SourceSetConfig>,
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

/// Load source set configs from a project, returning empty vec if no config exists.
pub fn load_source_set_config(
    project_root: &Path,
) -> Vec<crate::resolver::source_sets::SourceSetConfig> {
    let path = match find_config_path(project_root, None) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match toml::from_str::<ConfigWithSourceSets>(&content) {
        Ok(wrapper) => wrapper.source_sets,
        Err(_) => Vec::new(),
    }
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
    fn test_parse_tag_boundary_rule() {
        let toml = r#"
[tags]
api = ["src/api/**"]
internal = ["src/internal/**"]
shared = ["src/shared/**"]

[[rules]]
id = "no-api-to-internal"
severity = "error"
description = "API must not access internal modules"

[rules.tag_boundary]
from_tag = "api"
deny_tags = ["internal"]
except_tags = ["shared"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.tags.len(), 3);
        assert_eq!(config.tags["api"], vec!["src/api/**"]);
        assert_eq!(config.tags["internal"], vec!["src/internal/**"]);

        match &config.rules[0].rule {
            RuleKind::TagBoundary(t) => {
                assert_eq!(t.from_tag, "api");
                assert_eq!(t.deny_tags, vec!["internal"]);
                assert_eq!(
                    t.except_tags.as_deref(),
                    Some(vec!["shared".to_string()].as_slice())
                );
            }
            other => panic!("Expected TagBoundary rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tag_boundary_no_except() {
        let toml = r#"
[tags]
api = ["src/api/**"]
db = ["src/db/**"]

[[rules]]
id = "no-api-to-db"
severity = "warning"
description = "API must not access DB directly"

[rules.tag_boundary]
from_tag = "api"
deny_tags = ["db"]
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::TagBoundary(t) => {
                assert_eq!(t.from_tag, "api");
                assert_eq!(t.deny_tags, vec!["db"]);
                assert!(t.except_tags.is_none());
            }
            other => panic!("Expected TagBoundary rule, got {:?}", other),
        }
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

    // =========================================================================
    // Phase 2b rule config parsing
    // =========================================================================

    #[test]
    fn test_parse_cycle_policy_rule() {
        let toml = r#"
[[rules]]
id = "no-cycles"
severity = "error"
description = "No cycles allowed"

[rules.cycle_policy]
max_cycle_length = 0
pattern = ["src/**"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::CyclePolicy(c) => {
                assert_eq!(c.max_cycle_length, 0);
                assert_eq!(
                    c.pattern.as_deref(),
                    Some(vec!["src/**".to_string()].as_slice())
                );
            }
            other => panic!("Expected CyclePolicy rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_cycle_policy_no_pattern() {
        let toml = r#"
[[rules]]
id = "no-cycles"
severity = "error"
description = "No cycles allowed"

[rules.cycle_policy]
max_cycle_length = 3
"#;

        let config = parse_config(toml).unwrap();
        match &config.rules[0].rule {
            RuleKind::CyclePolicy(c) => {
                assert_eq!(c.max_cycle_length, 3);
                assert!(c.pattern.is_none());
            }
            other => panic!("Expected CyclePolicy rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stability_limit_rule() {
        let toml = r#"
[[rules]]
id = "stable-model"
severity = "warning"
description = "Model layer must be stable"

[rules.stability_limit]
pattern = ["src/model/**"]
max_instability = 0.3
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::StabilityLimit(s) => {
                assert_eq!(s.pattern, vec!["src/model/**"]);
                assert!((s.max_instability - 0.3).abs() < f64::EPSILON);
            }
            other => panic!("Expected StabilityLimit rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_naming_boundary_rule() {
        let toml = r#"
[[rules]]
id = "service-naming"
severity = "warning"
description = "Services must follow naming convention"

[rules.naming_boundary]
pattern = ["src/services/**"]
must_match = ".*Service\\.(ts|rs)$"
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::NamingBoundary(n) => {
                assert_eq!(n.pattern, vec!["src/services/**"]);
                assert_eq!(n.must_match, r".*Service\.(ts|rs)$");
            }
            other => panic!("Expected NamingBoundary rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_restricted_consumer_rule() {
        let toml = r#"
[[rules]]
id = "secrets-restricted"
severity = "error"
description = "Only auth can access secrets"

[rules.restricted_consumer]
target = ["src/core/secrets.ts"]
allowed_consumers = ["src/auth/**"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::RestrictedConsumer(rc) => {
                assert_eq!(rc.target, vec!["src/core/secrets.ts"]);
                assert_eq!(rc.allowed_consumers, vec!["src/auth/**"]);
            }
            other => panic!("Expected RestrictedConsumer rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_export_limit_rule() {
        let toml = r#"
[[rules]]
id = "export-limit"
severity = "warning"
description = "Too many exports"

[rules.export_limit]
pattern = ["src/**"]
max_exports = 10
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::ExportLimit(el) => {
                assert_eq!(el.pattern, vec!["src/**"]);
                assert_eq!(el.max_exports, 10);
            }
            other => panic!("Expected ExportLimit rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_coupling_weight_rule() {
        let toml = r#"
[[rules]]
id = "coupling"
severity = "warning"
description = "Too tightly coupled"

[rules.coupling_weight]
pattern = ["src/**"]
max_names_per_edge = 5
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::CouplingWeight(cw) => {
                assert_eq!(cw.pattern, vec!["src/**"]);
                assert_eq!(cw.max_names_per_edge, 5);
            }
            other => panic!("Expected CouplingWeight rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_cohesion_rule() {
        let toml = r#"
[[rules]]
id = "cohesion"
severity = "warning"
description = "Low module cohesion"

[rules.cohesion]
pattern = ["src/modules/**"]
max_external_ratio = 0.6
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0].rule {
            RuleKind::Cohesion(c) => {
                assert_eq!(c.pattern, vec!["src/modules/**"]);
                assert!((c.max_external_ratio - 0.6).abs() < f64::EPSILON);
            }
            other => panic!("Expected Cohesion rule, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_config_with_only_entry_points_no_rules() {
        let toml = r#"
[entry_points]
patterns = ["**/Bootstrap.java"]
annotations = ["Scheduled"]
"#;

        let config = parse_config(toml).unwrap();
        assert!(config.rules.is_empty());
    }

    // =========================================================================
    // Scope config
    // =========================================================================

    #[test]
    fn test_parse_scope_config() {
        let toml = r#"
[scope.production]
include = ["src/main/java/**", "src/**/*.rs"]
exclude = ["src/**/test/**"]

[scope.test]
include = ["src/test/**", "tests/**"]
role = "entry_point"
lint = false

[scope.fixture]
include = ["test-fixtures/**"]
role = "entry_point"
analysis = false
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.scope.len(), 3);

        let prod = &config.scope["production"];
        assert_eq!(prod.include, vec!["src/main/java/**", "src/**/*.rs"]);
        assert_eq!(prod.exclude, vec!["src/**/test/**"]);
        assert!(prod.role.is_none());
        assert!(prod.lint);
        assert!(prod.analysis);

        let test = &config.scope["test"];
        assert_eq!(test.include, vec!["src/test/**", "tests/**"]);
        assert!(test.exclude.is_empty());
        assert_eq!(test.role.as_deref(), Some("entry_point"));
        assert!(!test.lint);
        assert!(test.analysis);

        let fixture = &config.scope["fixture"];
        assert_eq!(fixture.include, vec!["test-fixtures/**"]);
        assert_eq!(fixture.role.as_deref(), Some("entry_point"));
        assert!(fixture.lint);
        assert!(!fixture.analysis);
    }

    #[test]
    fn test_parse_scope_config_defaults() {
        let toml = r#"
[scope.production]
include = ["src/**"]
"#;

        let config = parse_config(toml).unwrap();
        let prod = &config.scope["production"];
        assert_eq!(prod.include, vec!["src/**"]);
        assert!(prod.exclude.is_empty());
        assert!(prod.role.is_none());
        assert!(prod.lint);
        assert!(prod.analysis);
    }

    #[test]
    fn test_parse_scope_config_empty() {
        let toml = r#"
rules = []
"#;

        let config = parse_config(toml).unwrap();
        assert!(config.scope.is_empty());
    }

    #[test]
    fn test_parse_scope_with_rules() {
        let toml = r#"
[scope.production]
include = ["src/**"]

[[rules]]
id = "test"
severity = "error"
description = "test rule"

[rules.boundary]
from = ["src/a/**"]
deny = ["src/b/**"]
"#;

        let config = parse_config(toml).unwrap();
        assert_eq!(config.scope.len(), 1);
        assert_eq!(config.rules.len(), 1);
    }

    #[test]
    fn test_parse_scope_missing_include() {
        let toml = r#"
[scope.bad]
role = "entry_point"
"#;

        let result = parse_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_scope_index_classify() {
        let mut scope = HashMap::new();
        scope.insert(
            "production".to_string(),
            ScopeSetConfig {
                include: vec!["src/main/**".to_string()],
                exclude: vec![],
                role: None,
                lint: true,
                analysis: true,
            },
        );
        scope.insert(
            "test".to_string(),
            ScopeSetConfig {
                include: vec!["src/test/**".to_string(), "tests/**".to_string()],
                exclude: vec![],
                role: Some("entry_point".to_string()),
                lint: false,
                analysis: true,
            },
        );

        let index = ScopeIndex::build(&scope).unwrap();

        assert_eq!(
            index.classify(Path::new("src/main/java/Foo.java")),
            Some("production")
        );
        assert_eq!(
            index.classify(Path::new("src/test/java/FooTest.java")),
            Some("test")
        );
        assert_eq!(
            index.classify(Path::new("tests/integration.rs")),
            Some("test")
        );
        assert_eq!(index.classify(Path::new("other/file.txt")), None);
    }

    #[test]
    fn test_scope_index_classify_with_exclude() {
        let mut scope = HashMap::new();
        scope.insert(
            "production".to_string(),
            ScopeSetConfig {
                include: vec!["src/**".to_string()],
                exclude: vec!["src/test/**".to_string()],
                role: None,
                lint: true,
                analysis: true,
            },
        );
        scope.insert(
            "test".to_string(),
            ScopeSetConfig {
                include: vec!["src/test/**".to_string()],
                exclude: vec![],
                role: Some("entry_point".to_string()),
                lint: false,
                analysis: true,
            },
        );

        let index = ScopeIndex::build(&scope).unwrap();

        assert_eq!(
            index.classify(Path::new("src/main/Foo.java")),
            Some("production")
        );
        // src/test/** is excluded from production, so it falls through to "test"
        assert_eq!(
            index.classify(Path::new("src/test/FooTest.java")),
            Some("test")
        );
    }

    #[test]
    fn test_scope_index_lint_and_analysis() {
        let mut scope = HashMap::new();
        scope.insert(
            "production".to_string(),
            ScopeSetConfig {
                include: vec!["src/**".to_string()],
                exclude: vec![],
                role: None,
                lint: true,
                analysis: true,
            },
        );
        scope.insert(
            "test".to_string(),
            ScopeSetConfig {
                include: vec!["tests/**".to_string()],
                exclude: vec![],
                role: Some("entry_point".to_string()),
                lint: false,
                analysis: true,
            },
        );
        scope.insert(
            "fixture".to_string(),
            ScopeSetConfig {
                include: vec!["fixtures/**".to_string()],
                exclude: vec![],
                role: Some("entry_point".to_string()),
                lint: true,
                analysis: false,
            },
        );

        let index = ScopeIndex::build(&scope).unwrap();

        assert!(index.lint_enabled("production"));
        assert!(!index.lint_enabled("test"));
        assert!(index.lint_enabled("fixture"));
        assert!(index.lint_enabled("nonexistent")); // default true

        assert!(index.analysis_enabled("production"));
        assert!(index.analysis_enabled("test"));
        assert!(!index.analysis_enabled("fixture"));
        assert!(index.analysis_enabled("nonexistent")); // default true
    }

    #[test]
    fn test_scope_index_has_role() {
        let mut scope = HashMap::new();
        scope.insert(
            "production".to_string(),
            ScopeSetConfig {
                include: vec!["src/**".to_string()],
                exclude: vec![],
                role: None,
                lint: true,
                analysis: true,
            },
        );
        scope.insert(
            "test".to_string(),
            ScopeSetConfig {
                include: vec!["tests/**".to_string()],
                exclude: vec![],
                role: Some("entry_point".to_string()),
                lint: false,
                analysis: true,
            },
        );

        let index = ScopeIndex::build(&scope).unwrap();

        assert!(!index.has_role("production", "entry_point"));
        assert!(index.has_role("test", "entry_point"));
        assert!(!index.has_role("nonexistent", "entry_point"));
    }

    #[test]
    fn test_scope_index_empty() {
        let scope = HashMap::new();
        let index = ScopeIndex::build(&scope).unwrap();
        assert!(index.is_empty());
        assert_eq!(index.classify(Path::new("any/file.rs")), None);
    }

    #[test]
    fn test_scope_set_config_has_role() {
        let config = ScopeSetConfig {
            include: vec!["tests/**".to_string()],
            exclude: vec![],
            role: Some("entry_point".to_string()),
            lint: true,
            analysis: true,
        };
        assert!(config.has_role("entry_point"));
        assert!(!config.has_role("other"));

        let no_role = ScopeSetConfig {
            include: vec!["src/**".to_string()],
            exclude: vec![],
            role: None,
            lint: true,
            analysis: true,
        };
        assert!(!no_role.has_role("entry_point"));
    }

    #[test]
    fn test_load_scope_config_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let statik_dir = dir.path().join(".statik");
        std::fs::create_dir_all(&statik_dir).unwrap();
        std::fs::write(
            statik_dir.join("rules.toml"),
            r#"
[scope.production]
include = ["src/**"]

[scope.test]
include = ["tests/**"]
role = "entry_point"
lint = false
"#,
        )
        .unwrap();

        let scope = load_scope_config(dir.path());
        assert_eq!(scope.len(), 2);
        assert!(scope.contains_key("production"));
        assert!(scope.contains_key("test"));
        assert!(!scope["test"].lint);
    }

    #[test]
    fn test_load_scope_config_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = load_scope_config(dir.path());
        assert!(scope.is_empty());
    }
}
