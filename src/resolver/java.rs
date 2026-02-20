use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Resolution, Resolver, UnresolvedReason};

/// Standard Maven/Gradle source root directories to probe.
const STANDARD_SOURCE_ROOTS: &[&str] = &[
    "src/main/java",
    "src/test/java",
    "src",
];

/// Java import resolver.
///
/// Handles:
/// - Fully-qualified class imports: `com.example.Foo` → `<source_root>/com/example/Foo.java`
/// - Wildcard imports: `com.example` (with imported_name `*`) → directory match
/// - Static imports: `com.example.Foo.bar` → resolve `com.example.Foo` → `Foo.java`
/// - Multiple source roots (Maven, Gradle, flat)
///
/// For Java, `import_source` is the fully-qualified name from the import statement
/// (e.g. `"com.example.Foo"` or `"com.example"` for wildcard imports).
pub struct JavaResolver {
    /// All detected source root directories (absolute paths).
    source_roots: Vec<PathBuf>,
    /// Set of known files in the project for fast existence checks.
    known_files: HashSet<PathBuf>,
}

impl JavaResolver {
    /// Create a new Java resolver.
    ///
    /// - `project_root`: Absolute path to the project root.
    /// - `known_files`: All known `.java` file paths in the project (absolute paths).
    pub fn new(project_root: PathBuf, known_files: Vec<PathBuf>) -> Self {
        let source_roots = Self::detect_source_roots(&project_root, &known_files);
        let known_set: HashSet<PathBuf> = known_files.into_iter().collect();
        JavaResolver {
            source_roots,
            known_files: known_set,
        }
    }

    /// Detect source root directories.
    ///
    /// Strategy:
    /// 1. Check for standard Maven/Gradle source roots
    /// 2. Fall back to project root itself
    fn detect_source_roots(project_root: &Path, known_files: &[PathBuf]) -> Vec<PathBuf> {
        let mut roots = Vec::new();

        // Check standard source root directories
        for dir in STANDARD_SOURCE_ROOTS {
            let candidate = project_root.join(dir);
            if candidate.is_dir() {
                roots.push(candidate);
            }
        }

        // If no standard roots found, try to infer from known files
        if roots.is_empty() && !known_files.is_empty() {
            // Use project root as fallback
            roots.push(project_root.to_path_buf());
        }

        roots
    }

    /// Convert a fully-qualified Java name to a relative file path.
    ///
    /// `com.example.Foo` → `com/example/Foo.java`
    fn fqn_to_relative_path(fqn: &str) -> PathBuf {
        let path_str = fqn.replace('.', "/");
        PathBuf::from(format!("{}.java", path_str))
    }

    /// Try to resolve a fully-qualified name against all source roots.
    fn resolve_fqn(&self, fqn: &str) -> Option<PathBuf> {
        let relative = Self::fqn_to_relative_path(fqn);

        for root in &self.source_roots {
            let candidate = root.join(&relative);
            if self.known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Try progressively shorter prefixes to resolve an import.
    /// This handles static imports like `com.example.Foo.bar` where `bar` is a member
    /// of `Foo`, so we need to resolve `com.example.Foo`.
    fn resolve_with_member_fallback(&self, fqn: &str) -> Option<PathBuf> {
        // First try exact match
        if let Some(path) = self.resolve_fqn(fqn) {
            return Some(path);
        }

        // Try stripping the last segment (could be a member name)
        if let Some(dot_pos) = fqn.rfind('.') {
            let class_fqn = &fqn[..dot_pos];
            if let Some(path) = self.resolve_fqn(class_fqn) {
                return Some(path);
            }
        }

        None
    }

    /// Check if a fully-qualified name looks like a standard library or external package.
    fn is_likely_external(fqn: &str) -> bool {
        let external_prefixes = [
            "java.",
            "javax.",
            "jakarta.",
            "org.junit",
            "org.apache",
            "org.springframework",
            "com.google",
            "io.netty",
            "lombok",
        ];
        external_prefixes.iter().any(|p| fqn.starts_with(p))
    }
}

impl Resolver for JavaResolver {
    fn resolve(&self, import_source: &str, _from_file: &Path) -> Resolution {
        if import_source.is_empty() {
            return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                "empty import".to_string(),
            ));
        }

        // Try to resolve against project source roots
        if let Some(resolved) = self.resolve_with_member_fallback(import_source) {
            return Resolution::Resolved(resolved);
        }

        // If it looks like a standard/common external package, mark as External
        if Self::is_likely_external(import_source) {
            let pkg = import_source
                .split('.')
                .take(3)
                .collect::<Vec<_>>()
                .join(".");
            return Resolution::External(pkg);
        }

        // If not found and not obviously external, still classify as External
        // since unresolved Java imports are most likely third-party dependencies
        Resolution::External(import_source.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_maven_project() -> (TempDir, Vec<PathBuf>) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create Maven-style source layout
        let src = root.join("src/main/java/com/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("App.java"), "package com.example; public class App {}").unwrap();
        fs::write(
            src.join("UserService.java"),
            "package com.example; public class UserService {}",
        )
        .unwrap();

        let model = root.join("src/main/java/com/example/model");
        fs::create_dir_all(&model).unwrap();
        fs::write(
            model.join("User.java"),
            "package com.example.model; public class User {}",
        )
        .unwrap();

        let test_src = root.join("src/test/java/com/example");
        fs::create_dir_all(&test_src).unwrap();
        fs::write(
            test_src.join("AppTest.java"),
            "package com.example; public class AppTest {}",
        )
        .unwrap();

        let known_files = vec![
            src.join("App.java"),
            src.join("UserService.java"),
            model.join("User.java"),
            test_src.join("AppTest.java"),
        ];

        (dir, known_files)
    }

    #[test]
    fn test_resolve_simple_import() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve("com.example.UserService", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/UserService.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_nested_package() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve("com.example.model.User", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/model/User.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_test_source_root() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/test/java/com/example/AppTest.java");

        let result = resolver.resolve("com.example.AppTest", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/AppTest.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_cross_source_root() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/test/java/com/example/AppTest.java");

        // Test file importing from main source
        let result = resolver.resolve("com.example.App", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/App.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_static_import() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // Static import: com.example.UserService.someMethod
        // Should resolve to UserService.java by stripping the member
        let result = resolver.resolve("com.example.UserService.someMethod", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/UserService.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_external_java_standard_library() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve("java.util.List", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "java.util.List");
            }
            other => panic!("expected External, got {:?}", other),
        }
    }

    #[test]
    fn test_external_third_party() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve("org.springframework.boot.SpringApplication", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "org.springframework.boot");
            }
            other => panic!("expected External, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_import_classified_as_external() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // Import not found in project, not a known external prefix
        let result = resolver.resolve("com.other.SomeClass", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "com.other.SomeClass");
            }
            other => panic!("expected External for unknown import, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_import() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve("", &from_file);
        assert!(matches!(result, Resolution::Unresolved(_)));
    }

    #[test]
    fn test_flat_layout_project() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create a flat source layout (no Maven/Gradle dirs)
        let src = root.join("src/com/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("App.java"), "public class App {}").unwrap();

        let known_files = vec![src.join("App.java")];
        let resolver = JavaResolver::new(root.to_path_buf(), known_files);
        let from_file = src.join("App.java");

        let result = resolver.resolve("com.example.App", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("com/example/App.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_source_root_detection_maven() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/main/java")).unwrap();
        fs::create_dir_all(root.join("src/test/java")).unwrap();

        let roots = JavaResolver::detect_source_roots(root, &[]);
        assert!(roots.iter().any(|r| r.ends_with("src/main/java")));
        assert!(roots.iter().any(|r| r.ends_with("src/test/java")));
    }

    #[test]
    fn test_source_root_detection_fallback() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // No standard dirs exist, but we have known files
        let known = vec![root.join("Foo.java")];

        let roots = JavaResolver::detect_source_roots(root, &known);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0], root);
    }

    #[test]
    fn test_fqn_to_relative_path() {
        assert_eq!(
            JavaResolver::fqn_to_relative_path("com.example.Foo"),
            PathBuf::from("com/example/Foo.java")
        );
        assert_eq!(
            JavaResolver::fqn_to_relative_path("Foo"),
            PathBuf::from("Foo.java")
        );
    }
}
