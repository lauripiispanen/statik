use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::rules::LintViolation;

const BASELINE_PATH: &str = ".statik/lint-baseline.json";

/// A single baseline entry identifying a known violation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub rule_id: String,
    pub source_file: PathBuf,
    pub target_file: PathBuf,
    pub line: usize,
}

/// Stored baseline of known lint violations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    pub created: String,
    pub violations: Vec<BaselineEntry>,
}

fn now_iso8601() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple UTC timestamp without external dependency
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01
    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    for md in &month_days {
        if remaining_days < *md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        remaining_days + 1,
        hours,
        minutes,
        seconds,
    )
}

impl Baseline {
    /// Load a baseline from the project's `.statik/lint-baseline.json`.
    /// Returns `None` if the file does not exist.
    pub fn load(project_root: &Path) -> Result<Option<Self>> {
        let path = project_root.join(BASELINE_PATH);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let baseline: Baseline = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(Some(baseline))
    }

    /// Save this baseline to the project's `.statik/lint-baseline.json`.
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join(BASELINE_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Create a baseline from a list of violations.
    pub fn from_violations(violations: &[LintViolation]) -> Self {
        let entries: Vec<BaselineEntry> = violations
            .iter()
            .map(|v| BaselineEntry {
                rule_id: v.rule_id.clone(),
                source_file: v.source_file.clone(),
                target_file: v.target_file.clone(),
                line: v.line,
            })
            .collect();
        Baseline {
            version: 1,
            created: now_iso8601(),
            violations: entries,
        }
    }

    /// Check if a violation is known in this baseline.
    pub fn is_known(&self, entry: &BaselineEntry) -> bool {
        self.as_set().contains(entry)
    }

    fn as_set(&self) -> HashSet<&BaselineEntry> {
        self.violations.iter().collect()
    }

    /// Filter out violations that are present in this baseline.
    /// Returns only the new/unknown violations.
    pub fn filter_known(&self, violations: Vec<LintViolation>) -> Vec<LintViolation> {
        let known = self.as_set();
        violations
            .into_iter()
            .filter(|v| {
                let entry = BaselineEntry {
                    rule_id: v.rule_id.clone(),
                    source_file: v.source_file.clone(),
                    target_file: v.target_file.clone(),
                    line: v.line,
                };
                !known.contains(&entry)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Confidence;
    use crate::linting::config::Severity;

    fn make_violation(rule_id: &str, source: &str, target: &str, line: usize) -> LintViolation {
        LintViolation {
            rule_id: rule_id.to_string(),
            severity: Severity::Error,
            description: "test".to_string(),
            rationale: None,
            source_file: PathBuf::from(source),
            target_file: PathBuf::from(target),
            imported_names: vec![],
            line,
            confidence: Confidence::Certain,
            fix_direction: None,
        }
    }

    #[test]
    fn test_from_violations_and_filter() {
        let violations = vec![
            make_violation("rule-1", "src/a.ts", "src/b.ts", 5),
            make_violation("rule-2", "src/c.ts", "src/d.ts", 10),
        ];

        let baseline = Baseline::from_violations(&violations);
        assert_eq!(baseline.version, 1);
        assert_eq!(baseline.violations.len(), 2);

        // Same violations should be filtered out
        let filtered = baseline.filter_known(violations.clone());
        assert!(filtered.is_empty());

        // A new violation should pass through
        let new_violations = vec![
            make_violation("rule-1", "src/a.ts", "src/b.ts", 5), // known
            make_violation("rule-3", "src/e.ts", "src/f.ts", 1), // new
        ];
        let filtered = baseline.filter_known(new_violations);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule_id, "rule-3");
    }

    #[test]
    fn test_is_known() {
        let violations = vec![make_violation("rule-1", "src/a.ts", "src/b.ts", 5)];
        let baseline = Baseline::from_violations(&violations);

        let known = BaselineEntry {
            rule_id: "rule-1".to_string(),
            source_file: PathBuf::from("src/a.ts"),
            target_file: PathBuf::from("src/b.ts"),
            line: 5,
        };
        assert!(baseline.is_known(&known));

        let unknown = BaselineEntry {
            rule_id: "rule-1".to_string(),
            source_file: PathBuf::from("src/a.ts"),
            target_file: PathBuf::from("src/b.ts"),
            line: 6, // different line
        };
        assert!(!baseline.is_known(&unknown));
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let statik_dir = dir.path().join(".statik");
        std::fs::create_dir_all(&statik_dir).unwrap();

        let violations = vec![
            make_violation("rule-1", "src/a.ts", "src/b.ts", 5),
            make_violation("rule-2", "src/c.ts", "src/d.ts", 10),
        ];

        let baseline = Baseline::from_violations(&violations);
        baseline.save(dir.path()).unwrap();

        let loaded = Baseline::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.violations.len(), 2);
        assert_eq!(loaded.violations[0].rule_id, "rule-1");
        assert_eq!(loaded.violations[1].rule_id, "rule-2");
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = Baseline::load(dir.path()).unwrap();
        assert!(result.is_none());
    }
}
