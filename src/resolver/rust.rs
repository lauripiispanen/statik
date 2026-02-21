use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Resolution, Resolver, UnresolvedReason};

const RUST_STDLIB_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

pub struct RustResolver {
    known_files: HashSet<PathBuf>,
    /// Crate root files (lib.rs, main.rs, src/bin/*.rs)
    crate_roots: Vec<PathBuf>,
}

impl RustResolver {
    pub fn new(project_root: PathBuf, known_files: Vec<PathBuf>) -> Self {
        let known_set: HashSet<PathBuf> = known_files.iter().cloned().collect();
        let crate_roots = Self::detect_crate_roots(&project_root, &known_set);

        RustResolver {
            known_files: known_set,
            crate_roots,
        }
    }

    fn detect_crate_roots(project_root: &Path, known_files: &HashSet<PathBuf>) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let lib_rs = project_root.join("src/lib.rs");
        if known_files.contains(&lib_rs) {
            roots.push(lib_rs);
        }
        let main_rs = project_root.join("src/main.rs");
        if known_files.contains(&main_rs) {
            roots.push(main_rs);
        }
        // Check for binary targets in src/bin/
        for f in known_files {
            if let Ok(rel) = f.strip_prefix(project_root) {
                let rel_str = rel.to_string_lossy();
                if rel_str.starts_with("src/bin/") {
                    roots.push(f.clone());
                }
            }
        }
        roots
    }

    fn find_crate_root_for(&self, from_file: &Path) -> Option<&PathBuf> {
        // Find the crate root that this file belongs to.
        // Simple heuristic: find the crate root whose parent directory is an ancestor of from_file.
        self.crate_roots.iter().find(|root| {
            if let Some(root_dir) = root.parent() {
                from_file.starts_with(root_dir)
            } else {
                false
            }
        })
    }

    fn crate_src_dir(&self, from_file: &Path) -> Option<PathBuf> {
        self.find_crate_root_for(from_file)
            .and_then(|root| root.parent().map(|p| p.to_path_buf()))
    }

    /// Resolve `@mod:foo` to `foo.rs` or `foo/mod.rs` relative to the file containing the mod declaration.
    pub fn resolve_mod(&self, mod_name: &str, from_file: &Path) -> Resolution {
        let parent_dir = match from_file.parent() {
            Some(d) => d,
            None => {
                return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                    "no parent directory".to_string(),
                ))
            }
        };

        // Try 2018 style: foo.rs
        let rs_file = parent_dir.join(format!("{}.rs", mod_name));
        if self.known_files.contains(&rs_file) {
            return Resolution::Resolved(rs_file);
        }

        // Try 2015 style: foo/mod.rs
        let mod_file = parent_dir.join(mod_name).join("mod.rs");
        if self.known_files.contains(&mod_file) {
            return Resolution::Resolved(mod_file);
        }

        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "module '{}' not found as {}.rs or {}/mod.rs",
            mod_name, mod_name, mod_name
        )))
    }

    /// Walk path segments from a starting directory to find the target file.
    /// Returns the resolved file path if found.
    fn resolve_path_segments(&self, base_dir: &Path, segments: &[&str]) -> Option<PathBuf> {
        if segments.is_empty() {
            return None;
        }

        let mut current_dir = base_dir.to_path_buf();

        for (i, segment) in segments.iter().enumerate() {
            let is_last = i == segments.len() - 1;
            let remaining = &segments[i + 1..];

            // Try as directory with mod.rs first (prefer directory resolution for deeper paths)
            let mod_file = current_dir.join(segment).join("mod.rs");
            if self.known_files.contains(&mod_file) {
                if is_last {
                    return Some(mod_file);
                }
                // Try to resolve remaining segments inside this directory
                let sub_dir = current_dir.join(segment);
                if let Some(deeper) = self.resolve_path_segments(&sub_dir, remaining) {
                    return Some(deeper);
                }
                // Remaining segments are symbols; return the mod.rs
                return Some(mod_file);
            }

            // Try as a file: segment.rs
            let rs_file = current_dir.join(format!("{}.rs", segment));
            if self.known_files.contains(&rs_file) {
                if is_last {
                    return Some(rs_file);
                }
                // Check if there's also a directory that can resolve deeper
                let sub_dir = current_dir.join(segment);
                if sub_dir.is_dir() {
                    if let Some(deeper) = self.resolve_path_segments(&sub_dir, remaining) {
                        return Some(deeper);
                    }
                }
                // Remaining segments are symbols
                return Some(rs_file);
            }

            // Try as just a directory (no mod.rs, no .rs file)
            let dir = current_dir.join(segment);
            if dir.is_dir() {
                current_dir = dir;
                continue;
            }

            // Can't resolve further
            break;
        }

        None
    }

    fn resolve_crate_path(&self, path: &str, from_file: &Path) -> Resolution {
        let stripped = path.strip_prefix("crate::").unwrap_or(path);
        let segments: Vec<&str> = stripped.split("::").collect();

        if segments.is_empty() {
            return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                "empty crate path".to_string(),
            ));
        }

        if let Some(src_dir) = self.crate_src_dir(from_file) {
            if let Some(resolved) = self.resolve_path_segments(&src_dir, &segments) {
                if resolved != from_file {
                    return Resolution::Resolved(resolved);
                }
            }
        }

        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "crate path '{}' not found",
            path
        )))
    }

    fn resolve_super_path(&self, path: &str, from_file: &Path) -> Resolution {
        let stripped = path.strip_prefix("super::").unwrap_or(path);

        // Go up one module level from current file
        let parent_dir = match from_file.parent() {
            Some(d) => d,
            None => {
                return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                    "no parent directory".to_string(),
                ))
            }
        };

        // Determine if we're in a mod.rs or a regular file
        let file_name = from_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let module_dir = if file_name == "mod" {
            // In foo/mod.rs, super is the parent of foo/
            parent_dir.parent().unwrap_or(parent_dir)
        } else {
            // In foo.rs, super is the parent directory
            parent_dir
        };

        let segments: Vec<&str> = stripped.split("::").collect();
        if let Some(resolved) = self.resolve_path_segments(module_dir, &segments) {
            return Resolution::Resolved(resolved);
        }

        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "super path '{}' not found",
            path
        )))
    }

    fn resolve_self_path(&self, path: &str, from_file: &Path) -> Resolution {
        let stripped = path.strip_prefix("self::").unwrap_or(path);

        let parent_dir = match from_file.parent() {
            Some(d) => d,
            None => {
                return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                    "no parent directory".to_string(),
                ))
            }
        };

        let module_dir = parent_dir;

        let segments: Vec<&str> = stripped.split("::").collect();
        if let Some(resolved) = self.resolve_path_segments(module_dir, &segments) {
            return Resolution::Resolved(resolved);
        }

        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "self path '{}' not found",
            path
        )))
    }

    fn is_stdlib_path(first_segment: &str) -> bool {
        RUST_STDLIB_CRATES.contains(&first_segment)
    }
}

impl Resolver for RustResolver {
    fn resolve(&self, import_source: &str, from_file: &Path) -> Resolution {
        if import_source.is_empty() {
            return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                "empty import".to_string(),
            ));
        }

        // Handle @mod: prefix (module declarations)
        if let Some(mod_name) = import_source.strip_prefix("@mod:") {
            return self.resolve_mod(mod_name, from_file);
        }

        // Handle extern:: prefix
        if let Some(crate_name) = import_source.strip_prefix("extern::") {
            return Resolution::External(crate_name.to_string());
        }

        // Handle crate:: prefix
        if import_source.starts_with("crate::") {
            return self.resolve_crate_path(import_source, from_file);
        }

        // Handle super:: prefix
        if import_source.starts_with("super::") {
            return self.resolve_super_path(import_source, from_file);
        }

        // Handle self:: prefix
        if import_source.starts_with("self::") {
            return self.resolve_self_path(import_source, from_file);
        }

        // For bare paths: check if first segment is stdlib
        let first_segment = import_source.split("::").next().unwrap_or(import_source);
        if Self::is_stdlib_path(first_segment) {
            return Resolution::External(first_segment.to_string());
        }

        // Try to resolve as a crate-relative path (Rust 2015 style)
        let segments: Vec<&str> = import_source.split("::").collect();
        if let Some(src_dir) = self.crate_src_dir(from_file) {
            if let Some(resolved) = self.resolve_path_segments(&src_dir, &segments) {
                if resolved != from_file {
                    return Resolution::Resolved(resolved);
                }
            }
        }

        // Not found in project — classify as external
        Resolution::External(first_segment.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_rust_project() -> (TempDir, Vec<PathBuf>) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create a Rust project structure
        let src = root.join("src");
        fs::create_dir_all(src.join("model")).unwrap();
        fs::create_dir_all(src.join("service")).unwrap();
        fs::write(src.join("lib.rs"), "mod model; mod service;").unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("model.rs"), "pub struct User;").unwrap();
        fs::write(src.join("model/user.rs"), "pub struct User;").unwrap();
        fs::write(src.join("model/mod.rs"), "pub mod user;").unwrap();
        fs::write(src.join("service.rs"), "use crate::model::User;").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("main.rs"),
            src.join("model.rs"),
            src.join("model/user.rs"),
            src.join("model/mod.rs"),
            src.join("service.rs"),
        ];

        (dir, known)
    }

    #[test]
    fn test_resolve_crate_path() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/service.rs");

        let result = resolver.resolve("crate::model", &from);
        match result {
            Resolution::Resolved(path) => {
                // Should resolve to either model.rs or model/mod.rs
                assert!(
                    path.ends_with("model.rs") || path.ends_with("model/mod.rs"),
                    "got {:?}",
                    path
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_crate_nested_path() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/service.rs");

        let result = resolver.resolve("crate::model::user", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("model/user.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_mod_declaration() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        let result = resolver.resolve("@mod:model", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(
                    path.ends_with("model.rs") || path.ends_with("model/mod.rs"),
                    "got {:?}",
                    path
                );
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_mod_dir_style() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("foo")).unwrap();
        fs::write(src.join("lib.rs"), "mod foo;").unwrap();
        fs::write(src.join("foo/mod.rs"), "pub fn bar() {}").unwrap();

        let known = vec![src.join("lib.rs"), src.join("foo/mod.rs")];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("lib.rs");

        let result = resolver.resolve("@mod:foo", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("foo/mod.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_extern_crate() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        let result = resolver.resolve("extern::serde", &from);
        match result {
            Resolution::External(name) => assert_eq!(name, "serde"),
            other => panic!("expected External, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_std_lib() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        let result = resolver.resolve("std::collections::HashMap", &from);
        match result {
            Resolution::External(name) => assert_eq!(name, "std"),
            other => panic!("expected External, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_unknown_external() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        let result = resolver.resolve("serde::Serialize", &from);
        match result {
            Resolution::External(name) => assert_eq!(name, "serde"),
            other => panic!("expected External, got {:?}", other),
        }
    }

    #[test]
    fn test_crate_root_detection_lib() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "").unwrap();

        let known: HashSet<PathBuf> = [src.join("lib.rs")].into();
        let roots = RustResolver::detect_crate_roots(dir.path(), &known);
        assert!(roots.iter().any(|r| r.ends_with("src/lib.rs")));
    }

    #[test]
    fn test_crate_root_detection_bin() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();

        let known: HashSet<PathBuf> = [src.join("main.rs")].into();
        let roots = RustResolver::detect_crate_roots(dir.path(), &known);
        assert!(roots.iter().any(|r| r.ends_with("src/main.rs")));
    }

    #[test]
    fn test_resolve_super_path() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("model")).unwrap();
        fs::write(src.join("lib.rs"), "mod model;").unwrap();
        fs::write(src.join("model/mod.rs"), "mod user; mod role;").unwrap();
        fs::write(src.join("model/user.rs"), "pub struct User;").unwrap();
        fs::write(src.join("model/role.rs"), "use super::user;").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("model/mod.rs"),
            src.join("model/user.rs"),
            src.join("model/role.rs"),
        ];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("model/role.rs");

        let result = resolver.resolve("super::user", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("model/user.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_self_path() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("model")).unwrap();
        fs::write(src.join("lib.rs"), "mod model;").unwrap();
        fs::write(src.join("model/mod.rs"), "mod user; use self::user::User;").unwrap();
        fs::write(src.join("model/user.rs"), "pub struct User;").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("model/mod.rs"),
            src.join("model/user.rs"),
        ];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("model/mod.rs");

        let result = resolver.resolve("self::user", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("model/user.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_wildcard_resolution() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/service.rs");

        // Wildcard import resolves to the module file itself
        let result = resolver.resolve("crate::model", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(
                    path.ends_with("model.rs") || path.ends_with("model/mod.rs"),
                    "got {:?}",
                    path
                );
            }
            other => panic!("expected Resolved for wildcard base, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_import() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        let result = resolver.resolve("", &from);
        assert!(matches!(result, Resolution::Unresolved(_)));
    }
}
