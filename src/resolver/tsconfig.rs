use std::path::{Path, PathBuf};

use anyhow::Result;

/// Parsed tsconfig.json relevant to import resolution.
#[derive(Debug, Clone)]
pub struct TsConfig {
    /// The directory containing the tsconfig.json file.
    pub config_dir: PathBuf,
    /// The baseUrl for non-relative module resolution.
    pub base_url: Option<PathBuf>,
    /// Path alias mappings from compilerOptions.paths.
    /// Key is the pattern (e.g. "@utils/*"), value is the list of substitutions.
    pub paths: Vec<PathMapping>,
}

/// A single path mapping from tsconfig.json paths.
#[derive(Debug, Clone)]
pub struct PathMapping {
    /// The prefix before the wildcard, e.g. "@utils/" for "@utils/*"
    pub prefix: String,
    /// The suffix after the wildcard (usually empty)
    pub suffix: String,
    /// The substitution paths, resolved to absolute paths.
    /// For "@utils/*": ["src/utils/*"], the substitution would be
    /// the base_url-resolved path with the wildcard portion.
    pub substitutions: Vec<PathSubstitution>,
}

/// A single substitution target for a path mapping.
#[derive(Debug, Clone)]
pub struct PathSubstitution {
    /// The directory prefix before the wildcard, resolved to absolute path.
    pub prefix: PathBuf,
    /// The suffix after the wildcard (usually empty).
    pub suffix: String,
}

impl TsConfig {
    /// Parse a tsconfig.json file for import resolution settings.
    pub fn parse(tsconfig_path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(tsconfig_path)?;
        Self::parse_from_str(&content, tsconfig_path)
    }

    /// Parse tsconfig.json content from a string.
    /// `tsconfig_path` is used to resolve relative paths.
    pub fn parse_from_str(content: &str, tsconfig_path: &Path) -> Result<Self> {
        let config_dir = tsconfig_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        let json: serde_json::Value = serde_json::from_str(content)?;

        let compiler_options = json.get("compilerOptions");

        let base_url = compiler_options
            .and_then(|co| co.get("baseUrl"))
            .and_then(|v| v.as_str())
            .map(|url| config_dir.join(url));

        let paths = Self::parse_paths(compiler_options, &config_dir, base_url.as_deref());

        Ok(TsConfig {
            config_dir,
            base_url,
            paths,
        })
    }

    fn parse_paths(
        compiler_options: Option<&serde_json::Value>,
        config_dir: &Path,
        base_url: Option<&Path>,
    ) -> Vec<PathMapping> {
        let paths_obj = match compiler_options
            .and_then(|co| co.get("paths"))
            .and_then(|p| p.as_object())
        {
            Some(obj) => obj,
            None => return Vec::new(),
        };

        // The base for path resolution: baseUrl if set, otherwise config_dir
        let resolution_base = base_url.unwrap_or(config_dir);

        let mut mappings = Vec::new();

        for (pattern, targets) in paths_obj {
            let targets = match targets.as_array() {
                Some(arr) => arr,
                None => continue,
            };

            // Split pattern on "*"
            let (prefix, suffix) = split_on_wildcard(pattern);

            let substitutions: Vec<PathSubstitution> = targets
                .iter()
                .filter_map(|t| t.as_str())
                .map(|target| {
                    let (t_prefix, t_suffix) = split_on_wildcard(target);
                    PathSubstitution {
                        prefix: resolution_base.join(t_prefix),
                        suffix: t_suffix.to_string(),
                    }
                })
                .collect();

            mappings.push(PathMapping {
                prefix: prefix.to_string(),
                suffix: suffix.to_string(),
                substitutions,
            });
        }

        // Sort by prefix length descending for most-specific-first matching
        mappings.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));

        mappings
    }

    /// Try to resolve an import path using tsconfig paths.
    /// Returns a list of candidate paths to try (not yet checked for file existence).
    pub fn resolve_path_alias(&self, import_path: &str) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        for mapping in &self.paths {
            if let Some(matched_wildcard) =
                match_pattern(import_path, &mapping.prefix, &mapping.suffix)
            {
                for sub in &mapping.substitutions {
                    let resolved = sub
                        .prefix
                        .join(format!("{}{}", matched_wildcard, sub.suffix));
                    candidates.push(resolved);
                }
            }
        }

        // Also try baseUrl resolution for non-relative, non-scoped paths
        if candidates.is_empty() {
            if let Some(ref base_url) = self.base_url {
                if !import_path.starts_with('.') && !import_path.starts_with('/') {
                    candidates.push(base_url.join(import_path));
                }
            }
        }

        candidates
    }
}

/// Split a pattern string on the first "*" wildcard.
/// Returns (prefix, suffix). If no wildcard, the entire string is the prefix.
fn split_on_wildcard(pattern: &str) -> (&str, &str) {
    match pattern.find('*') {
        Some(pos) => (&pattern[..pos], &pattern[pos + 1..]),
        None => (pattern, ""),
    }
}

/// Match an import path against a tsconfig path pattern.
/// Returns the portion matched by the wildcard, or None if no match.
fn match_pattern<'a>(import_path: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    if suffix.is_empty() {
        // Pattern like "@utils/*" -> prefix is "@utils/"
        if let Some(rest) = import_path.strip_prefix(prefix) {
            Some(rest)
        } else {
            None
        }
    } else {
        // Pattern like "@utils/*.js" -> prefix is "@utils/", suffix is ".js"
        if import_path.starts_with(prefix) && import_path.ends_with(suffix) {
            let wildcard_end = import_path.len() - suffix.len();
            if prefix.len() <= wildcard_end {
                Some(&import_path[prefix.len()..wildcard_end])
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_tsconfig() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@utils/*": ["src/utils/*"],
                    "@models/*": ["src/models/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        assert_eq!(config.base_url, Some(PathBuf::from("/project")));
        assert_eq!(config.paths.len(), 2);
    }

    #[test]
    fn test_parse_tsconfig_without_paths() {
        let content = r#"{
            "compilerOptions": {
                "target": "ES2020",
                "strict": true
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        assert!(config.paths.is_empty());
        assert!(config.base_url.is_none());
    }

    #[test]
    fn test_parse_tsconfig_with_base_url() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": "./src"
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        assert_eq!(config.base_url, Some(PathBuf::from("/project/src")));
    }

    #[test]
    fn test_resolve_path_alias_simple() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@utils/*": ["src/utils/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        let candidates = config.resolve_path_alias("@utils/format");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], PathBuf::from("/project/src/utils/format"));
    }

    #[test]
    fn test_resolve_path_alias_multiple_targets() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@/*": ["src/*", "lib/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        let candidates = config.resolve_path_alias("@/components/Button");
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0],
            PathBuf::from("/project/src/components/Button")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/project/lib/components/Button")
        );
    }

    #[test]
    fn test_resolve_unmatched_alias() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@utils/*": ["src/utils/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        let candidates = config.resolve_path_alias("@models/user");
        // Falls back to baseUrl
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], PathBuf::from("/project/@models/user"));
    }

    #[test]
    fn test_resolve_relative_path_not_aliased() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@utils/*": ["src/utils/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        let candidates = config.resolve_path_alias("./utils/format");
        // Relative paths should not be aliased
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_split_on_wildcard() {
        assert_eq!(split_on_wildcard("@utils/*"), ("@utils/", ""));
        assert_eq!(split_on_wildcard("@/*"), ("@/", ""));
        assert_eq!(split_on_wildcard("src/*"), ("src/", ""));
        assert_eq!(split_on_wildcard("exact-match"), ("exact-match", ""));
    }

    #[test]
    fn test_match_pattern() {
        assert_eq!(
            match_pattern("@utils/format", "@utils/", ""),
            Some("format")
        );
        assert_eq!(
            match_pattern("@utils/deep/path", "@utils/", ""),
            Some("deep/path")
        );
        assert_eq!(match_pattern("@models/user", "@utils/", ""), None);
        assert_eq!(
            match_pattern("@/components/Button", "@/", ""),
            Some("components/Button")
        );
    }

    #[test]
    fn test_exact_path_mapping() {
        let content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "config": ["src/config/index"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        let candidates = config.resolve_path_alias("config");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], PathBuf::from("/project/src/config/index"));
    }

    #[test]
    fn test_paths_without_base_url() {
        // TypeScript allows paths without baseUrl in newer versions
        let content = r#"{
            "compilerOptions": {
                "paths": {
                    "@utils/*": ["src/utils/*"]
                }
            }
        }"#;

        let config =
            TsConfig::parse_from_str(content, Path::new("/project/tsconfig.json")).unwrap();
        assert!(config.base_url.is_none());
        let candidates = config.resolve_path_alias("@utils/format");
        assert_eq!(candidates.len(), 1);
        // Without baseUrl, paths are resolved relative to tsconfig directory
        assert_eq!(candidates[0], PathBuf::from("/project/src/utils/format"));
    }
}
