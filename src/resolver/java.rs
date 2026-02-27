use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{Resolution, Resolver, UnresolvedReason};

/// Standard Maven/Gradle source root directories to probe.
const STANDARD_SOURCE_ROOTS: &[&str] = &["src/main/java", "src/test/java", "src"];

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
    /// Package name -> list of (class_name, file_path) for same-package resolution.
    package_files: HashMap<String, Vec<(String, PathBuf)>>,
    /// Wildcard import packages for the current file being resolved.
    /// Populated per-file before resolving type-refs.
    file_wildcards: Vec<String>,
    /// Optional source set index for scoping same-package resolution.
    source_set_index: Option<super::source_sets::SourceSetIndex>,
}

impl JavaResolver {
    /// Create a new Java resolver.
    ///
    /// - `project_root`: Absolute path to the project root.
    /// - `known_files`: All known `.java` file paths in the project (absolute paths).
    /// - `configured_roots`: Optional explicit source root paths from config.
    ///   When provided, these are used directly (skip auto-detection).
    pub fn new(
        project_root: PathBuf,
        known_files: Vec<PathBuf>,
        configured_roots: Option<Vec<String>>,
    ) -> Self {
        let source_roots = if let Some(roots) = configured_roots {
            roots
                .iter()
                .map(|r| project_root.join(r))
                .filter(|p| p.is_dir())
                .collect()
        } else {
            Self::detect_source_roots(&project_root, &known_files)
        };
        let package_files = Self::build_package_map(&source_roots, &known_files);
        let known_set: HashSet<PathBuf> = known_files.into_iter().collect();
        JavaResolver {
            source_roots,
            known_files: known_set,
            package_files,
            file_wildcards: Vec::new(),
            source_set_index: None,
        }
    }

    /// Set the source set index for scoping same-package resolution.
    pub fn set_source_set_index(&mut self, index: super::source_sets::SourceSetIndex) {
        self.source_set_index = Some(index);
    }

    /// Detect source root directories.
    ///
    /// Strategy:
    /// 1. Check for standard Maven/Gradle source roots at the project root
    /// 2. Scan known `.java` file paths for standard source root patterns
    ///    in ancestor directories (handles monorepos)
    /// 3. Fall back to project root itself
    fn detect_source_roots(project_root: &Path, known_files: &[PathBuf]) -> Vec<PathBuf> {
        let mut roots = Vec::new();

        // Pass 1: check standard source root dirs at the project root (cheap)
        for dir in STANDARD_SOURCE_ROOTS {
            let candidate = project_root.join(dir);
            if candidate.is_dir() {
                roots.push(candidate);
            }
        }

        if !roots.is_empty() {
            return roots;
        }

        // Pass 2: scan known file paths for standard source root patterns
        // in any ancestor directory (handles monorepos like server/core/src/main/java/)
        let mut seen = HashSet::new();
        for file_path in known_files {
            let ext = file_path.extension().and_then(|e| e.to_str());
            if ext != Some("java") {
                continue;
            }
            if let Some(root) = Self::extract_source_root(file_path) {
                if seen.insert(root.clone()) {
                    roots.push(root);
                }
            }
        }

        // Fall back to project root if nothing found
        if roots.is_empty() && !known_files.is_empty() {
            roots.push(project_root.to_path_buf());
        }

        roots
    }

    /// Given a `.java` file path, find the source root by looking for a standard
    /// source root pattern (e.g. `src/main/java`) in its ancestor path.
    fn extract_source_root(file_path: &Path) -> Option<PathBuf> {
        let path_str = file_path.to_string_lossy();

        // Check for multi-segment patterns first (more specific)
        for pattern in &["src/main/java", "src/test/java"] {
            if let Some(idx) = path_str.find(pattern) {
                let root = &path_str[..idx + pattern.len()];
                return Some(PathBuf::from(root));
            }
        }

        // Then check single-segment patterns
        // "src/java" — non-standard but used in some projects
        if let Some(idx) = path_str.find("src/java/") {
            let root = &path_str[..idx + "src/java".len()];
            return Some(PathBuf::from(root));
        }

        // Bare "src" — only match if followed by a package-like path
        // (i.e., src/com/... or src/org/... to avoid false positives)
        let components: Vec<_> = file_path.components().collect();
        for (i, comp) in components.iter().enumerate() {
            if comp.as_os_str() == "src" && i + 1 < components.len() {
                let next = components[i + 1].as_os_str().to_string_lossy();
                // Only treat bare "src" as source root if next dir looks like a package
                if next.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    let root: PathBuf = components[..=i].iter().collect();
                    return Some(root);
                }
            }
        }

        None
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

    /// Set wildcard import packages for the current file being resolved.
    /// Call before resolving type-refs for a file.
    pub fn set_file_wildcards(&mut self, imports: &[crate::model::ImportRecord]) {
        self.file_wildcards = imports
            .iter()
            .filter(|i| i.is_namespace && !i.source_path.starts_with('@'))
            .filter(|i| Self::is_likely_external(&i.source_path) || i.source_path.starts_with("java."))
            .map(|i| i.source_path.clone())
            .collect();
    }

    pub fn clear_file_wildcards(&mut self) {
        self.file_wildcards.clear();
    }

    pub fn is_likely_external(fqn: &str) -> bool {
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

    fn build_package_map(
        source_roots: &[PathBuf],
        known_files: &[PathBuf],
    ) -> HashMap<String, Vec<(String, PathBuf)>> {
        let mut map: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
        for file_path in known_files {
            let ext = file_path.extension().and_then(|e| e.to_str());
            if ext != Some("java") {
                continue;
            }
            let class_name = match file_path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Infer package from relative path under source root
            if let Some(pkg) = Self::infer_package(file_path, source_roots) {
                map.entry(pkg)
                    .or_default()
                    .push((class_name, file_path.clone()));
            }
        }
        map
    }

    fn infer_package(file_path: &Path, source_roots: &[PathBuf]) -> Option<String> {
        for root in source_roots {
            if let Ok(rel) = file_path.strip_prefix(root) {
                if let Some(parent) = rel.parent() {
                    if parent.as_os_str().is_empty() {
                        return Some(String::new()); // default package
                    }
                    let pkg = parent
                        .components()
                        .map(|c| c.as_os_str().to_str().unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join(".");
                    return Some(pkg);
                }
            }
        }
        None
    }

    pub fn resolve_type_ref(&self, type_name: &str, from_file: &Path) -> Resolution {
        if JAVA_LANG_TYPES.contains(&type_name) {
            return Resolution::External("java.lang".to_string());
        }

        // Determine from_file's package
        let from_pkg = Self::infer_package(from_file, &self.source_roots).unwrap_or_default();

        if let Some(siblings) = self.package_files.get(&from_pkg) {
            // Collect all matches
            let matches: Vec<&PathBuf> = siblings
                .iter()
                .filter(|(class_name, path)| class_name == type_name && path != from_file)
                .map(|(_, path)| path)
                .collect();

            if !matches.is_empty() {
                // If we have a source set index, prefer same source set, then visible deps
                if let Some(ref index) = self.source_set_index {
                    let from_set = index.file_source_set(from_file);
                    // First pass: prefer files in the same source set
                    if let Some(from_set_name) = from_set {
                        for path in &matches {
                            if index.file_source_set(path) == Some(from_set_name) {
                                return Resolution::Resolved((*path).clone());
                            }
                        }
                    }
                    // Second pass: any visible file
                    for path in &matches {
                        if index.can_see(from_file, path) {
                            return Resolution::Resolved((*path).clone());
                        }
                    }
                    // No visible match — fall through (don't resolve to invisible files)
                } else {
                    // No source set index: return first match (original behavior)
                    return Resolution::Resolved(matches[0].clone());
                }
            }
        }

        // If the file has external wildcard imports, this type likely came from one
        if !self.file_wildcards.is_empty() {
            return Resolution::External(self.file_wildcards[0].clone());
        }

        // Not found in same package and no external wildcard imports
        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "type ref '{}' not in same package",
            type_name
        )))
    }

    pub fn resolve_wildcard(&self, package_fqn: &str) -> Vec<PathBuf> {
        if Self::is_likely_external(package_fqn) || package_fqn.starts_with("java.") {
            return Vec::new();
        }
        if let Some(files) = self.package_files.get(package_fqn) {
            return files.iter().map(|(_, path)| path.clone()).collect();
        }
        Vec::new()
    }
}

const JAVA_LANG_TYPES: &[&str] = &[
    "String",
    "Object",
    "Integer",
    "Long",
    "Double",
    "Float",
    "Boolean",
    "Byte",
    "Short",
    "Character",
    "Number",
    "Class",
    "System",
    "Exception",
    "RuntimeException",
    "Error",
    "Throwable",
    "NullPointerException",
    "IllegalArgumentException",
    "IllegalStateException",
    "UnsupportedOperationException",
    "IndexOutOfBoundsException",
    "ClassCastException",
    "StringBuilder",
    "StringBuffer",
    "Math",
    "Thread",
    "Runnable",
    "Comparable",
    "Iterable",
    "AutoCloseable",
    "Override",
    "Deprecated",
    "SuppressWarnings",
    "FunctionalInterface",
    "SafeVarargs",
];

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
        fs::write(
            src.join("App.java"),
            "package com.example; public class App {}",
        )
        .unwrap();
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
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
        let resolver = JavaResolver::new(root.to_path_buf(), known_files, None);
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

    // =========================================================================
    // Type-ref resolution tests
    // =========================================================================

    #[test]
    fn test_resolve_type_ref_same_package() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve_type_ref("UserService", &from_file);
        match result {
            Resolution::Resolved(path) => {
                assert!(
                    path.ends_with("com/example/UserService.java"),
                    "Should resolve to UserService.java, got {:?}",
                    path
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_type_ref_java_lang() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        let result = resolver.resolve_type_ref("String", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "java.lang");
            }
            other => panic!("expected External(java.lang), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_type_ref_not_in_package() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // User is in com.example.model, not in com.example (App's package)
        let result = resolver.resolve_type_ref("User", &from_file);
        assert!(
            matches!(result, Resolution::Unresolved(_)),
            "User should not resolve from App's package, got {:?}",
            result
        );
    }

    #[test]
    fn test_resolve_type_ref_self_excluded() {
        let (dir, known_files) = setup_maven_project();
        let resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // App resolving "App" should not resolve to itself
        let result = resolver.resolve_type_ref("App", &from_file);
        assert!(
            matches!(result, Resolution::Unresolved(_)),
            "Self-reference should be unresolved, got {:?}",
            result
        );
    }

    #[test]
    fn test_resolve_type_ref_with_external_wildcard() {
        let (dir, known_files) = setup_maven_project();
        let mut resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // Simulate a file with `import java.util.*`
        let wildcard_import = crate::model::ImportRecord {
            file: crate::model::FileId(1),
            source_path: "java.util".to_string(),
            imported_name: "*".to_string(),
            local_name: String::new(),
            span: crate::model::Span { start: 0, end: 0 },
            line_span: crate::model::LineSpan {
                start: crate::model::Position { line: 0, column: 0 },
                end: crate::model::Position { line: 0, column: 0 },
            },
            is_default: false,
            is_namespace: true,
            is_type_only: false,
            is_side_effect: false,
            is_dynamic: false,
        };
        resolver.set_file_wildcards(&[wildcard_import]);

        // "List" is not in JAVA_LANG_TYPES and not in same package,
        // but should resolve as External via the wildcard
        let result = resolver.resolve_type_ref("List", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "java.util");
            }
            other => panic!("expected External(java.util), got {:?}", other),
        }

        // Same-package resolution should still work
        let result = resolver.resolve_type_ref("UserService", &from_file);
        assert!(
            matches!(result, Resolution::Resolved(_)),
            "Same-package ref should still resolve, got {:?}",
            result
        );

        // java.lang types should still work
        let result = resolver.resolve_type_ref("String", &from_file);
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "java.lang");
            }
            other => panic!("expected External(java.lang), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_type_ref_without_wildcard_still_unresolved() {
        let (dir, known_files) = setup_maven_project();
        let mut resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);
        let from_file = dir.path().join("src/main/java/com/example/App.java");

        // Without wildcards, unknown types remain unresolved
        resolver.clear_file_wildcards();
        let result = resolver.resolve_type_ref("List", &from_file);
        assert!(
            matches!(result, Resolution::Unresolved(_)),
            "Without wildcards, List should be unresolved, got {:?}",
            result
        );
    }

    #[test]
    fn test_set_file_wildcards_filters_correctly() {
        let (dir, known_files) = setup_maven_project();
        let mut resolver = JavaResolver::new(dir.path().to_path_buf(), known_files, None);

        let imports = vec![
            // External wildcard: should be included
            crate::model::ImportRecord {
                file: crate::model::FileId(1),
                source_path: "java.util".to_string(),
                imported_name: "*".to_string(),
                local_name: String::new(),
                span: crate::model::Span { start: 0, end: 0 },
                line_span: crate::model::LineSpan {
                    start: crate::model::Position { line: 0, column: 0 },
                    end: crate::model::Position { line: 0, column: 0 },
                },
                is_default: false,
                is_namespace: true,
                is_type_only: false,
                is_side_effect: false,
                is_dynamic: false,
            },
            // Project-internal wildcard: should NOT be included (not external)
            crate::model::ImportRecord {
                file: crate::model::FileId(1),
                source_path: "com.example.model".to_string(),
                imported_name: "*".to_string(),
                local_name: String::new(),
                span: crate::model::Span { start: 0, end: 0 },
                line_span: crate::model::LineSpan {
                    start: crate::model::Position { line: 0, column: 0 },
                    end: crate::model::Position { line: 0, column: 0 },
                },
                is_default: false,
                is_namespace: true,
                is_type_only: false,
                is_side_effect: false,
                is_dynamic: false,
            },
            // Synthetic import: should NOT be included (starts with @)
            crate::model::ImportRecord {
                file: crate::model::FileId(1),
                source_path: "@type-ref:List".to_string(),
                imported_name: "List".to_string(),
                local_name: String::new(),
                span: crate::model::Span { start: 0, end: 0 },
                line_span: crate::model::LineSpan {
                    start: crate::model::Position { line: 0, column: 0 },
                    end: crate::model::Position { line: 0, column: 0 },
                },
                is_default: false,
                is_namespace: false,
                is_type_only: true,
                is_side_effect: false,
                is_dynamic: false,
            },
            // Regular (non-namespace) import: should NOT be included
            crate::model::ImportRecord {
                file: crate::model::FileId(1),
                source_path: "java.io.File".to_string(),
                imported_name: "File".to_string(),
                local_name: String::new(),
                span: crate::model::Span { start: 0, end: 0 },
                line_span: crate::model::LineSpan {
                    start: crate::model::Position { line: 0, column: 0 },
                    end: crate::model::Position { line: 0, column: 0 },
                },
                is_default: false,
                is_namespace: false,
                is_type_only: false,
                is_side_effect: false,
                is_dynamic: false,
            },
        ];

        resolver.set_file_wildcards(&imports);

        // Only java.util should be in file_wildcards
        let result = resolver.resolve_type_ref("SomeUnknownType", &dir.path().join("src/main/java/com/example/App.java"));
        match result {
            Resolution::External(pkg) => {
                assert_eq!(pkg, "java.util", "Should resolve via java.util wildcard");
            }
            other => panic!("expected External, got {:?}", other),
        }
    }

    // =========================================================================
    // Source set scoped same-package resolution
    // =========================================================================

    #[test]
    fn test_type_ref_scoped_by_source_set() {
        use crate::resolver::source_sets::{SourceSetConfig, SourceSetIndex};

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create two modules with same package
        let fw_src = root.join("framework/src/main/java/com/example");
        fs::create_dir_all(&fw_src).unwrap();
        fs::write(fw_src.join("Foo.java"), "package com.example;").unwrap();
        fs::write(fw_src.join("Bar.java"), "package com.example;").unwrap();

        let app_src = root.join("app/src/main/java/com/example");
        fs::create_dir_all(&app_src).unwrap();
        fs::write(app_src.join("Baz.java"), "package com.example;").unwrap();
        fs::write(app_src.join("Bar.java"), "package com.example;").unwrap();

        let known = vec![
            fw_src.join("Foo.java"),
            fw_src.join("Bar.java"),
            app_src.join("Baz.java"),
            app_src.join("Bar.java"),
        ];

        let configs = vec![
            SourceSetConfig {
                name: "framework".to_string(),
                roots: vec!["framework/src/main/java".to_string()],
                deps: vec![],
            },
            SourceSetConfig {
                name: "app".to_string(),
                roots: vec!["app/src/main/java".to_string()],
                deps: vec!["framework".to_string()],
            },
        ];

        let index = SourceSetIndex::build(&configs, root).unwrap();
        let mut resolver = JavaResolver::new(root.to_path_buf(), known, None);
        resolver.set_source_set_index(index);

        // Framework file resolving "Bar" should get framework's Bar, not app's Bar
        let fw_foo = fw_src.join("Foo.java");
        let result = resolver.resolve_type_ref("Bar", &fw_foo);
        match result {
            Resolution::Resolved(path) => {
                assert_eq!(
                    path,
                    fw_src.join("Bar.java"),
                    "Framework should resolve Bar to its own source set"
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }

        // App file resolving "Bar" should get app's Bar (visible: same set)
        let app_baz = app_src.join("Baz.java");
        let result = resolver.resolve_type_ref("Bar", &app_baz);
        match result {
            Resolution::Resolved(path) => {
                assert_eq!(
                    path,
                    app_src.join("Bar.java"),
                    "App should resolve Bar to its own source set"
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }

        // App file resolving "Foo" should get framework's Foo (visible: app deps framework)
        let result = resolver.resolve_type_ref("Foo", &app_baz);
        match result {
            Resolution::Resolved(path) => {
                assert_eq!(
                    path,
                    fw_src.join("Foo.java"),
                    "App should resolve Foo from framework (dep)"
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }

        // Framework file resolving "Baz" should NOT resolve (framework can't see app)
        let result = resolver.resolve_type_ref("Baz", &fw_foo);
        assert!(
            !matches!(result, Resolution::Resolved(_)),
            "Framework should not resolve Baz from app"
        );
    }

    #[test]
    fn test_type_ref_same_set_still_works() {
        use crate::resolver::source_sets::{SourceSetConfig, SourceSetIndex};

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let src = root.join("mod/src/main/java/com/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("A.java"), "package com.example;").unwrap();
        fs::write(src.join("B.java"), "package com.example;").unwrap();

        let known = vec![src.join("A.java"), src.join("B.java")];

        let configs = vec![SourceSetConfig {
            name: "mod".to_string(),
            roots: vec!["mod/src/main/java".to_string()],
            deps: vec![],
        }];

        let index = SourceSetIndex::build(&configs, root).unwrap();
        let mut resolver = JavaResolver::new(root.to_path_buf(), known, None);
        resolver.set_source_set_index(index);

        let a = src.join("A.java");
        let result = resolver.resolve_type_ref("B", &a);
        match result {
            Resolution::Resolved(path) => {
                assert_eq!(path, src.join("B.java"));
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }
}
