use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Configuration for a single source set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSetConfig {
    pub name: String,
    pub roots: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
}

/// Index mapping files to source sets with visibility queries.
///
/// Built from `Vec<SourceSetConfig>` at graph-build time. Provides
/// `can_see(from, to)` to determine whether one file is allowed to
/// depend on another based on source set boundaries.
#[derive(Debug, Clone)]
pub struct SourceSetIndex {
    /// Map from absolute root prefix to source set name.
    root_to_set: Vec<(PathBuf, String)>,
    /// Transitive dependency closure per source set (includes the set itself).
    visible_sets: HashMap<String, HashSet<String>>,
}

impl SourceSetIndex {
    /// Build a SourceSetIndex from config, validating deps.
    ///
    /// Returns an error if:
    /// - A source set references an unknown dep name
    /// - There is a cycle in the dependency graph
    pub fn build(configs: &[SourceSetConfig], project_root: &Path) -> Result<Self> {
        if configs.is_empty() {
            return Ok(Self {
                root_to_set: Vec::new(),
                visible_sets: HashMap::new(),
            });
        }

        let names: HashSet<&str> = configs.iter().map(|c| c.name.as_str()).collect();

        // Validate: all deps reference known source set names
        for config in configs {
            for dep in &config.deps {
                if !names.contains(dep.as_str()) {
                    bail!(
                        "source set '{}' depends on unknown source set '{}'",
                        config.name,
                        dep
                    );
                }
            }
        }

        // Build adjacency list for dep graph
        let dep_graph: HashMap<&str, Vec<&str>> = configs
            .iter()
            .map(|c| (c.name.as_str(), c.deps.iter().map(|d| d.as_str()).collect()))
            .collect();

        // Detect cycles using DFS
        Self::check_cycles(&dep_graph)?;

        // Compute transitive closure for each source set
        let mut visible_sets: HashMap<String, HashSet<String>> = HashMap::new();
        for config in configs {
            let mut visible = HashSet::new();
            visible.insert(config.name.clone());
            Self::collect_transitive_deps(&config.name, &dep_graph, &mut visible);
            visible_sets.insert(config.name.clone(), visible);
        }

        // Build root-to-set mapping (longest prefix first for correct matching)
        let mut root_to_set: Vec<(PathBuf, String)> = Vec::new();
        for config in configs {
            for root in &config.roots {
                let abs_root = project_root.join(root);
                root_to_set.push((abs_root, config.name.clone()));
            }
        }
        // Sort by path length descending so longest (most specific) prefix matches first
        root_to_set.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));

        Ok(Self {
            root_to_set,
            visible_sets,
        })
    }

    /// Check whether `from_file` is allowed to depend on `to_file`.
    ///
    /// Returns true when:
    /// - Both files are in the same source set
    /// - `from_file`'s source set has `to_file`'s source set in its transitive deps
    /// - Either file is not in any source set (the implicit "default" set sees everything)
    /// - No source sets are configured (empty index = everything visible)
    pub fn can_see(&self, from_file: &Path, to_file: &Path) -> bool {
        // No source sets configured: everything is visible
        if self.root_to_set.is_empty() {
            return true;
        }

        let from_set = self.file_source_set(from_file);
        let to_set = self.file_source_set(to_file);

        match (from_set, to_set) {
            // Either file is in the default set: allow
            (None, _) | (_, None) => true,
            // Both in named sets: check visibility
            (Some(from), Some(to)) => {
                if from == to {
                    return true;
                }
                self.visible_sets
                    .get(from)
                    .is_some_and(|visible| visible.contains(to))
            }
        }
    }

    /// Determine which source set a file belongs to, if any.
    pub fn file_source_set(&self, file: &Path) -> Option<&str> {
        for (root, set_name) in &self.root_to_set {
            if file.starts_with(root) {
                return Some(set_name.as_str());
            }
        }
        None
    }

    /// Whether this index has any source sets configured.
    pub fn is_empty(&self) -> bool {
        self.root_to_set.is_empty()
    }

    fn collect_transitive_deps(
        name: &str,
        dep_graph: &HashMap<&str, Vec<&str>>,
        visited: &mut HashSet<String>,
    ) {
        if let Some(deps) = dep_graph.get(name) {
            for dep in deps {
                if visited.insert(dep.to_string()) {
                    Self::collect_transitive_deps(dep, dep_graph, visited);
                }
            }
        }
    }

    fn check_cycles(dep_graph: &HashMap<&str, Vec<&str>>) -> Result<()> {
        // States: 0 = unvisited, 1 = in progress, 2 = done
        let mut state: HashMap<&str, u8> = dep_graph.keys().map(|k| (*k, 0u8)).collect();

        for &node in dep_graph.keys() {
            if state[node] == 0 {
                Self::dfs_cycle(node, dep_graph, &mut state, &mut Vec::new())?;
            }
        }

        Ok(())
    }

    fn dfs_cycle<'a>(
        node: &'a str,
        dep_graph: &HashMap<&'a str, Vec<&'a str>>,
        state: &mut HashMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
    ) -> Result<()> {
        state.insert(node, 1); // in progress
        path.push(node);

        if let Some(deps) = dep_graph.get(node) {
            for &dep in deps {
                match state.get(dep).copied().unwrap_or(0) {
                    1 => {
                        // Found a cycle — find the cycle start in path
                        let cycle_start = path.iter().position(|&n| n == dep).unwrap_or(0);
                        let cycle: Vec<&str> = path[cycle_start..].to_vec();
                        bail!(
                            "cyclic dependency among source sets: {} -> {}",
                            cycle.join(" -> "),
                            dep
                        );
                    }
                    0 => {
                        Self::dfs_cycle(dep, dep_graph, state, path)?;
                    }
                    _ => {} // already done
                }
            }
        }

        path.pop();
        state.insert(node, 2); // done
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_config(name: &str, roots: &[&str], deps: &[&str]) -> SourceSetConfig {
        SourceSetConfig {
            name: name.to_string(),
            roots: roots.iter().map(|r| r.to_string()).collect(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
        }
    }

    // =========================================================================
    // Config parsing
    // =========================================================================

    #[test]
    fn test_source_set_config_deserialize() {
        let toml = r#"
[[source_sets]]
name = "framework"
roots = ["framework/src/main/java"]

[[source_sets]]
name = "app"
roots = ["app/src/main/java"]
deps = ["framework"]
"#;

        #[derive(Deserialize)]
        struct Wrapper {
            source_sets: Vec<SourceSetConfig>,
        }

        let wrapper: Wrapper = toml::from_str(toml).unwrap();
        assert_eq!(wrapper.source_sets.len(), 2);
        assert_eq!(wrapper.source_sets[0].name, "framework");
        assert_eq!(
            wrapper.source_sets[0].roots,
            vec!["framework/src/main/java"]
        );
        assert!(wrapper.source_sets[0].deps.is_empty());
        assert_eq!(wrapper.source_sets[1].name, "app");
        assert_eq!(wrapper.source_sets[1].deps, vec!["framework"]);
    }

    #[test]
    fn test_source_set_config_missing_roots() {
        let toml = r#"
[[source_sets]]
name = "bad"
deps = ["other"]
"#;

        #[derive(Deserialize)]
        struct Wrapper {
            source_sets: Vec<SourceSetConfig>,
        }

        let result: Result<Wrapper, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    // =========================================================================
    // SourceSetIndex: unknown deps
    // =========================================================================

    #[test]
    fn test_unknown_dep_errors() {
        let dir = TempDir::new().unwrap();
        let configs = vec![make_config("app", &["app/src"], &["nonexistent"])];

        let result = SourceSetIndex::build(&configs, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown source set 'nonexistent'"), "{}", err);
    }

    // =========================================================================
    // SourceSetIndex: cyclic deps
    // =========================================================================

    #[test]
    fn test_cyclic_deps_errors() {
        let dir = TempDir::new().unwrap();
        let configs = vec![
            make_config("a", &["a/src"], &["b"]),
            make_config("b", &["b/src"], &["a"]),
        ];

        let result = SourceSetIndex::build(&configs, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cyclic"), "{}", err);
    }

    #[test]
    fn test_self_cycle_errors() {
        let dir = TempDir::new().unwrap();
        let configs = vec![make_config("a", &["a/src"], &["a"])];

        let result = SourceSetIndex::build(&configs, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cyclic"), "{}", err);
    }

    // =========================================================================
    // SourceSetIndex: file-to-source-set mapping
    // =========================================================================

    #[test]
    fn test_file_to_source_set() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src/main/java"], &[]),
            make_config("app", &["app/src/main/java"], &["framework"]),
        ];

        let index = SourceSetIndex::build(&configs, root).unwrap();

        assert_eq!(
            index.file_source_set(&root.join("framework/src/main/java/com/example/Foo.java")),
            Some("framework")
        );
        assert_eq!(
            index.file_source_set(&root.join("app/src/main/java/com/example/Bar.java")),
            Some("app")
        );
        // File not in any source set
        assert_eq!(
            index.file_source_set(&root.join("other/src/Thing.java")),
            None
        );
    }

    #[test]
    fn test_longest_prefix_wins() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src"], &[]),
            make_config(
                "framework-test",
                &["framework/src/test/java"],
                &["framework"],
            ),
        ];

        let index = SourceSetIndex::build(&configs, root).unwrap();

        // Test file should match the more specific root
        assert_eq!(
            index.file_source_set(&root.join("framework/src/test/java/com/Test.java")),
            Some("framework-test")
        );
        // Production file should match the broader root
        assert_eq!(
            index.file_source_set(&root.join("framework/src/main/java/com/Prod.java")),
            Some("framework")
        );
    }

    // =========================================================================
    // SourceSetIndex: can_see with direct deps
    // =========================================================================

    #[test]
    fn test_can_see_same_set() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![make_config("framework", &["framework/src"], &[])];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let a = root.join("framework/src/A.java");
        let b = root.join("framework/src/B.java");
        assert!(index.can_see(&a, &b));
        assert!(index.can_see(&b, &a));
    }

    #[test]
    fn test_can_see_with_dep() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src"], &[]),
            make_config("app", &["app/src"], &["framework"]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let fw = root.join("framework/src/Foo.java");
        let app = root.join("app/src/Bar.java");

        // app depends on framework: app can see framework
        assert!(index.can_see(&app, &fw));
        // framework does NOT depend on app: framework cannot see app
        assert!(!index.can_see(&fw, &app));
    }

    // =========================================================================
    // SourceSetIndex: can_see with transitive deps
    // =========================================================================

    #[test]
    fn test_can_see_transitive() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("core", &["core/src"], &[]),
            make_config("framework", &["framework/src"], &["core"]),
            make_config("app", &["app/src"], &["framework"]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let core = root.join("core/src/Core.java");
        let app = root.join("app/src/App.java");

        // app -> framework -> core (transitive)
        assert!(index.can_see(&app, &core));
        // core cannot see app
        assert!(!index.can_see(&core, &app));
    }

    // =========================================================================
    // SourceSetIndex: can_see with unrelated sets
    // =========================================================================

    #[test]
    fn test_can_see_unrelated_sets() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("mod_a", &["mod_a/src"], &[]),
            make_config("mod_b", &["mod_b/src"], &[]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let a = root.join("mod_a/src/A.java");
        let b = root.join("mod_b/src/B.java");

        // Unrelated sets cannot see each other
        assert!(!index.can_see(&a, &b));
        assert!(!index.can_see(&b, &a));
    }

    // =========================================================================
    // SourceSetIndex: default set
    // =========================================================================

    #[test]
    fn test_default_set_sees_everything() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src"], &[]),
            make_config("app", &["app/src"], &["framework"]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let fw = root.join("framework/src/Foo.java");
        let unknown = root.join("other/src/Unknown.java");

        // File not in any source set (default) can see anything
        assert!(index.can_see(&unknown, &fw));
        // Named set can see default set files
        assert!(index.can_see(&fw, &unknown));
    }

    // =========================================================================
    // SourceSetIndex: empty config = no filtering
    // =========================================================================

    #[test]
    fn test_empty_config_no_filtering() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs: Vec<SourceSetConfig> = vec![];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let a = root.join("anywhere/A.java");
        let b = root.join("elsewhere/B.java");

        assert!(index.is_empty());
        assert!(index.can_see(&a, &b));
        assert!(index.can_see(&b, &a));
    }

    // =========================================================================
    // SourceSetIndex: complex multi-module setup
    // =========================================================================

    #[test]
    fn test_complex_multi_module() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src/main/java"], &[]),
            make_config(
                "framework-test",
                &["framework/src/test/java"],
                &["framework"],
            ),
            make_config(
                "app",
                &["app/src/main/java", "app/src/java"],
                &["framework"],
            ),
            make_config("app-test", &["app/src/test/java"], &["app", "framework"]),
        ];

        let index = SourceSetIndex::build(&configs, root).unwrap();

        let fw_main = root.join("framework/src/main/java/com/Fw.java");
        let fw_test = root.join("framework/src/test/java/com/FwTest.java");
        let app_main = root.join("app/src/main/java/com/App.java");
        let app_test = root.join("app/src/test/java/com/AppTest.java");

        // framework-test can see framework
        assert!(index.can_see(&fw_test, &fw_main));
        // framework cannot see its tests
        assert!(!index.can_see(&fw_main, &fw_test));

        // app can see framework
        assert!(index.can_see(&app_main, &fw_main));
        // app cannot see framework tests
        assert!(!index.can_see(&app_main, &fw_test));
        // framework cannot see app
        assert!(!index.can_see(&fw_main, &app_main));

        // app-test can see app and framework (direct deps)
        assert!(index.can_see(&app_test, &app_main));
        assert!(index.can_see(&app_test, &fw_main));

        // app-test cannot see framework-test (no dep)
        assert!(!index.can_see(&app_test, &fw_test));
    }

    // =========================================================================
    // FileGraph::filter_by_source_sets integration tests
    // =========================================================================

    use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
    use crate::model::{FileId, Language};

    fn make_graph_with_source_sets() -> (TempDir, FileGraph, SourceSetIndex) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("framework", &["framework/src"], &[]),
            make_config("app", &["app/src"], &["framework"]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: root.join("framework/src/Fw.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: root.join("app/src/App.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_file(FileInfo {
            id: FileId(3),
            path: root.join("other/Unknown.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });

        (dir, graph, index)
    }

    #[test]
    fn test_filter_drops_edge_between_unrelated_sets() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let configs = vec![
            make_config("mod_a", &["mod_a/src"], &[]),
            make_config("mod_b", &["mod_b/src"], &[]),
        ];
        let index = SourceSetIndex::build(&configs, root).unwrap();

        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: root.join("mod_a/src/A.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: root.join("mod_b/src/B.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["B".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let filtered = graph.filter_by_source_sets(&index);
        assert!(filtered.direct_imports(FileId(1)).is_empty());
        assert!(filtered.direct_importers(FileId(2)).is_empty());
    }

    #[test]
    fn test_filter_keeps_edge_same_set() {
        let (_dir, mut graph, index) = make_graph_with_source_sets();

        // Add a second framework file
        let root = graph
            .get_file(FileId(1))
            .unwrap()
            .path
            .parent()
            .unwrap()
            .to_path_buf();
        graph.add_file(FileInfo {
            id: FileId(4),
            path: root.join("Other.java"),
            language: Language::Java,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
        });
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(4),
            imported_names: vec!["Other".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let filtered = graph.filter_by_source_sets(&index);
        assert_eq!(filtered.direct_imports(FileId(1)), vec![FileId(4)]);
    }

    #[test]
    fn test_filter_keeps_edge_dependent_to_dependency() {
        let (_dir, mut graph, index) = make_graph_with_source_sets();

        // app -> framework: allowed (app depends on framework)
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(1),
            imported_names: vec!["Fw".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let filtered = graph.filter_by_source_sets(&index);
        assert_eq!(filtered.direct_imports(FileId(2)), vec![FileId(1)]);
    }

    #[test]
    fn test_filter_drops_edge_dependency_to_dependent() {
        let (_dir, mut graph, index) = make_graph_with_source_sets();

        // framework -> app: NOT allowed (framework doesn't depend on app)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["App".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let filtered = graph.filter_by_source_sets(&index);
        assert!(filtered.direct_imports(FileId(1)).is_empty());
    }

    #[test]
    fn test_filter_default_set_sees_everything() {
        let (_dir, mut graph, index) = make_graph_with_source_sets();

        // unknown file -> framework: allowed (default set sees everything)
        graph.add_import(FileImport {
            from: FileId(3),
            to: FileId(1),
            imported_names: vec!["Fw".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });
        // framework -> unknown file: allowed (named set can see default set)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(3),
            imported_names: vec!["Unknown".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 2,
        });

        let filtered = graph.filter_by_source_sets(&index);
        assert_eq!(filtered.direct_imports(FileId(3)), vec![FileId(1)]);
        assert_eq!(filtered.direct_imports(FileId(1)), vec![FileId(3)]);
    }
}
