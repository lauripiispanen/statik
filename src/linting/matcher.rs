use std::path::Path;

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// Matches file paths against a set of glob patterns.
///
/// Patterns prefixed with `!` are exclusions. A path matches if it
/// matches any include pattern and does not match any exclude pattern.
pub struct FileMatcher {
    include: GlobSet,
    exclude: GlobSet,
}

impl FileMatcher {
    /// Create a new matcher from a list of glob patterns.
    ///
    /// Patterns starting with `!` are treated as exclusions.
    /// An empty pattern list matches nothing.
    pub fn new(patterns: &[String]) -> Result<Self> {
        let mut include_builder = GlobSetBuilder::new();
        let mut exclude_builder = GlobSetBuilder::new();

        for pattern in patterns {
            if let Some(negated) = pattern.strip_prefix('!') {
                exclude_builder.add(Glob::new(negated)?);
            } else {
                include_builder.add(Glob::new(pattern)?);
            }
        }

        Ok(Self {
            include: include_builder.build()?,
            exclude: exclude_builder.build()?,
        })
    }

    /// Check if a project-relative path matches this matcher.
    pub fn matches(&self, path: &Path) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

/// Convert an absolute path to a project-relative path.
pub fn to_relative<'a>(path: &'a Path, project_root: &Path) -> &'a Path {
    path.strip_prefix(project_root).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_basic_glob_match() {
        let matcher = FileMatcher::new(&["src/ui/**".to_string()]).unwrap();
        assert!(matcher.matches(Path::new("src/ui/Button.ts")));
        assert!(matcher.matches(Path::new("src/ui/components/Header.tsx")));
        assert!(!matcher.matches(Path::new("src/db/connection.ts")));
    }

    #[test]
    fn test_multiple_patterns() {
        let matcher =
            FileMatcher::new(&["src/ui/**".to_string(), "src/components/**".to_string()]).unwrap();
        assert!(matcher.matches(Path::new("src/ui/Button.ts")));
        assert!(matcher.matches(Path::new("src/components/Header.tsx")));
        assert!(!matcher.matches(Path::new("src/services/api.ts")));
    }

    #[test]
    fn test_negation_pattern() {
        let matcher =
            FileMatcher::new(&["src/ui/**".to_string(), "!src/ui/shared/**".to_string()]).unwrap();
        assert!(matcher.matches(Path::new("src/ui/Button.ts")));
        assert!(!matcher.matches(Path::new("src/ui/shared/types.ts")));
    }

    #[test]
    fn test_empty_patterns_match_nothing() {
        let matcher = FileMatcher::new(&[]).unwrap();
        assert!(!matcher.matches(Path::new("src/anything.ts")));
    }

    #[test]
    fn test_single_file_pattern() {
        let matcher = FileMatcher::new(&["src/db/schema.ts".to_string()]).unwrap();
        assert!(matcher.matches(Path::new("src/db/schema.ts")));
        assert!(!matcher.matches(Path::new("src/db/connection.ts")));
    }

    #[test]
    fn test_star_pattern() {
        // In globset, `*` matches path separators by default
        let matcher = FileMatcher::new(&["src/*.ts".to_string()]).unwrap();
        assert!(matcher.matches(Path::new("src/index.ts")));
        assert!(matcher.matches(Path::new("src/utils/helper.ts")));
        assert!(!matcher.matches(Path::new("lib/index.ts")));
    }

    #[test]
    fn test_to_relative() {
        let root = PathBuf::from("/home/user/project");
        let abs = PathBuf::from("/home/user/project/src/index.ts");
        assert_eq!(to_relative(&abs, &root), Path::new("src/index.ts"));
    }

    #[test]
    fn test_to_relative_non_child() {
        let root = PathBuf::from("/home/user/project");
        let abs = PathBuf::from("/other/path/file.ts");
        // Should return the original path if not under root
        assert_eq!(to_relative(&abs, &root), Path::new("/other/path/file.ts"));
    }

    #[test]
    fn test_only_exclude_patterns_match_nothing() {
        // If there are no include patterns, nothing matches
        let matcher = FileMatcher::new(&["!src/ui/**".to_string()]).unwrap();
        assert!(!matcher.matches(Path::new("src/db/connection.ts")));
        assert!(!matcher.matches(Path::new("src/ui/Button.ts")));
    }
}
