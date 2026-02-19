use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::tsconfig::TsConfig;
use super::{Resolution, ResolutionCaveat, Resolver, UnresolvedReason};

/// File extensions to try when resolving TypeScript/JavaScript imports.
const TS_EXTENSIONS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

/// Index file names to try when an import points to a directory.
const INDEX_FILES: &[&str] = &["index.ts", "index.tsx", "index.js", "index.jsx"];

/// TypeScript/JavaScript import resolver.
///
/// Handles:
/// - Relative imports (`./foo`, `../bar`)
/// - Index file resolution (`./services` -> `./services/index.ts`)
/// - tsconfig.json `paths` aliases (`@/components/Button`)
/// - tsconfig.json `baseUrl` resolution
/// - External package detection (bare specifiers -> `External`)
///
/// Does NOT handle (documented as limitations):
/// - node_modules resolution
/// - Dynamic imports with computed paths
/// - Module augmentation / ambient declarations
/// - Conditional exports in package.json
pub struct TypeScriptResolver {
    /// Absolute path to the project root directory.
    #[allow(dead_code)]
    project_root: PathBuf,
    /// Parsed tsconfig.json settings, if available.
    tsconfig: Option<TsConfig>,
    /// Set of known files in the project for fast existence checks.
    known_files: HashSet<PathBuf>,
    /// Cache of file existence checks to avoid repeated filesystem lookups.
    existence_cache: Mutex<HashMap<PathBuf, bool>>,
}

impl TypeScriptResolver {
    /// Create a new TypeScript resolver.
    ///
    /// - `project_root`: Absolute path to the project root.
    /// - `known_files`: All known file paths in the project (absolute paths).
    /// - `tsconfig`: Optional parsed tsconfig.json.
    pub fn new(
        project_root: PathBuf,
        known_files: Vec<PathBuf>,
        tsconfig: Option<TsConfig>,
    ) -> Self {
        let known_set: HashSet<PathBuf> = known_files.into_iter().collect();
        TypeScriptResolver {
            project_root,
            tsconfig,
            known_files: known_set,
            existence_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Create a resolver by auto-detecting tsconfig.json in the project root.
    pub fn new_auto(project_root: PathBuf, known_files: Vec<PathBuf>) -> Self {
        let tsconfig = Self::find_and_parse_tsconfig(&project_root);
        Self::new(project_root, known_files, tsconfig)
    }

    /// Find and parse tsconfig.json starting from the project root.
    fn find_and_parse_tsconfig(project_root: &Path) -> Option<TsConfig> {
        let tsconfig_path = project_root.join("tsconfig.json");
        if tsconfig_path.exists() {
            TsConfig::parse(&tsconfig_path).ok()
        } else {
            None
        }
    }

    /// Check if a file exists, using the known files set first, then the cache.
    fn file_exists(&self, path: &Path) -> bool {
        // First check our known files set (fast, no I/O)
        if self.known_files.contains(path) {
            return true;
        }

        // Then check the cache
        let mut cache = self.existence_cache.lock().unwrap();
        if let Some(&exists) = cache.get(path) {
            return exists;
        }

        // Fall back to filesystem check and cache the result
        let exists = path.exists();
        cache.insert(path.to_path_buf(), exists);
        exists
    }

    /// Try to resolve a path by appending TypeScript/JavaScript extensions.
    /// Returns the first matching file path.
    fn try_with_extensions(&self, base_path: &Path) -> Option<PathBuf> {
        // Try exact path first (already has extension)
        if self.file_exists(base_path) && base_path.extension().is_some() {
            return Some(base_path.to_path_buf());
        }

        // Try adding extensions
        for ext in TS_EXTENSIONS {
            let with_ext = PathBuf::from(format!("{}{}", base_path.display(), ext));
            if self.file_exists(&with_ext) {
                return Some(with_ext);
            }
        }

        // Try as directory with index files
        for index in INDEX_FILES {
            let with_index = base_path.join(index);
            if self.file_exists(&with_index) {
                return Some(with_index);
            }
        }

        None
    }

    /// Resolve a relative import path (starts with "./" or "../").
    fn resolve_relative(&self, import_source: &str, from_file: &Path) -> Resolution {
        let base_dir = match from_file.parent() {
            Some(dir) => dir,
            None => {
                return Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
                    "cannot determine parent directory of {}",
                    from_file.display()
                )))
            }
        };

        let raw_path = base_dir.join(import_source);
        let normalized = normalize_path(&raw_path);

        match self.try_with_extensions(&normalized) {
            Some(resolved) => Resolution::Resolved(resolved),
            None => {
                Resolution::Unresolved(UnresolvedReason::FileNotFound(import_source.to_string()))
            }
        }
    }

    /// Resolve using tsconfig.json paths aliases.
    fn resolve_via_tsconfig(&self, import_source: &str) -> Option<Resolution> {
        let tsconfig = self.tsconfig.as_ref()?;
        let candidates = tsconfig.resolve_path_alias(import_source);

        if candidates.is_empty() {
            return None;
        }

        for candidate in &candidates {
            let normalized = normalize_path(candidate);
            if let Some(resolved) = self.try_with_extensions(&normalized) {
                return Some(Resolution::ResolvedWithCaveat(
                    resolved,
                    ResolutionCaveat::PathAlias,
                ));
            }
        }

        None
    }

    /// Check if an import path looks like a bare specifier (external package).
    fn is_bare_specifier(import_source: &str) -> bool {
        // Bare specifiers don't start with ".", "..", or "/"
        !import_source.starts_with('.') && !import_source.starts_with('/')
        // Scoped packages start with "@" but are still bare specifiers
        // UNLESS they match a tsconfig path alias (handled before this check)
    }

    /// Extract the package name from a bare specifier.
    /// e.g. "react" -> "react", "@types/node" -> "@types/node",
    /// "lodash/debounce" -> "lodash"
    fn extract_package_name(import_source: &str) -> &str {
        if import_source.starts_with('@') {
            // Scoped package: @scope/package or @scope/package/subpath
            match import_source.find('/') {
                Some(first_slash) => match import_source[first_slash + 1..].find('/') {
                    Some(second_slash) => &import_source[..first_slash + 1 + second_slash],
                    None => import_source,
                },
                None => import_source,
            }
        } else {
            // Regular package: package or package/subpath
            match import_source.find('/') {
                Some(slash) => &import_source[..slash],
                None => import_source,
            }
        }
    }
}

impl Resolver for TypeScriptResolver {
    fn resolve(&self, import_source: &str, from_file: &Path) -> Resolution {
        // Empty import path
        if import_source.is_empty() {
            return Resolution::Unresolved(UnresolvedReason::UnsupportedSyntax(
                "empty import path".to_string(),
            ));
        }

        // Step 1: Relative imports
        if import_source.starts_with('.') {
            return self.resolve_relative(import_source, from_file);
        }

        // Step 2: Try tsconfig paths alias resolution
        if let Some(resolution) = self.resolve_via_tsconfig(import_source) {
            return resolution;
        }

        // Step 3: Bare specifiers are external packages
        if Self::is_bare_specifier(import_source) {
            let package_name = Self::extract_package_name(import_source);
            return Resolution::External(package_name.to_string());
        }

        // Step 4: Absolute paths (uncommon in TS, but possible)
        if import_source.starts_with('/') {
            let path = PathBuf::from(import_source);
            match self.try_with_extensions(&path) {
                Some(resolved) => return Resolution::Resolved(resolved),
                None => {
                    return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                        import_source.to_string(),
                    ))
                }
            }
        }

        Resolution::Unresolved(UnresolvedReason::UnsupportedSyntax(format!(
            "unrecognized import pattern: {}",
            import_source
        )))
    }
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Only pop if there's a normal component to pop
                if components
                    .last()
                    .is_some_and(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            std::path::Component::CurDir => {} // skip
            other => {
                components.push(other);
            }
        }
    }
    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a test project with files.
    fn setup_test_project(files: &[&str]) -> (TempDir, Vec<PathBuf>) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let mut paths = Vec::new();

        for file in files {
            let full_path = root.join(file);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full_path, "// test file").unwrap();
            paths.push(full_path);
        }

        (dir, paths)
    }

    // -------------------------------------------------------
    // Relative imports
    // -------------------------------------------------------

    #[test]
    fn test_resolve_relative_with_extension() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    #[test]
    fn test_resolve_relative_tsx() {
        let (dir, paths) = setup_test_project(&["src/App.tsx", "src/Button.tsx"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/App.tsx");

        let result = resolver.resolve("./Button", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/Button.tsx"))
        );
    }

    #[test]
    fn test_resolve_relative_js() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/legacy.js"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./legacy", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/legacy.js"))
        );
    }

    #[test]
    fn test_resolve_relative_jsx() {
        let (dir, paths) = setup_test_project(&["src/App.tsx", "src/Widget.jsx"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/App.tsx");

        let result = resolver.resolve("./Widget", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/Widget.jsx"))
        );
    }

    #[test]
    fn test_resolve_relative_parent_directory() {
        let (dir, paths) = setup_test_project(&["src/components/Button.ts", "src/utils.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/components/Button.ts");

        let result = resolver.resolve("../utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    #[test]
    fn test_resolve_relative_not_found() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./nonexistent", &from);
        assert!(matches!(
            result,
            Resolution::Unresolved(UnresolvedReason::FileNotFound(_))
        ));
    }

    #[test]
    fn test_resolve_relative_explicit_extension() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        // Import with explicit extension
        let result = resolver.resolve("./utils.ts", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    // -------------------------------------------------------
    // Index file resolution (barrel files)
    // -------------------------------------------------------

    #[test]
    fn test_resolve_directory_to_index_ts() {
        let (dir, paths) =
            setup_test_project(&["src/index.ts", "src/models/index.ts", "src/models/user.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./models", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/models/index.ts"))
        );
    }

    #[test]
    fn test_resolve_directory_to_index_tsx() {
        let (dir, paths) = setup_test_project(&["src/App.tsx", "src/components/index.tsx"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/App.tsx");

        let result = resolver.resolve("./components", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/components/index.tsx"))
        );
    }

    #[test]
    fn test_resolve_directory_to_index_js() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/legacy/index.js"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./legacy", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/legacy/index.js"))
        );
    }

    #[test]
    fn test_resolve_file_preferred_over_directory() {
        // If both src/models.ts and src/models/index.ts exist,
        // the file (models.ts) should be preferred.
        let (dir, paths) =
            setup_test_project(&["src/index.ts", "src/models.ts", "src/models/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./models", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/models.ts"))
        );
    }

    // -------------------------------------------------------
    // Side-effect imports
    // -------------------------------------------------------

    #[test]
    fn test_resolve_side_effect_import() {
        // import "./polyfill" should resolve to the file
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/polyfill.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./polyfill", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/polyfill.ts"))
        );
    }

    // -------------------------------------------------------
    // External packages
    // -------------------------------------------------------

    #[test]
    fn test_resolve_external_package() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("react", &from);
        assert_eq!(result, Resolution::External("react".to_string()));
    }

    #[test]
    fn test_resolve_scoped_package() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("@types/node", &from);
        assert_eq!(result, Resolution::External("@types/node".to_string()));
    }

    #[test]
    fn test_resolve_package_with_subpath() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("lodash/debounce", &from);
        assert_eq!(result, Resolution::External("lodash".to_string()));
    }

    #[test]
    fn test_resolve_scoped_package_with_subpath() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("@angular/core/testing", &from);
        assert_eq!(result, Resolution::External("@angular/core".to_string()));
    }

    // -------------------------------------------------------
    // tsconfig paths resolution
    // -------------------------------------------------------

    #[test]
    fn test_resolve_tsconfig_path_alias() {
        let (dir, paths) =
            setup_test_project(&["src/index.ts", "src/utils/format.ts", "tsconfig.json"]);

        // Write tsconfig with path aliases
        let tsconfig_content =
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@utils/*": ["src/utils/*"]}}}"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let tsconfig = TsConfig::parse(&dir.path().join("tsconfig.json")).unwrap();
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, Some(tsconfig));
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("@utils/format", &from);
        assert!(matches!(
            result,
            Resolution::ResolvedWithCaveat(_, ResolutionCaveat::PathAlias)
        ));

        if let Resolution::ResolvedWithCaveat(path, _) = result {
            assert_eq!(path, dir.path().join("src/utils/format.ts"));
        }
    }

    #[test]
    fn test_resolve_tsconfig_path_alias_to_directory() {
        let (dir, paths) =
            setup_test_project(&["src/index.ts", "src/components/index.ts", "tsconfig.json"]);

        let tsconfig_content = r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@components/*": ["src/components/*"]}}}"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let tsconfig = TsConfig::parse(&dir.path().join("tsconfig.json")).unwrap();
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, Some(tsconfig));
        let from = dir.path().join("src/index.ts");

        // This should not match "@components/*" since there's no wildcard part
        // It should fall through to external
        let result = resolver.resolve("@components", &from);
        // "@components" doesn't match "@components/*" (no wildcard portion), so it falls through
        assert!(matches!(result, Resolution::External(_)));
    }

    #[test]
    fn test_resolve_tsconfig_base_url() {
        let (dir, paths) =
            setup_test_project(&["src/index.ts", "src/utils/format.ts", "tsconfig.json"]);

        let tsconfig_content = r#"{"compilerOptions": {"baseUrl": "./src"}}"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let tsconfig = TsConfig::parse(&dir.path().join("tsconfig.json")).unwrap();
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, Some(tsconfig));
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("utils/format", &from);
        assert!(matches!(
            result,
            Resolution::ResolvedWithCaveat(_, ResolutionCaveat::PathAlias)
        ));

        if let Resolution::ResolvedWithCaveat(path, _) = result {
            assert_eq!(path, dir.path().join("src/utils/format.ts"));
        }
    }

    #[test]
    fn test_resolve_tsconfig_path_not_found() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "tsconfig.json"]);

        let tsconfig_content =
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@utils/*": ["src/utils/*"]}}}"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let tsconfig = TsConfig::parse(&dir.path().join("tsconfig.json")).unwrap();
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, Some(tsconfig));
        let from = dir.path().join("src/index.ts");

        // Path alias matches, but the resolved file doesn't exist.
        // Falls through to external since @utils/nonexistent is still a bare specifier.
        let result = resolver.resolve("@utils/nonexistent", &from);
        // tsconfig alias tried first, didn't find file, falls through to bare specifier
        assert!(matches!(result, Resolution::External(_)));
    }

    // -------------------------------------------------------
    // Re-exports / barrel files (resolution, not re-export tracing)
    // -------------------------------------------------------

    #[test]
    fn test_resolve_import_through_barrel_index() {
        // import { Button } from "./components" resolves to ./components/index.ts
        let (dir, paths) = setup_test_project(&[
            "src/index.ts",
            "src/components/index.ts",
            "src/components/Button.ts",
        ]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./components", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/components/index.ts"))
        );
    }

    #[test]
    fn test_resolve_import_specific_file_in_barrel_dir() {
        // import { Button } from "./components/Button" resolves directly
        let (dir, paths) = setup_test_project(&[
            "src/index.ts",
            "src/components/index.ts",
            "src/components/Button.ts",
        ]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./components/Button", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/components/Button.ts"))
        );
    }

    // -------------------------------------------------------
    // Default / namespace imports (resolution is the same)
    // -------------------------------------------------------

    #[test]
    fn test_resolve_default_import() {
        // import Foo from "./foo" resolves the same way as named imports
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/foo.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./foo", &from);
        assert_eq!(result, Resolution::Resolved(dir.path().join("src/foo.ts")));
    }

    #[test]
    fn test_resolve_namespace_import() {
        // import * as utils from "./utils" resolves the same way
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    // -------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------

    #[test]
    fn test_resolve_empty_import() {
        let (dir, paths) = setup_test_project(&["src/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("", &from);
        assert!(matches!(
            result,
            Resolution::Unresolved(UnresolvedReason::UnsupportedSyntax(_))
        ));
    }

    #[test]
    fn test_resolve_deep_relative_path() {
        let (dir, paths) = setup_test_project(&["src/a/b/c/deep.ts", "src/x/y/z/target.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/a/b/c/deep.ts");

        let result = resolver.resolve("../../../x/y/z/target", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/x/y/z/target.ts"))
        );
    }

    #[test]
    fn test_resolve_current_dir_import() {
        let (dir, paths) = setup_test_project(&["src/a.ts", "src/b.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/a.ts");

        let result = resolver.resolve("./b", &from);
        assert_eq!(result, Resolution::Resolved(dir.path().join("src/b.ts")));
    }

    #[test]
    fn test_resolve_mjs_extension() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/config.mjs"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./config", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/config.mjs"))
        );
    }

    #[test]
    fn test_resolve_cjs_extension() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/legacy.cjs"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./legacy", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/legacy.cjs"))
        );
    }

    #[test]
    fn test_ts_preferred_over_js() {
        // When both .ts and .js exist, .ts should be preferred
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts", "src/utils.js"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    #[test]
    fn test_resolve_with_known_files_only() {
        // Resolver should work with known_files even if files don't exist on disk
        let resolver = TypeScriptResolver::new(
            PathBuf::from("/project"),
            vec![
                PathBuf::from("/project/src/index.ts"),
                PathBuf::from("/project/src/utils.ts"),
            ],
            None,
        );
        let from = PathBuf::from("/project/src/index.ts");

        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(PathBuf::from("/project/src/utils.ts"))
        );
    }

    #[test]
    fn test_resolve_with_known_files_index() {
        let resolver = TypeScriptResolver::new(
            PathBuf::from("/project"),
            vec![
                PathBuf::from("/project/src/index.ts"),
                PathBuf::from("/project/src/models/index.ts"),
                PathBuf::from("/project/src/models/user.ts"),
            ],
            None,
        );
        let from = PathBuf::from("/project/src/index.ts");

        let result = resolver.resolve("./models", &from);
        assert_eq!(
            result,
            Resolution::Resolved(PathBuf::from("/project/src/models/index.ts"))
        );
    }

    // -------------------------------------------------------
    // normalize_path
    // -------------------------------------------------------

    #[test]
    fn test_normalize_path_parent_dir() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    fn test_normalize_path_current_dir() {
        assert_eq!(
            normalize_path(Path::new("/a/./b/./c")),
            PathBuf::from("/a/b/c")
        );
    }

    #[test]
    fn test_normalize_path_mixed() {
        assert_eq!(
            normalize_path(Path::new("/a/b/c/../../d/./e")),
            PathBuf::from("/a/d/e")
        );
    }

    #[test]
    fn test_normalize_path_no_change() {
        assert_eq!(normalize_path(Path::new("/a/b/c")), PathBuf::from("/a/b/c"));
    }

    // -------------------------------------------------------
    // extract_package_name
    // -------------------------------------------------------

    #[test]
    fn test_extract_package_name_simple() {
        assert_eq!(TypeScriptResolver::extract_package_name("react"), "react");
    }

    #[test]
    fn test_extract_package_name_with_subpath() {
        assert_eq!(
            TypeScriptResolver::extract_package_name("lodash/debounce"),
            "lodash"
        );
    }

    #[test]
    fn test_extract_package_name_scoped() {
        assert_eq!(
            TypeScriptResolver::extract_package_name("@types/node"),
            "@types/node"
        );
    }

    #[test]
    fn test_extract_package_name_scoped_with_subpath() {
        assert_eq!(
            TypeScriptResolver::extract_package_name("@angular/core/testing"),
            "@angular/core"
        );
    }

    #[test]
    fn test_extract_package_name_scope_only() {
        assert_eq!(TypeScriptResolver::extract_package_name("@scope"), "@scope");
    }

    // -------------------------------------------------------
    // Auto-detection of tsconfig
    // -------------------------------------------------------

    #[test]
    fn test_new_auto_with_tsconfig() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils/format.ts"]);

        let tsconfig_content =
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@utils/*": ["src/utils/*"]}}}"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let resolver = TypeScriptResolver::new_auto(dir.path().to_path_buf(), paths);
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("@utils/format", &from);
        assert!(matches!(
            result,
            Resolution::ResolvedWithCaveat(_, ResolutionCaveat::PathAlias)
        ));
    }

    #[test]
    fn test_new_auto_without_tsconfig() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts"]);

        let resolver = TypeScriptResolver::new_auto(dir.path().to_path_buf(), paths);
        let from = dir.path().join("src/index.ts");

        // Relative imports should still work
        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils.ts"))
        );
    }

    // -------------------------------------------------------
    // Caching behavior
    // -------------------------------------------------------

    #[test]
    fn test_existence_cache_populated() {
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/utils.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        // First resolve populates cache
        let result = resolver.resolve("./utils", &from);
        assert!(matches!(result, Resolution::Resolved(_)));

        // Second resolve uses cache (same result)
        let result2 = resolver.resolve("./utils", &from);
        assert_eq!(result, result2);
    }

    #[test]
    fn test_known_files_bypass_filesystem() {
        // Create resolver with known files that don't exist on disk
        let resolver = TypeScriptResolver::new(
            PathBuf::from("/nonexistent/project"),
            vec![
                PathBuf::from("/nonexistent/project/src/index.ts"),
                PathBuf::from("/nonexistent/project/src/utils.ts"),
            ],
            None,
        );

        let from = PathBuf::from("/nonexistent/project/src/index.ts");
        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(PathBuf::from("/nonexistent/project/src/utils.ts"))
        );
    }

    // -------------------------------------------------------
    // Integration-style tests with real fixture layout
    // -------------------------------------------------------

    #[test]
    fn test_basic_project_layout() {
        let (dir, paths) = setup_test_project(&[
            "src/index.ts",
            "src/services/userService.ts",
            "src/services/postService.ts",
            "src/models/user.ts",
            "src/models/post.ts",
            "src/utils/format.ts",
            "src/utils/helpers.ts",
            "src/utils/logger.ts",
            "src/controllers/userController.ts",
        ]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);

        // index.ts -> services/userService
        let from = dir.path().join("src/index.ts");
        let result = resolver.resolve("./services/userService", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/services/userService.ts"))
        );

        // index.ts -> utils/format
        let result = resolver.resolve("./utils/format", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils/format.ts"))
        );

        // services/userService.ts -> ../models/user
        let from = dir.path().join("src/services/userService.ts");
        let result = resolver.resolve("../models/user", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/models/user.ts"))
        );

        // external package
        let result = resolver.resolve("express", &from);
        assert_eq!(result, Resolution::External("express".to_string()));
    }

    #[test]
    fn test_barrel_exports_layout() {
        let (dir, paths) = setup_test_project(&[
            "src/index.ts",
            "src/components/index.ts",
            "src/components/Button.ts",
            "src/components/Input.ts",
            "src/components/Select.ts",
            "src/hooks/index.ts",
            "src/hooks/useToggle.ts",
            "src/hooks/useCounter.ts",
            "src/utils/index.ts",
            "src/utils/math.ts",
            "src/utils/validation.ts",
        ]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);

        let from = dir.path().join("src/index.ts");

        // import { Button } from "./components" -> components/index.ts
        let result = resolver.resolve("./components", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/components/index.ts"))
        );

        // import { useToggle } from "./hooks" -> hooks/index.ts
        let result = resolver.resolve("./hooks", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/hooks/index.ts"))
        );

        // import { clamp } from "./utils" -> utils/index.ts
        let result = resolver.resolve("./utils", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/utils/index.ts"))
        );

        // Re-export resolution within barrel file:
        // components/index.ts -> ./Button
        let from_barrel = dir.path().join("src/components/index.ts");
        let result = resolver.resolve("./Button", &from_barrel);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/components/Button.ts"))
        );
    }

    #[test]
    fn test_tsconfig_paths_layout() {
        let (dir, paths) = setup_test_project(&[
            "src/index.ts",
            "src/utils/format.ts",
            "src/models/user.ts",
            "src/services/userService.ts",
            "tsconfig.json",
        ]);

        let tsconfig_content = r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@utils/*": ["src/utils/*"],
                    "@models/*": ["src/models/*"],
                    "@services/*": ["src/services/*"]
                }
            }
        }"#;
        fs::write(dir.path().join("tsconfig.json"), tsconfig_content).unwrap();

        let tsconfig = TsConfig::parse(&dir.path().join("tsconfig.json")).unwrap();
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, Some(tsconfig));
        let from = dir.path().join("src/index.ts");

        let result = resolver.resolve("@utils/format", &from);
        if let Resolution::ResolvedWithCaveat(path, caveat) = &result {
            assert_eq!(*path, dir.path().join("src/utils/format.ts"));
            assert_eq!(*caveat, ResolutionCaveat::PathAlias);
        } else {
            panic!("expected ResolvedWithCaveat, got {:?}", result);
        }

        let result = resolver.resolve("@models/user", &from);
        if let Resolution::ResolvedWithCaveat(path, _) = &result {
            assert_eq!(*path, dir.path().join("src/models/user.ts"));
        } else {
            panic!("expected ResolvedWithCaveat, got {:?}", result);
        }

        let result = resolver.resolve("@services/userService", &from);
        if let Resolution::ResolvedWithCaveat(path, _) = &result {
            assert_eq!(*path, dir.path().join("src/services/userService.ts"));
        } else {
            panic!("expected ResolvedWithCaveat, got {:?}", result);
        }
    }

    #[test]
    fn test_extension_priority_order() {
        // Verify that .ts is tried before .tsx, .js, .jsx
        let (dir, paths) = setup_test_project(&["src/index.ts", "src/Component.tsx"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/index.ts");

        // Only .tsx exists, should resolve to it
        let result = resolver.resolve("./Component", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/Component.tsx"))
        );
    }

    #[test]
    fn test_index_file_priority_order() {
        // index.ts should be preferred over index.js
        let (dir, paths) = setup_test_project(&["src/app.ts", "src/lib/index.ts"]);
        let resolver = TypeScriptResolver::new(dir.path().to_path_buf(), paths, None);
        let from = dir.path().join("src/app.ts");

        let result = resolver.resolve("./lib", &from);
        assert_eq!(
            result,
            Resolution::Resolved(dir.path().join("src/lib/index.ts"))
        );
    }
}
