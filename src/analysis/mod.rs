pub mod churn;
pub mod cycles;
pub mod dead_code;
pub mod dependencies;
pub mod diff;
pub mod impact;
pub mod linker;
pub mod ownership;
pub mod who;

use serde::{Deserialize, Serialize};

/// Confidence level for analysis results.
///
/// All analysis results include a confidence field to communicate
/// how reliable the finding is. This is critical for preventing
/// false positives -- when confidence is low, we say so rather
/// than asserting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
    Certain,
}

impl Confidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Confidence::Certain => "certain",
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A limitation that reduces analysis accuracy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limitation {
    pub description: String,
    pub count: usize,
}

/// Check if a language supports module-level imports where importing a name
/// that matches the file stem implicitly imports all of the module's exports.
///
/// Example: Rust's `use crate::cli::commands` imports the module name "commands"
/// but the actual exports are individual symbols like "run_deps", "build_file_graph".
pub fn supports_module_stem_import(lang: crate::model::Language) -> bool {
    matches!(lang, crate::model::Language::Rust)
    // Go and Python will be added here when their parsers are implemented
}

/// Compute overall confidence from the state of the graph.
pub fn compute_confidence(
    total_imports: usize,
    unresolved_count: usize,
    has_wildcards: bool,
) -> Confidence {
    if unresolved_count == 0 && !has_wildcards {
        Confidence::Certain
    } else if unresolved_count == 0 && has_wildcards {
        Confidence::High
    } else {
        let ratio = unresolved_count as f64 / total_imports.max(1) as f64;
        if ratio < 0.1 {
            Confidence::High
        } else if ratio < 0.3 {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }
}
