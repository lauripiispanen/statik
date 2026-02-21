use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use ignore::WalkBuilder;

use crate::model::Language;

/// A discovered source file with its metadata.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub language: Language,
    pub mtime: u64,
}

/// Configuration for file discovery.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryConfig {
    /// Glob patterns to include (empty means include all).
    pub include: Vec<String>,
    /// Glob patterns to exclude.
    pub exclude: Vec<String>,
    /// Filter to specific languages.
    pub languages: Vec<Language>,
}

/// Default exclude patterns for common build output and IDE directories.
const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
    // Java / JVM
    "target/", "build/", ".gradle/", ".idea/", "*.class",
];

/// Discover source files in a project directory, respecting .gitignore.
pub fn discover_files(root: &Path, config: &DiscoveryConfig) -> Result<Vec<DiscoveredFile>> {
    let mut files = Vec::new();

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // don't skip dot-prefixed dirs entirely (let gitignore decide)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true);

    // Add exclude patterns as ignore overrides (defaults + user config)
    {
        let has_patterns = !config.exclude.is_empty() || !config.include.is_empty();
        let has_defaults = !DEFAULT_EXCLUDE_PATTERNS.is_empty();
        if has_patterns || has_defaults {
            let mut overrides = ignore::overrides::OverrideBuilder::new(root);
            for pattern in DEFAULT_EXCLUDE_PATTERNS {
                overrides
                    .add(&format!("!{}", pattern))
                    .context("invalid default exclude pattern")?;
            }
            for pattern in &config.exclude {
                overrides
                    .add(&format!("!{}", pattern))
                    .context("invalid exclude pattern")?;
            }
            for pattern in &config.include {
                overrides.add(pattern).context("invalid include pattern")?;
            }
            builder.overrides(overrides.build().context("failed to build overrides")?);
        }
    }

    for entry in builder.build() {
        let entry = entry.context("error reading directory entry")?;

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();

        // Detect language from extension
        let language = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => match Language::from_extension(ext) {
                Some(lang) => lang,
                None => continue, // skip unsupported files
            },
            None => continue,
        };

        // Apply language filter
        if !config.languages.is_empty() && !config.languages.contains(&language) {
            continue;
        }

        // Get modification time
        let mtime = get_mtime(path).unwrap_or(0);

        files.push(DiscoveredFile {
            path: path.to_path_buf(),
            language,
            mtime,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn get_mtime(path: &Path) -> Result<u64> {
    let metadata = std::fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();
    Ok(mtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create source files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/index.ts"), "export const x = 1;").unwrap();
        fs::write(root.join("src/utils.ts"), "export function helper() {}").unwrap();
        fs::write(root.join("src/styles.css"), "body { color: red; }").unwrap();
        fs::write(root.join("src/app.js"), "console.log('hello');").unwrap();

        // Initialize a git repo so the ignore crate respects .gitignore
        fs::create_dir(root.join(".git")).unwrap();

        // Create a gitignore
        fs::write(root.join(".gitignore"), "node_modules/\n*.log\n").unwrap();

        // Create node_modules (should be ignored)
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join("node_modules/pkg/index.ts"), "// ignored").unwrap();

        // Create a log file (should be ignored)
        fs::write(root.join("debug.log"), "some log").unwrap();

        dir
    }

    #[test]
    fn test_discovers_supported_files() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();

        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        assert!(paths.iter().any(|p| p.ends_with("src/index.ts")));
        assert!(paths.iter().any(|p| p.ends_with("src/utils.ts")));
        assert!(paths.iter().any(|p| p.ends_with("src/app.js")));
    }

    #[test]
    fn test_skips_unsupported_extensions() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        assert!(!paths.iter().any(|p| p.ends_with("styles.css")));
    }

    #[test]
    fn test_respects_gitignore() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();

        // node_modules should be ignored
        assert!(!paths
            .iter()
            .any(|p| p.to_string_lossy().contains("node_modules")));
        // .log files should be ignored
        assert!(!paths.iter().any(|p| p.to_string_lossy().ends_with(".log")));
    }

    #[test]
    fn test_language_detection() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();

        let ts_file = files.iter().find(|f| f.path.ends_with("index.ts")).unwrap();
        assert_eq!(ts_file.language, Language::TypeScript);

        let js_file = files.iter().find(|f| f.path.ends_with("app.js")).unwrap();
        assert_eq!(js_file.language, Language::JavaScript);
    }

    #[test]
    fn test_language_filter() {
        let dir = setup_test_project();
        let config = DiscoveryConfig {
            languages: vec![Language::TypeScript],
            ..Default::default()
        };
        let files = discover_files(dir.path(), &config).unwrap();

        assert!(files.iter().all(|f| f.language == Language::TypeScript));
        assert!(files.len() >= 2); // index.ts, utils.ts
    }

    #[test]
    fn test_mtime_is_set() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();

        for file in &files {
            assert!(
                file.mtime > 0,
                "mtime should be non-zero for {}",
                file.path.display()
            );
        }
    }

    #[test]
    fn test_empty_directory() {
        let dir = TempDir::new().unwrap();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_results_are_sorted_by_path() {
        let dir = setup_test_project();
        let files = discover_files(dir.path(), &DiscoveryConfig::default()).unwrap();
        let paths: Vec<_> = files.iter().map(|f| &f.path).collect();
        for window in paths.windows(2) {
            assert!(window[0] <= window[1], "files should be sorted by path");
        }
    }

    #[test]
    fn test_exclude_pattern_filters_files() {
        let dir = setup_test_project();
        let config = DiscoveryConfig {
            exclude: vec!["*.js".to_string()],
            ..Default::default()
        };
        let files = discover_files(dir.path(), &config).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();

        // JS files should be excluded
        assert!(!paths.iter().any(|p| p.to_string_lossy().ends_with(".js")));
        // TS files should still be present
        assert!(paths.iter().any(|p| p.ends_with("src/index.ts")));
    }

    #[test]
    fn test_tsx_jsx_mjs_cjs_extensions() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/component.tsx"),
            "export default function App() {}",
        )
        .unwrap();
        fs::write(root.join("src/legacy.jsx"), "const el = <div />;").unwrap();
        fs::write(root.join("src/esm.mjs"), "export const x = 1;").unwrap();
        fs::write(root.join("src/cjs.cjs"), "module.exports = {};").unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();

        let tsx = files.iter().find(|f| f.path.ends_with("component.tsx"));
        assert!(tsx.is_some(), "should discover .tsx files");
        assert_eq!(tsx.unwrap().language, Language::TypeScript);

        let jsx = files.iter().find(|f| f.path.ends_with("legacy.jsx"));
        assert!(jsx.is_some(), "should discover .jsx files");
        assert_eq!(jsx.unwrap().language, Language::JavaScript);

        let mjs = files.iter().find(|f| f.path.ends_with("esm.mjs"));
        assert!(mjs.is_some(), "should discover .mjs files");
        assert_eq!(mjs.unwrap().language, Language::JavaScript);

        let cjs = files.iter().find(|f| f.path.ends_with("cjs.cjs"));
        assert!(cjs.is_some(), "should discover .cjs files");
        assert_eq!(cjs.unwrap().language, Language::JavaScript);
    }

    #[test]
    fn test_python_and_rust_files_discovered() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.py"), "def main(): pass").unwrap();
        fs::write(root.join("src/types.pyi"), "def main() -> None: ...").unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}").unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();

        let py = files.iter().find(|f| f.path.ends_with("main.py"));
        assert!(py.is_some(), "should discover .py files");
        assert_eq!(py.unwrap().language, Language::Python);

        let pyi = files.iter().find(|f| f.path.ends_with("types.pyi"));
        assert!(pyi.is_some(), "should discover .pyi files");
        assert_eq!(pyi.unwrap().language, Language::Python);

        let rs = files.iter().find(|f| f.path.ends_with("lib.rs"));
        assert!(rs.is_some(), "should discover .rs files");
        assert_eq!(rs.unwrap().language, Language::Rust);
    }

    #[test]
    fn test_files_without_extensions_are_skipped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("Makefile"), "all: build").unwrap();
        fs::write(root.join("Dockerfile"), "FROM rust:latest").unwrap();
        fs::write(root.join("main.ts"), "const x = 1;").unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();

        // Only main.ts should be found
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("main.ts"));
    }

    #[test]
    fn test_nonexistent_directory_returns_error() {
        let result = discover_files(
            Path::new("/nonexistent/path/that/surely/doesnt/exist"),
            &DiscoveryConfig::default(),
        );
        assert!(result.is_err(), "should error on nonexistent directory");
    }

    #[test]
    fn test_deeply_nested_files_are_discovered() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let deep_path = root.join("a/b/c/d/e/f");
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("deep.ts"), "export const deep = true;").unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("deep.ts"));
    }

    #[test]
    fn test_java_files_discovered() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/main/java/com/example")).unwrap();
        fs::write(
            root.join("src/main/java/com/example/App.java"),
            "package com.example; public class App {}",
        )
        .unwrap();
        fs::write(
            root.join("src/main/java/com/example/Util.java"),
            "package com.example; public class Util {}",
        )
        .unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();

        let java_files: Vec<_> = files
            .iter()
            .filter(|f| f.language == Language::Java)
            .collect();
        assert_eq!(java_files.len(), 2, "should discover .java files");
        assert!(java_files.iter().all(|f| f.language == Language::Java));
    }

    #[test]
    fn test_java_build_dirs_excluded() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Source file (should be discovered)
        fs::create_dir_all(root.join("src/main/java")).unwrap();
        fs::write(root.join("src/main/java/App.java"), "public class App {}").unwrap();

        // Build output dirs (should be excluded)
        fs::create_dir_all(root.join("target/classes")).unwrap();
        fs::write(root.join("target/classes/App.java"), "// compiled").unwrap();
        fs::create_dir_all(root.join("build/classes")).unwrap();
        fs::write(root.join("build/classes/App.java"), "// compiled").unwrap();
        fs::create_dir_all(root.join(".gradle")).unwrap();
        fs::write(root.join(".gradle/config.java"), "// gradle").unwrap();

        let files = discover_files(root, &DiscoveryConfig::default()).unwrap();
        let java_files: Vec<_> = files
            .iter()
            .filter(|f| f.language == Language::Java)
            .collect();
        assert_eq!(
            java_files.len(),
            1,
            "only source file should be discovered, not build artifacts"
        );
        assert!(java_files[0].path.ends_with("App.java"));
    }

    #[test]
    fn test_java_language_filter() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/App.java"), "public class App {}").unwrap();
        fs::write(root.join("src/index.ts"), "export const x = 1;").unwrap();

        let config = DiscoveryConfig {
            languages: vec![Language::Java],
            ..Default::default()
        };
        let files = discover_files(root, &config).unwrap();

        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("App.java"));
        assert_eq!(files[0].language, Language::Java);
    }
}
