use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{ExportRecord, FileId, ImportRecord, Language};

/// Tuple of extracted file data for graph building.
type ExtractedFile = (
    FileId,
    PathBuf,
    Language,
    Vec<ImportRecord>,
    Vec<ExportRecord>,
);

/// A resolved file-level import edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileImport {
    pub from: FileId,
    pub to: FileId,
    pub imported_names: Vec<String>,
    pub is_type_only: bool,
    pub line: usize,
}

/// Metadata about a file in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub id: FileId,
    pub path: PathBuf,
    pub language: Language,
    pub exports: Vec<ExportRecord>,
    pub is_entry_point: bool,
}

/// Reason why an import could not be resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnresolvedReason {
    External(String),
    FileNotFound(String),
    DynamicPath,
}

/// An unresolved import for tracking limitations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedImport {
    pub file: FileId,
    pub import_path: String,
    pub reason: UnresolvedReason,
    pub line: usize,
}

/// File-level dependency graph with adjacency lists.
///
/// This is the primary data structure for analysis queries.
/// Built from resolved imports after extraction and resolution phases.
#[derive(Debug, Default)]
pub struct FileGraph {
    pub files: HashMap<FileId, FileInfo>,
    /// File A imports from File B: imports[A] contains edges to B.
    pub imports: HashMap<FileId, Vec<FileImport>>,
    /// File B is imported by File A: imported_by[B] contains edges from A.
    pub imported_by: HashMap<FileId, Vec<FileImport>>,
    /// Imports that could not be resolved.
    pub unresolved: Vec<UnresolvedImport>,
    /// Path to FileId lookup.
    path_to_id: HashMap<PathBuf, FileId>,
}

impl FileGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, info: FileInfo) {
        self.path_to_id.insert(info.path.clone(), info.id);
        self.imports.entry(info.id).or_default();
        self.imported_by.entry(info.id).or_default();
        self.files.insert(info.id, info);
    }

    pub fn add_import(&mut self, import: FileImport) {
        let reverse = FileImport {
            from: import.from,
            to: import.to,
            imported_names: import.imported_names.clone(),
            is_type_only: import.is_type_only,
            line: import.line,
        };
        self.imports.entry(import.from).or_default().push(import);
        self.imported_by
            .entry(reverse.to)
            .or_default()
            .push(reverse);
    }

    pub fn add_unresolved(&mut self, unresolved: UnresolvedImport) {
        self.unresolved.push(unresolved);
    }

    pub fn file_by_path(&self, path: &Path) -> Option<FileId> {
        self.path_to_id.get(path).copied()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get direct imports of a file (files this file depends on).
    pub fn direct_imports(&self, file: FileId) -> Vec<FileId> {
        self.imports
            .get(&file)
            .map(|edges| {
                let mut targets: Vec<_> = edges.iter().map(|e| e.to).collect();
                targets.sort();
                targets.dedup();
                targets
            })
            .unwrap_or_default()
    }

    /// Get direct importers of a file (files that depend on this file).
    pub fn direct_importers(&self, file: FileId) -> Vec<FileId> {
        self.imported_by
            .get(&file)
            .map(|edges| {
                let mut sources: Vec<_> = edges.iter().map(|e| e.from).collect();
                sources.sort();
                sources.dedup();
                sources
            })
            .unwrap_or_default()
    }

    /// Get all entry point files.
    pub fn entry_points(&self) -> Vec<FileId> {
        self.files
            .values()
            .filter(|f| f.is_entry_point)
            .map(|f| f.id)
            .collect()
    }

    /// Get all file IDs.
    pub fn all_file_ids(&self) -> Vec<FileId> {
        self.files.keys().copied().collect()
    }

    /// Get file info by ID.
    pub fn get_file(&self, id: FileId) -> Option<&FileInfo> {
        self.files.get(&id)
    }

    /// Return a new FileGraph with type-only edges removed.
    /// Useful for --runtime-only analysis where only runtime dependencies matter.
    pub fn without_type_only_edges(&self) -> Self {
        let mut new_graph = Self::new();

        // Copy all files
        for info in self.files.values() {
            new_graph.add_file(info.clone());
        }

        // Copy only non-type-only edges
        for edges in self.imports.values() {
            for edge in edges {
                if !edge.is_type_only {
                    new_graph.add_import(edge.clone());
                }
            }
        }

        // Copy unresolved imports
        new_graph.unresolved = self.unresolved.clone();

        new_graph
    }

    /// Get the unresolved imports list.
    pub fn unresolved_imports(&self) -> &[UnresolvedImport] {
        &self.unresolved
    }

    /// Get import edges for a file.
    pub fn import_edges(&self, file: FileId) -> Option<&Vec<FileImport>> {
        self.imports.get(&file)
    }

    /// Get reverse import edges for a file.
    pub fn imported_by_edges(&self, file: FileId) -> Option<&Vec<FileImport>> {
        self.imported_by.get(&file)
    }

    /// Iterate over all files.
    pub fn all_files(&self) -> impl Iterator<Item = (&FileId, &FileInfo)> {
        self.files.iter()
    }

    /// Iterate over all import edge lists.
    pub fn all_import_edges(&self) -> impl Iterator<Item = (&FileId, &Vec<FileImport>)> {
        self.imports.iter()
    }

    /// Total number of import edges.
    pub fn total_import_count(&self) -> usize {
        self.imports.values().map(|v| v.len()).sum()
    }

    /// Check if a file has unresolved imports.
    pub fn has_unresolved_imports(&self, file: FileId) -> bool {
        self.unresolved.iter().any(|u| u.file == file)
    }

    /// Count unresolved imports for a file.
    pub fn unresolved_import_count(&self, file: FileId) -> usize {
        self.unresolved.iter().filter(|u| u.file == file).count()
    }

    /// Pre-compute the set of files that have unresolved imports.
    pub fn files_with_unresolved_imports(&self) -> std::collections::HashSet<FileId> {
        self.unresolved.iter().map(|u| u.file).collect()
    }

    /// Build the graph from extracted data.
    ///
    /// Takes parsed imports and resolves them to file-level edges.
    /// This is a simplified resolver that handles relative paths.
    pub fn build_from_extracted(files: &[ExtractedFile], entry_point_patterns: &[&str]) -> Self {
        let mut graph = Self::new();

        // Build path lookup
        let mut path_lookup: HashMap<PathBuf, FileId> = HashMap::new();
        for (id, path, _, _, _) in files {
            path_lookup.insert(path.clone(), *id);
        }

        // Add files to graph
        for (id, path, language, _, exports) in files {
            let is_entry = is_entry_point(path, entry_point_patterns);
            graph.add_file(FileInfo {
                id: *id,
                path: path.clone(),
                language: *language,
                exports: exports.clone(),
                is_entry_point: is_entry,
            });
        }

        // Resolve imports to file edges
        for (file_id, file_path, _, imports, _) in files {
            for import in imports {
                let resolved = resolve_import_path(&import.source_path, file_path, &path_lookup);
                match resolved {
                    Some(target_id) => {
                        graph.add_import(FileImport {
                            from: *file_id,
                            to: target_id,
                            imported_names: vec![import.imported_name.clone()],
                            is_type_only: false,
                            line: import.line_span.start.line,
                        });
                    }
                    None => {
                        let reason = if !import.source_path.starts_with('.') {
                            UnresolvedReason::External(import.source_path.clone())
                        } else {
                            UnresolvedReason::FileNotFound(import.source_path.clone())
                        };
                        graph.add_unresolved(UnresolvedImport {
                            file: *file_id,
                            import_path: import.source_path.clone(),
                            reason,
                            line: import.line_span.start.line,
                        });
                    }
                }
            }
        }

        graph
    }
}

/// Check if a file is an entry point based on common patterns.
fn is_entry_point(path: &Path, patterns: &[&str]) -> bool {
    let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let file_name_with_ext = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

    // Default entry point patterns
    let default_patterns = ["index", "main", "app", "server", "cli"];

    for pattern in default_patterns.iter().chain(patterns.iter()) {
        if file_name == *pattern {
            return true;
        }
    }

    // Test files are entry points (they are roots that should not be reported as dead)
    if file_name_with_ext.contains(".test.")
        || file_name_with_ext.contains(".spec.")
        || file_name.ends_with("_test")
        || file_name.ends_with("_spec")
        || file_name.ends_with(".test")
        || file_name.ends_with(".spec")
    {
        return true;
    }

    false
}

/// Resolve a relative import path to a FileId.
fn resolve_import_path(
    import_path: &str,
    from_file: &Path,
    path_lookup: &HashMap<PathBuf, FileId>,
) -> Option<FileId> {
    // Only resolve relative imports
    if !import_path.starts_with('.') {
        return None;
    }

    let base_dir = from_file.parent()?;
    let raw_path = base_dir.join(import_path);

    // Normalize the path (resolve .. and .)
    let normalized = normalize_path(&raw_path);

    // Try extensions in order: .ts, .tsx, .js, .jsx, /index.ts, /index.tsx, /index.js, /index.jsx
    let extensions = [".ts", ".tsx", ".js", ".jsx"];
    let index_files = ["index.ts", "index.tsx", "index.js", "index.jsx"];

    // Try exact path first (if it already has an extension)
    if path_lookup.contains_key(&normalized) {
        return path_lookup.get(&normalized).copied();
    }

    // Try adding extensions
    for ext in &extensions {
        let with_ext = PathBuf::from(format!("{}{}", normalized.display(), ext));
        if let Some(id) = path_lookup.get(&with_ext) {
            return Some(*id);
        }
    }

    // Try as directory with index files
    for index in &index_files {
        let with_index = normalized.join(index);
        if let Some(id) = path_lookup.get(&with_index) {
            return Some(*id);
        }
    }

    None
}

/// Normalize a path by resolving `.` and `..` components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
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
    use crate::model::{LineSpan, Position, Span};

    fn make_import(source_path: &str) -> ImportRecord {
        ImportRecord {
            file: FileId(0),
            source_path: source_path.to_string(),
            imported_name: "default".to_string(),
            local_name: "default".to_string(),
            span: Span { start: 0, end: 10 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position {
                    line: 1,
                    column: 10,
                },
            },
            is_default: true,
            is_namespace: false,
            is_type_only: false,
            is_side_effect: false,
            is_dynamic: false,
        }
    }

    #[test]
    fn test_build_simple_graph() {
        let files = vec![
            (
                FileId(1),
                PathBuf::from("src/index.ts"),
                Language::TypeScript,
                vec![make_import("./utils")],
                vec![],
            ),
            (
                FileId(2),
                PathBuf::from("src/utils.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
        ];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        assert_eq!(graph.file_count(), 2);

        let imports = graph.direct_imports(FileId(1));
        assert_eq!(imports, vec![FileId(2)]);

        let importers = graph.direct_importers(FileId(2));
        assert_eq!(importers, vec![FileId(1)]);
    }

    #[test]
    fn test_index_file_resolution() {
        let files = vec![
            (
                FileId(1),
                PathBuf::from("src/app.ts"),
                Language::TypeScript,
                vec![make_import("./models")],
                vec![],
            ),
            (
                FileId(2),
                PathBuf::from("src/models/index.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
        ];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        let imports = graph.direct_imports(FileId(1));
        assert_eq!(imports, vec![FileId(2)]);
    }

    #[test]
    fn test_entry_point_detection() {
        let files = vec![
            (
                FileId(1),
                PathBuf::from("src/index.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
            (
                FileId(2),
                PathBuf::from("src/utils.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
            (
                FileId(3),
                PathBuf::from("src/app.test.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
        ];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        let entry_points = graph.entry_points();
        assert!(entry_points.contains(&FileId(1))); // index.ts
        assert!(!entry_points.contains(&FileId(2))); // utils.ts is not an entry
        assert!(entry_points.contains(&FileId(3))); // test file is an entry
    }

    #[test]
    fn test_external_imports_are_unresolved() {
        let files = vec![(
            FileId(1),
            PathBuf::from("src/app.ts"),
            Language::TypeScript,
            vec![make_import("react")],
            vec![],
        )];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        assert_eq!(graph.unresolved.len(), 1);
        assert!(matches!(
            graph.unresolved[0].reason,
            UnresolvedReason::External(_)
        ));
    }

    #[test]
    fn test_parent_dir_resolution() {
        let files = vec![
            (
                FileId(1),
                PathBuf::from("src/components/Button.ts"),
                Language::TypeScript,
                vec![make_import("../utils")],
                vec![],
            ),
            (
                FileId(2),
                PathBuf::from("src/utils.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
        ];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        let imports = graph.direct_imports(FileId(1));
        assert_eq!(imports, vec![FileId(2)]);
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(&PathBuf::from("src/components/../utils")),
            PathBuf::from("src/utils")
        );
        assert_eq!(
            normalize_path(&PathBuf::from("src/./utils")),
            PathBuf::from("src/utils")
        );
    }

    #[test]
    fn test_empty_graph() {
        let graph = FileGraph::new();
        assert_eq!(graph.file_count(), 0);
        assert!(graph.entry_points().is_empty());
        assert!(graph.direct_imports(FileId(1)).is_empty());
        assert!(graph.direct_importers(FileId(1)).is_empty());
    }

    #[test]
    fn test_self_import_ignored() {
        // A file importing itself should produce an edge (unusual but valid)
        let files = vec![(
            FileId(1),
            PathBuf::from("src/self.ts"),
            Language::TypeScript,
            vec![make_import("./self")],
            vec![],
        )];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        let imports = graph.direct_imports(FileId(1));
        assert_eq!(imports, vec![FileId(1)]);
    }

    #[test]
    fn test_multiple_imports_same_target_deduped() {
        let files = vec![
            (
                FileId(1),
                PathBuf::from("src/app.ts"),
                Language::TypeScript,
                vec![make_import("./utils"), make_import("./utils")],
                vec![],
            ),
            (
                FileId(2),
                PathBuf::from("src/utils.ts"),
                Language::TypeScript,
                vec![],
                vec![],
            ),
        ];

        let graph = FileGraph::build_from_extracted(&files, &[]);
        let imports = graph.direct_imports(FileId(1));
        // direct_imports deduplicates
        assert_eq!(imports, vec![FileId(2)]);
    }

    /// Integration test: build a FileGraph using the real TypeScriptResolver
    /// with actual files on disk. This tests the handoff between the resolver
    /// and the graph-building logic that happens in `build_file_graph`.
    #[test]
    fn test_resolver_to_file_graph_integration() {
        use crate::resolver::typescript::TypeScriptResolver;
        use crate::resolver::{Resolution, Resolver};
        use std::collections::HashMap;
        use tempfile::TempDir;

        // Create a temp project directory with real files
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let file_defs: Vec<(&str, &[&str])> = vec![
            ("src/index.ts", &["./services/userService"] as &[&str]),
            (
                "src/services/userService.ts",
                &["../models/user", "../utils/format"],
            ),
            ("src/models/user.ts", &[]),
            ("src/utils/format.ts", &[]),
            ("src/orphan.ts", &[]),
        ];

        // Write real files to disk
        for (path, _) in &file_defs {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, "// test").unwrap();
        }

        // Build known_paths and file IDs
        let known_paths: Vec<PathBuf> = file_defs.iter().map(|(p, _)| root.join(p)).collect();

        let path_to_id: HashMap<PathBuf, FileId> = known_paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), FileId(i as u64 + 1)))
            .collect();

        let resolver = TypeScriptResolver::new(root.to_path_buf(), known_paths, None);

        // Build graph using the real resolver (mimicking build_file_graph in commands.rs)
        let mut graph = FileGraph::new();

        for (i, (path, _)) in file_defs.iter().enumerate() {
            let abs = root.join(path);
            let file_id = FileId(i as u64 + 1);
            let file_stem = std::path::Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let is_entry = file_stem == "index";

            graph.add_file(FileInfo {
                id: file_id,
                path: abs,
                language: Language::TypeScript,
                exports: vec![],
                is_entry_point: is_entry,
            });
        }

        for (i, (path, imports)) in file_defs.iter().enumerate() {
            let from_file = root.join(path);
            let from_id = FileId(i as u64 + 1);

            for import_source in *imports {
                let resolution = resolver.resolve(import_source, &from_file);
                match resolution {
                    Resolution::Resolved(resolved_path)
                    | Resolution::ResolvedWithCaveat(resolved_path, _) => {
                        if let Some(&target_id) = path_to_id.get(&resolved_path) {
                            graph.add_import(FileImport {
                                from: from_id,
                                to: target_id,
                                imported_names: vec!["default".to_string()],
                                is_type_only: false,
                                line: 1,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        // Verify the graph structure
        assert_eq!(graph.file_count(), 5);

        // index.ts (1) -> userService.ts (2)
        let index_imports = graph.direct_imports(FileId(1));
        assert_eq!(
            index_imports,
            vec![FileId(2)],
            "index.ts should import userService.ts"
        );

        // userService.ts (2) -> user.ts (3) and format.ts (4)
        let service_imports = graph.direct_imports(FileId(2));
        assert!(
            service_imports.contains(&FileId(3)),
            "userService.ts should import user.ts"
        );
        assert!(
            service_imports.contains(&FileId(4)),
            "userService.ts should import format.ts"
        );
        assert_eq!(service_imports.len(), 2);

        // user.ts and format.ts have no imports
        assert!(graph.direct_imports(FileId(3)).is_empty());
        assert!(graph.direct_imports(FileId(4)).is_empty());

        // orphan.ts is not imported by anyone
        assert!(graph.direct_importers(FileId(5)).is_empty());

        // Verify reverse edges: userService imported by index
        let service_importers = graph.direct_importers(FileId(2));
        assert_eq!(service_importers, vec![FileId(1)]);

        // Verify entry points
        let entries = graph.entry_points();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains(&FileId(1)));
    }

    #[test]
    fn test_without_type_only_edges() {
        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/index.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: true,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/types.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        });
        graph.add_file(FileInfo {
            id: FileId(3),
            path: PathBuf::from("src/utils.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        });

        // Type-only edge: index -> types
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["UserType".to_string()],
            is_type_only: true,
            line: 1,
        });
        // Runtime edge: index -> utils
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(3),
            imported_names: vec!["helper".to_string()],
            is_type_only: false,
            line: 2,
        });

        // Original graph has both edges
        assert_eq!(graph.direct_imports(FileId(1)).len(), 2);

        // Filtered graph should only have runtime edge
        let filtered = graph.without_type_only_edges();
        assert_eq!(filtered.file_count(), 3);
        let imports = filtered.direct_imports(FileId(1));
        assert_eq!(imports, vec![FileId(3)]);

        // Reverse edges should also be filtered
        assert!(filtered.direct_importers(FileId(2)).is_empty());
        assert_eq!(filtered.direct_importers(FileId(3)), vec![FileId(1)]);
    }

    #[test]
    fn test_without_type_only_edges_all_type_only() {
        let mut graph = FileGraph::new();
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/index.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: true,
        });
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/types.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
        });

        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["Type".to_string()],
            is_type_only: true,
            line: 1,
        });

        let filtered = graph.without_type_only_edges();
        assert!(filtered.direct_imports(FileId(1)).is_empty());
        assert!(filtered.direct_importers(FileId(2)).is_empty());
    }
}
