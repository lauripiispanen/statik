use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Resolution, ResolutionCaveat, Resolver, UnresolvedReason};

const RUST_STDLIB_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

pub struct RustResolver {
    known_files: HashSet<PathBuf>,
    /// Crate root files (lib.rs, main.rs, src/bin/*.rs)
    crate_roots: Vec<PathBuf>,
    /// Known dependency crate names from Cargo.toml
    known_crates: HashSet<String>,
}

impl RustResolver {
    pub fn new(project_root: PathBuf, known_files: Vec<PathBuf>) -> Self {
        let known_set: HashSet<PathBuf> = known_files.iter().cloned().collect();
        let crate_roots = Self::detect_crate_roots(&project_root, &known_set);
        let known_crates = Self::read_cargo_dependencies(&project_root);

        RustResolver {
            known_files: known_set,
            crate_roots,
            known_crates,
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

        // Check both 2018 style (foo.rs) and 2015 style (foo/mod.rs)
        let rs_file = parent_dir.join(format!("{}.rs", mod_name));
        let mod_file = parent_dir.join(mod_name).join("mod.rs");
        let has_rs = self.known_files.contains(&rs_file);
        let has_mod = self.known_files.contains(&mod_file);

        if has_rs && has_mod {
            // Both exist: this is Rust error E0761. Pick foo.rs but flag as ambiguous.
            return Resolution::ResolvedWithCaveat(rs_file, ResolutionCaveat::AmbiguousModule);
        }

        if has_rs {
            return Resolution::Resolved(rs_file);
        }

        if has_mod {
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

            // Try 2018 style first: segment.rs
            let rs_file = current_dir.join(format!("{}.rs", segment));
            if self.known_files.contains(&rs_file) {
                if is_last {
                    return Some(rs_file);
                }
                // Check if there's also a directory that can resolve deeper
                let sub_dir = current_dir.join(segment);
                if self.dir_exists_in_known_files(&sub_dir) {
                    if let Some(deeper) = self.resolve_path_segments(&sub_dir, remaining) {
                        return Some(deeper);
                    }
                }
                // Remaining segments are symbols
                return Some(rs_file);
            }

            // Try 2015 style: segment/mod.rs
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

            // Try as just a directory (no mod.rs, no .rs file)
            let dir = current_dir.join(segment);
            if self.dir_exists_in_known_files(&dir) {
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
        // Count and strip all leading `super::` prefixes
        let mut remaining = path;
        let mut super_count = 0u32;
        while let Some(rest) = remaining.strip_prefix("super::") {
            super_count += 1;
            remaining = rest;
        }
        // Handle bare `super` without trailing `::`
        if remaining == "super" {
            super_count += 1;
            remaining = "";
        }

        // Go up from current file's module directory
        let parent_dir = match from_file.parent() {
            Some(d) => d,
            None => {
                return Resolution::Unresolved(UnresolvedReason::FileNotFound(
                    "no parent directory".to_string(),
                ))
            }
        };

        // Determine the starting module directory
        let file_name = from_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut module_dir = if file_name == "mod" || file_name == "lib" || file_name == "main" {
            // In foo/mod.rs, the first super goes to parent of foo/
            parent_dir.parent().unwrap_or(parent_dir)
        } else {
            // In foo.rs, the first super goes to the parent directory
            parent_dir
        };

        // Apply additional super levels (we already handled the first one above)
        for _ in 1..super_count {
            module_dir = module_dir.parent().unwrap_or(module_dir);
        }

        if remaining.is_empty() {
            // Bare `super` or `super::super` - resolve to the module directory itself
            return Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
                "super path '{}' resolves to a directory, not a file",
                path
            )));
        }

        let segments: Vec<&str> = remaining.split("::").collect();
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

        let file_stem = from_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        // For mod.rs/lib.rs/main.rs, self:: refers to sibling modules in the same directory
        // For leaf files like bar.rs, self:: refers to submodules in a bar/ directory
        let module_dir = if file_stem == "mod" || file_stem == "lib" || file_stem == "main" {
            parent_dir.to_path_buf()
        } else {
            // bar.rs -> look in bar/ directory for submodules
            parent_dir.join(file_stem)
        };

        let segments: Vec<&str> = stripped.split("::").collect();
        if let Some(resolved) = self.resolve_path_segments(&module_dir, &segments) {
            return Resolution::Resolved(resolved);
        }

        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "self path '{}' not found",
            path
        )))
    }

    /// Read crate names from Cargo.toml [dependencies] and [dev-dependencies].
    fn read_cargo_dependencies(project_root: &Path) -> HashSet<String> {
        let cargo_toml = project_root.join("Cargo.toml");
        let content = match std::fs::read_to_string(&cargo_toml) {
            Ok(c) => c,
            Err(_) => return HashSet::new(),
        };
        let table: toml::Table = match content.parse() {
            Ok(t) => t,
            Err(_) => return HashSet::new(),
        };

        let mut crates = HashSet::new();
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(toml::Value::Table(deps)) = table.get(section) {
                for key in deps.keys() {
                    // Cargo normalizes hyphens to underscores for crate names
                    crates.insert(key.replace('-', "_"));
                }
            }
        }
        crates
    }

    /// Check if a directory exists by checking if any known file has it as a prefix.
    /// Avoids filesystem syscalls by deriving from known_files.
    fn dir_exists_in_known_files(&self, dir: &Path) -> bool {
        self.known_files
            .iter()
            .any(|f| f.starts_with(dir) && f != dir)
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

        // Check if the first segment is a known dependency crate
        if self.known_crates.contains(first_segment) {
            return Resolution::External(first_segment.to_string());
        }

        // Not found in project and not a known crate — report as unresolved
        Resolution::Unresolved(UnresolvedReason::FileNotFound(format!(
            "bare path '{}' not found in project and '{}' is not a known dependency",
            import_source, first_segment
        )))
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

        // Create a Cargo.toml with dependencies
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = "1"
"#,
        )
        .unwrap();

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
            Resolution::Resolved(path) | Resolution::ResolvedWithCaveat(path, _) => {
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

        // The test fixture has both model.rs and model/mod.rs, so we expect AmbiguousModule caveat
        let result = resolver.resolve("@mod:model", &from);
        match result {
            Resolution::ResolvedWithCaveat(path, ResolutionCaveat::AmbiguousModule) => {
                assert!(
                    path.ends_with("model.rs"),
                    "should prefer model.rs, got {:?}",
                    path
                );
            }
            Resolution::Resolved(path) => {
                assert!(
                    path.ends_with("model.rs") || path.ends_with("model/mod.rs"),
                    "got {:?}",
                    path
                );
            }
            other => panic!("expected Resolved or ResolvedWithCaveat, got {:?}", other),
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
    fn test_resolve_known_dependency_external() {
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
    fn test_resolve_unknown_bare_path_is_unresolved() {
        let (dir, known) = setup_rust_project();
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = dir.path().join("src/lib.rs");

        // Typo: "mdoel" is not a known crate or in-project module
        let result = resolver.resolve("mdoel::User", &from);
        assert!(
            matches!(result, Resolution::Unresolved(_)),
            "typo in bare path should be Unresolved, got {:?}",
            result
        );
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
            Resolution::Resolved(path) | Resolution::ResolvedWithCaveat(path, _) => {
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

    #[test]
    fn test_chained_super_resolution() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("lib.rs"), "mod a;").unwrap();
        fs::write(src.join("a/mod.rs"), "mod b;").unwrap();
        fs::write(src.join("a/b/mod.rs"), "use super::super::c;").unwrap();
        fs::write(src.join("c.rs"), "pub fn something() {}").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("a/mod.rs"),
            src.join("a/b/mod.rs"),
            src.join("c.rs"),
        ];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("a/b/mod.rs");

        // super::super::c from a/b/mod.rs -> go up to a/, then up to src/, resolve c.rs
        let result = resolver.resolve("super::super::c", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("c.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved for chained super, got {:?}", other),
        }
    }

    #[test]
    fn test_chained_super_from_regular_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("lib.rs"), "mod a; mod c;").unwrap();
        fs::write(src.join("a/mod.rs"), "mod b;").unwrap();
        fs::write(src.join("a/b.rs"), "use super::super::c;").unwrap();
        fs::write(src.join("c.rs"), "pub fn something() {}").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("a/mod.rs"),
            src.join("a/b.rs"),
            src.join("c.rs"),
        ];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("a/b.rs");

        // super::super::c from a/b.rs -> super goes to a/, super goes to src/, resolve c.rs
        let result = resolver.resolve("super::super::c", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("c.rs"), "got {:?}", path);
            }
            other => panic!("expected Resolved for chained super, got {:?}", other),
        }
    }

    #[test]
    fn test_self_path_from_leaf_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("parent")).unwrap();
        fs::write(src.join("lib.rs"), "mod parent;").unwrap();
        fs::write(src.join("parent.rs"), "use self::child;").unwrap();
        fs::create_dir_all(src.join("parent")).unwrap();
        fs::write(src.join("parent/child.rs"), "pub fn something() {}").unwrap();

        let known = vec![
            src.join("lib.rs"),
            src.join("parent.rs"),
            src.join("parent/child.rs"),
        ];
        let resolver = RustResolver::new(dir.path().to_path_buf(), known);
        let from = src.join("parent.rs");

        // self::child from parent.rs should look in parent/child.rs
        let result = resolver.resolve("self::child", &from);
        match result {
            Resolution::Resolved(path) => {
                assert!(path.ends_with("parent/child.rs"), "got {:?}", path);
            }
            other => panic!(
                "expected Resolved for self:: from leaf file, got {:?}",
                other
            ),
        }
    }
}
