use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::analysis::cycles::detect_cycles;
use crate::db::Database;
use crate::model::file_graph::FileGraph;

/// Classification of how an export change affects consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// A previously-existing export was removed. Consumers will break.
    Breaking,
    /// A new export was added to an existing or new file. No consumers break.
    Expanding,
    /// An export was renamed, moved, or had its signature change (future).
    Restructuring,
    /// No semantic change (e.g., file touched but exports unchanged).
    Safe,
}

/// A single export-level change between two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportChange {
    pub kind: ChangeKind,
    pub file_path: PathBuf,
    pub export_name: String,
    pub detail: String,
    /// Files that import the changed export (populated when graphs are available).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub affected_importers: Vec<PathBuf>,
    /// Confidence level: "certain" for removals with importers, "high" for renames,
    /// "medium" for restructuring, empty when not computed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub confidence: String,
}

/// A change in import edges between two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdgeChange {
    /// Whether this edge was added or removed.
    pub change: EdgeChangeKind,
    /// The file that has the import statement.
    pub from_path: PathBuf,
    /// The file being imported.
    pub to_path: PathBuf,
    /// Names imported across this edge.
    pub imported_names: Vec<String>,
}

/// Whether an import edge was added or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeChangeKind {
    Added,
    Removed,
}

/// A change in circular dependency cycles between two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleChange {
    /// Whether this cycle was introduced or resolved.
    pub change: CycleChangeKind,
    /// File paths in the cycle (sorted for deterministic output).
    pub files: Vec<PathBuf>,
    /// Number of files in the cycle.
    pub length: usize,
}

/// Whether a cycle was introduced or resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CycleChangeKind {
    Introduced,
    Resolved,
}

/// Summary statistics for the diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_added: usize,
    pub files_removed: usize,
    pub files_changed: usize,
    pub files_unchanged: usize,
    pub breaking_changes: usize,
    pub expanding_changes: usize,
    pub restructuring_changes: usize,
    pub import_edges_added: usize,
    pub import_edges_removed: usize,
    pub cycles_introduced: usize,
    pub cycles_resolved: usize,
}

/// Result of comparing two index snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub changes: Vec<ExportChange>,
    pub import_edge_changes: Vec<ImportEdgeChange>,
    pub cycle_changes: Vec<CycleChange>,
    pub summary: DiffSummary,
}

/// Compare two database snapshots and produce a structural diff.
///
/// `db_before` is the baseline (e.g., the old version).
/// `db_after` is the current state (e.g., after code changes).
///
/// This is the backward-compatible entry point that does not require
/// pre-built FileGraphs (no import edge or move detection).
pub fn compare_snapshots(db_before: &Database, db_after: &Database) -> anyhow::Result<DiffResult> {
    compare_snapshots_inner(db_before, db_after, None, None)
}

/// Compare two database snapshots with pre-built FileGraphs.
///
/// When graphs are provided, this enables:
/// - Import edge change tracking (added/removed dependency edges)
/// - Move detection (export removed from file A, same name+kind added to file B)
pub fn compare_snapshots_with_graphs(
    db_before: &Database,
    db_after: &Database,
    graph_before: &FileGraph,
    graph_after: &FileGraph,
) -> anyhow::Result<DiffResult> {
    compare_snapshots_inner(db_before, db_after, Some(graph_before), Some(graph_after))
}

/// Internal comparison engine used by both public entry points.
fn compare_snapshots_inner(
    db_before: &Database,
    db_after: &Database,
    graph_before: Option<&FileGraph>,
    graph_after: Option<&FileGraph>,
) -> anyhow::Result<DiffResult> {
    let files_before = db_before.all_files()?;
    let files_after = db_after.all_files()?;

    let paths_before: HashMap<PathBuf, _> =
        files_before.iter().map(|f| (f.path.clone(), f)).collect();
    let paths_after: HashMap<PathBuf, _> =
        files_after.iter().map(|f| (f.path.clone(), f)).collect();

    let all_paths: HashSet<&PathBuf> = paths_before.keys().chain(paths_after.keys()).collect();

    let mut changes = Vec::new();
    let mut files_added = 0usize;
    let mut files_removed = 0usize;
    let mut files_changed = 0usize;
    let mut files_unchanged = 0usize;

    // Collect all removed and added exports for cross-file move detection.
    // Key: (export_name), Value: list of file paths where this export was removed/added.
    let mut removed_exports: Vec<(String, PathBuf)> = Vec::new();
    let mut added_exports: Vec<(String, PathBuf)> = Vec::new();

    for path in &all_paths {
        match (paths_before.get(*path), paths_after.get(*path)) {
            (None, Some(after_file)) => {
                // File added
                files_added += 1;
                let exports = db_after.get_exports_by_file(after_file.id)?;
                for export in &exports {
                    added_exports.push((export.exported_name.clone(), (*path).clone()));
                    changes.push(ExportChange {
                        kind: ChangeKind::Expanding,
                        file_path: (*path).clone(),
                        export_name: export.exported_name.clone(),
                        detail: "new file".to_string(),
                        affected_importers: Vec::new(),
                        confidence: String::new(),
                    });
                }
            }
            (Some(before_file), None) => {
                // File removed
                files_removed += 1;
                let exports = db_before.get_exports_by_file(before_file.id)?;
                for export in &exports {
                    removed_exports.push((export.exported_name.clone(), (*path).clone()));
                    changes.push(ExportChange {
                        kind: ChangeKind::Breaking,
                        file_path: (*path).clone(),
                        export_name: export.exported_name.clone(),
                        detail: "file removed".to_string(),
                        affected_importers: Vec::new(),
                        confidence: String::new(),
                    });
                }
            }
            (Some(before_file), Some(after_file)) => {
                // File exists in both: compare exports
                let exports_before = db_before.get_exports_by_file(before_file.id)?;
                let exports_after = db_after.get_exports_by_file(after_file.id)?;

                let names_before: HashSet<String> = exports_before
                    .iter()
                    .map(|e| e.exported_name.clone())
                    .collect();
                let names_after: HashSet<String> = exports_after
                    .iter()
                    .map(|e| e.exported_name.clone())
                    .collect();

                let mut file_changed = false;

                // Removed exports (in before but not after)
                for name in names_before.difference(&names_after) {
                    removed_exports.push((name.clone(), (*path).clone()));
                    changes.push(ExportChange {
                        kind: ChangeKind::Breaking,
                        file_path: (*path).clone(),
                        export_name: name.clone(),
                        detail: "export removed".to_string(),
                        affected_importers: Vec::new(),
                        confidence: String::new(),
                    });
                    file_changed = true;
                }

                // Added exports (in after but not before)
                for name in names_after.difference(&names_before) {
                    added_exports.push((name.clone(), (*path).clone()));
                    changes.push(ExportChange {
                        kind: ChangeKind::Expanding,
                        file_path: (*path).clone(),
                        export_name: name.clone(),
                        detail: "export added".to_string(),
                        affected_importers: Vec::new(),
                        confidence: String::new(),
                    });
                    file_changed = true;
                }

                if file_changed {
                    files_changed += 1;
                } else {
                    files_unchanged += 1;
                }
            }
            (None, None) => unreachable!(),
        }
    }

    // Move detection: if an export name was removed from file A and added to file B,
    // reclassify both as Restructuring instead of Breaking+Expanding.
    if graph_before.is_some() {
        let removed_by_name: HashMap<&str, Vec<&PathBuf>> = {
            let mut map: HashMap<&str, Vec<&PathBuf>> = HashMap::new();
            for (name, path) in &removed_exports {
                map.entry(name.as_str()).or_default().push(path);
            }
            map
        };

        let added_by_name: HashMap<&str, Vec<&PathBuf>> = {
            let mut map: HashMap<&str, Vec<&PathBuf>> = HashMap::new();
            for (name, path) in &added_exports {
                map.entry(name.as_str()).or_default().push(path);
            }
            map
        };

        // For each export name that appears in both removed and added (but on different files),
        // reclassify as Restructuring.
        let mut moved_entries: HashSet<(String, PathBuf)> = HashSet::new();

        for (name, removed_paths) in &removed_by_name {
            if let Some(added_paths) = added_by_name.get(name) {
                // Only match moves across different files
                for rp in removed_paths {
                    for ap in added_paths {
                        if rp != ap {
                            moved_entries.insert((name.to_string(), (*rp).clone()));
                            moved_entries.insert((name.to_string(), (*ap).clone()));
                        }
                    }
                }
            }
        }

        for change in &mut changes {
            let key = (change.export_name.clone(), change.file_path.clone());
            if moved_entries.contains(&key) {
                change.kind = ChangeKind::Restructuring;
                if change.detail == "export removed" || change.detail == "file removed" {
                    change.detail = "moved to another file".to_string();
                } else if change.detail == "export added" || change.detail == "new file" {
                    change.detail = "moved from another file".to_string();
                }
            }
        }
    }

    // Importer-aware breaking change detection (when new graph is available).
    // For each Breaking change, check if any file in the NEW graph imports the
    // affected file with a matching imported name. If no importers reference
    // the removed export, downgrade to Safe.
    if let Some(g_after) = graph_after {
        // Build path -> FileId lookups outside the loop
        let path_to_id_after: HashMap<&PathBuf, crate::model::FileId> = g_after
            .all_files()
            .map(|(_, info)| (&info.path, info.id))
            .collect();

        let path_to_id_before: Option<HashMap<&PathBuf, crate::model::FileId>> =
            graph_before.map(|g_before| {
                g_before
                    .all_files()
                    .map(|(_, info)| (&info.path, info.id))
                    .collect()
            });

        for change in &mut changes {
            match change.kind {
                ChangeKind::Breaking => {
                    // Find importers of this file in the NEW graph that reference the export name
                    if let Some(&file_id) = path_to_id_after.get(&change.file_path) {
                        let mut importers = Vec::new();
                        if let Some(edges) = g_after.imported_by_edges(file_id) {
                            for edge in edges {
                                if edge.imported_names.contains(&change.export_name)
                                    || edge.imported_names.contains(&"*".to_string())
                                {
                                    if let Some(from_info) = g_after.get_file(edge.from) {
                                        if !importers.contains(&from_info.path) {
                                            importers.push(from_info.path.clone());
                                        }
                                    }
                                }
                            }
                        }
                        importers.sort();

                        if importers.is_empty() {
                            // No consumers reference this export -> Safe
                            change.kind = ChangeKind::Safe;
                            change.confidence = "certain".to_string();
                        } else {
                            change.confidence = "certain".to_string();
                            change.affected_importers = importers;
                        }
                    } else {
                        // File no longer exists in new graph (was removed) ->
                        // check old graph to see if the file had importers
                        // that still exist in new graph
                        if let (Some(g_before), Some(ref ptib)) = (graph_before, &path_to_id_before)
                        {
                            if let Some(&old_file_id) = ptib.get(&change.file_path) {
                                let mut importers = Vec::new();
                                if let Some(edges) = g_before.imported_by_edges(old_file_id) {
                                    for edge in edges {
                                        if let Some(from_info) = g_before.get_file(edge.from) {
                                            // Only report importers that still exist in the new graph
                                            if path_to_id_after.contains_key(&from_info.path)
                                                && !importers.contains(&from_info.path)
                                            {
                                                importers.push(from_info.path.clone());
                                            }
                                        }
                                    }
                                }
                                importers.sort();

                                if importers.is_empty() {
                                    change.kind = ChangeKind::Safe;
                                    change.confidence = "certain".to_string();
                                } else {
                                    change.confidence = "certain".to_string();
                                    change.affected_importers = importers;
                                }
                            }
                        }
                    }
                }
                ChangeKind::Restructuring => {
                    change.confidence = "medium".to_string();
                }
                _ => {}
            }
        }
    }

    // Sort for deterministic output
    changes.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.export_name.cmp(&b.export_name))
    });

    // Import edge comparison (only when graphs are provided)
    let import_edge_changes = if let (Some(g_before), Some(g_after)) = (graph_before, graph_after) {
        compute_import_edge_changes(g_before, g_after)
    } else {
        Vec::new()
    };

    let breaking_changes = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Breaking)
        .count();
    let expanding_changes = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Expanding)
        .count();
    let restructuring_changes = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Restructuring)
        .count();
    let import_edges_added = import_edge_changes
        .iter()
        .filter(|e| e.change == EdgeChangeKind::Added)
        .count();
    let import_edges_removed = import_edge_changes
        .iter()
        .filter(|e| e.change == EdgeChangeKind::Removed)
        .count();

    // Cycle comparison (only when graphs are provided)
    let cycle_changes = if let (Some(g_before), Some(g_after)) = (graph_before, graph_after) {
        compute_cycle_changes(g_before, g_after)
    } else {
        Vec::new()
    };
    let cycles_introduced = cycle_changes
        .iter()
        .filter(|c| c.change == CycleChangeKind::Introduced)
        .count();
    let cycles_resolved = cycle_changes
        .iter()
        .filter(|c| c.change == CycleChangeKind::Resolved)
        .count();

    Ok(DiffResult {
        changes,
        import_edge_changes,
        cycle_changes,
        summary: DiffSummary {
            files_added,
            files_removed,
            files_changed,
            files_unchanged,
            breaking_changes,
            expanding_changes,
            restructuring_changes,
            import_edges_added,
            import_edges_removed,
            cycles_introduced,
            cycles_resolved,
        },
    })
}

/// Represent a directed edge between two file paths for comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeKey {
    from: PathBuf,
    to: PathBuf,
}

/// Compare import edges between two FileGraphs and produce change records.
fn compute_import_edge_changes(
    graph_before: &FileGraph,
    graph_after: &FileGraph,
) -> Vec<ImportEdgeChange> {
    // Build sets of (from_path, to_path) -> imported_names for each graph
    let edges_before = collect_edges(graph_before);
    let edges_after = collect_edges(graph_after);

    let keys_before: HashSet<&EdgeKey> = edges_before.keys().collect();
    let keys_after: HashSet<&EdgeKey> = edges_after.keys().collect();

    let mut changes = Vec::new();

    // Removed edges
    for key in keys_before.difference(&keys_after) {
        changes.push(ImportEdgeChange {
            change: EdgeChangeKind::Removed,
            from_path: key.from.clone(),
            to_path: key.to.clone(),
            imported_names: edges_before[key].clone(),
        });
    }

    // Added edges
    for key in keys_after.difference(&keys_before) {
        changes.push(ImportEdgeChange {
            change: EdgeChangeKind::Added,
            from_path: key.from.clone(),
            to_path: key.to.clone(),
            imported_names: edges_after[key].clone(),
        });
    }

    // Sort for deterministic output
    changes.sort_by(|a, b| {
        a.from_path
            .cmp(&b.from_path)
            .then(a.to_path.cmp(&b.to_path))
    });

    changes
}

/// Collect all import edges from a FileGraph into a map of EdgeKey -> imported names.
fn collect_edges(graph: &FileGraph) -> HashMap<EdgeKey, Vec<String>> {
    let mut edges: HashMap<EdgeKey, Vec<String>> = HashMap::new();

    for (_file_id, import_list) in graph.all_import_edges() {
        for import in import_list {
            let from_path = graph
                .get_file(import.from)
                .map(|f| f.path.clone())
                .unwrap_or_default();
            let to_path = graph
                .get_file(import.to)
                .map(|f| f.path.clone())
                .unwrap_or_default();

            let key = EdgeKey {
                from: from_path,
                to: to_path,
            };
            let entry = edges.entry(key).or_default();
            for name in &import.imported_names {
                if !entry.contains(name) {
                    entry.push(name.clone());
                }
            }
        }
    }

    // Sort imported_names for deterministic comparison
    for names in edges.values_mut() {
        names.sort();
    }

    edges
}

/// Normalize a cycle to a canonical form for comparison: sorted set of file paths.
fn normalize_cycle(cycle: &crate::analysis::cycles::Cycle) -> BTreeSet<PathBuf> {
    cycle.files.iter().map(|f| f.path.clone()).collect()
}

/// Compare cycles between two FileGraphs and produce change records.
fn compute_cycle_changes(graph_before: &FileGraph, graph_after: &FileGraph) -> Vec<CycleChange> {
    // Filter mod declaration edges before cycle detection (no-op for non-Rust projects)
    let before_for_cycles = graph_before.without_mod_declaration_edges();
    let after_for_cycles = graph_after.without_mod_declaration_edges();

    let cycles_before = detect_cycles(&before_for_cycles);
    let cycles_after = detect_cycles(&after_for_cycles);

    let normalized_before: HashSet<BTreeSet<PathBuf>> =
        cycles_before.cycles.iter().map(normalize_cycle).collect();
    let normalized_after: HashSet<BTreeSet<PathBuf>> =
        cycles_after.cycles.iter().map(normalize_cycle).collect();

    let mut changes = Vec::new();

    // Introduced cycles: in after but not in before
    for cycle_set in normalized_after.difference(&normalized_before) {
        let mut files: Vec<PathBuf> = cycle_set.iter().cloned().collect();
        files.sort();
        let length = files.len();
        changes.push(CycleChange {
            change: CycleChangeKind::Introduced,
            files,
            length,
        });
    }

    // Resolved cycles: in before but not in after
    for cycle_set in normalized_before.difference(&normalized_after) {
        let mut files: Vec<PathBuf> = cycle_set.iter().cloned().collect();
        files.sort();
        let length = files.len();
        changes.push(CycleChange {
            change: CycleChangeKind::Resolved,
            files,
            length,
        });
    }

    // Sort by change kind (introduced first), then by first file path
    changes.sort_by(|a, b| {
        let kind_ord = match (&a.change, &b.change) {
            (CycleChangeKind::Introduced, CycleChangeKind::Resolved) => std::cmp::Ordering::Less,
            (CycleChangeKind::Resolved, CycleChangeKind::Introduced) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        kind_ord.then_with(|| a.files.cmp(&b.files))
    });

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::file_graph::{FileImport, FileInfo};
    use crate::model::{
        ExportRecord, FileId, FileRecord, Language, LineSpan, Position, Span, Symbol, SymbolId,
        SymbolKind, Visibility,
    };

    fn make_db_with_file(file_id: u64, path: &str, export_names: &[&str]) -> Database {
        let db = Database::in_memory().unwrap();
        let file = FileRecord {
            id: FileId(file_id),
            path: PathBuf::from(path),
            mtime: 1000,
            language: Language::TypeScript,
        };
        db.upsert_file(&file).unwrap();

        for (i, name) in export_names.iter().enumerate() {
            let sym_id = file_id * 100 + i as u64;
            let sym = Symbol {
                id: SymbolId(sym_id),
                name: name.to_string(),
                qualified_name: name.to_string(),
                kind: SymbolKind::Function,
                file: FileId(file_id),
                span: Span { start: 0, end: 10 },
                line_span: LineSpan {
                    start: Position { line: 1, column: 0 },
                    end: Position {
                        line: 1,
                        column: 10,
                    },
                },
                parent: None,
                visibility: Visibility::Public,
                signature: None,
            };
            db.insert_symbol(&sym).unwrap();

            let export = ExportRecord {
                file: FileId(file_id),
                symbol: SymbolId(sym_id),
                exported_name: name.to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
            };
            db.insert_export(&export).unwrap();
        }

        db
    }

    fn make_file_info(id: u64, path: &str) -> FileInfo {
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(path),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
            suppressions: std::collections::HashMap::new(),
            source_set: None,
        }
    }

    fn make_edge(from: u64, to: u64, names: &[&str]) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: names.iter().map(|s| s.to_string()).collect(),
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        }
    }

    // =========================================================================
    // Backward-compatible compare_snapshots tests
    // =========================================================================

    #[test]
    fn test_no_changes() {
        let before = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);
        let after = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert!(result.changes.is_empty());
        assert_eq!(result.summary.files_unchanged, 1);
        assert_eq!(result.summary.breaking_changes, 0);
        assert!(result.import_edge_changes.is_empty());
    }

    #[test]
    fn test_file_added() {
        let before = Database::in_memory().unwrap();
        let after = make_db_with_file(1, "src/new.ts", &["newFn"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].kind, ChangeKind::Expanding);
        assert_eq!(result.changes[0].export_name, "newFn");
        assert_eq!(result.summary.files_added, 1);
    }

    #[test]
    fn test_file_removed() {
        let before = make_db_with_file(1, "src/old.ts", &["oldFn"]);
        let after = Database::in_memory().unwrap();

        let result = compare_snapshots(&before, &after).unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].kind, ChangeKind::Breaking);
        assert_eq!(result.changes[0].export_name, "oldFn");
        assert_eq!(result.summary.files_removed, 1);
    }

    #[test]
    fn test_export_added() {
        let before = make_db_with_file(1, "src/utils.ts", &["foo"]);
        let after = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].kind, ChangeKind::Expanding);
        assert_eq!(result.changes[0].export_name, "bar");
        assert_eq!(result.summary.files_changed, 1);
    }

    #[test]
    fn test_export_removed() {
        let before = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);
        let after = make_db_with_file(1, "src/utils.ts", &["foo"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].kind, ChangeKind::Breaking);
        assert_eq!(result.changes[0].export_name, "bar");
    }

    #[test]
    fn test_mixed_changes() {
        let before = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);
        let after = make_db_with_file(1, "src/utils.ts", &["foo", "baz"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert_eq!(result.changes.len(), 2);

        let breaking: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Breaking)
            .collect();
        let expanding: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Expanding)
            .collect();

        assert_eq!(breaking.len(), 1);
        assert_eq!(breaking[0].export_name, "bar");
        assert_eq!(expanding.len(), 1);
        assert_eq!(expanding[0].export_name, "baz");
    }

    #[test]
    fn test_empty_databases() {
        let before = Database::in_memory().unwrap();
        let after = Database::in_memory().unwrap();

        let result = compare_snapshots(&before, &after).unwrap();
        assert!(result.changes.is_empty());
        assert_eq!(result.summary.files_added, 0);
        assert_eq!(result.summary.files_removed, 0);
    }

    // =========================================================================
    // Import edge change tests
    // =========================================================================

    #[test]
    fn test_import_edge_added() {
        let db_before = make_db_with_file(1, "src/a.ts", &["foo"]);
        let db_after = make_db_with_file(1, "src/a.ts", &["foo"]);
        // Add a second file to after DB
        let file2 = FileRecord {
            id: FileId(2),
            path: PathBuf::from("src/b.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        db_after.upsert_file(&file2).unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));
        graph_after.add_file(make_file_info(2, "src/b.ts"));
        graph_after.add_import(make_edge(1, 2, &["foo"]));

        let result =
            compare_snapshots_with_graphs(&db_before, &db_after, &graph_before, &graph_after)
                .unwrap();

        assert_eq!(result.import_edge_changes.len(), 1);
        assert_eq!(result.import_edge_changes[0].change, EdgeChangeKind::Added);
        assert_eq!(
            result.import_edge_changes[0].from_path,
            PathBuf::from("src/a.ts")
        );
        assert_eq!(
            result.import_edge_changes[0].to_path,
            PathBuf::from("src/b.ts")
        );
        assert_eq!(result.summary.import_edges_added, 1);
        assert_eq!(result.summary.import_edges_removed, 0);
    }

    #[test]
    fn test_import_edge_removed() {
        let db_before = make_db_with_file(1, "src/a.ts", &["foo"]);
        let db_after = make_db_with_file(1, "src/a.ts", &["foo"]);

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));
        graph_before.add_file(make_file_info(2, "src/b.ts"));
        graph_before.add_import(make_edge(1, 2, &["bar"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));

        let result =
            compare_snapshots_with_graphs(&db_before, &db_after, &graph_before, &graph_after)
                .unwrap();

        assert_eq!(result.import_edge_changes.len(), 1);
        assert_eq!(
            result.import_edge_changes[0].change,
            EdgeChangeKind::Removed
        );
        assert_eq!(result.summary.import_edges_removed, 1);
        assert_eq!(result.summary.import_edges_added, 0);
    }

    #[test]
    fn test_import_edges_unchanged() {
        let db_before = make_db_with_file(1, "src/a.ts", &["foo"]);
        let db_after = make_db_with_file(1, "src/a.ts", &["foo"]);

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));
        graph_before.add_file(make_file_info(2, "src/b.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));
        graph_after.add_file(make_file_info(2, "src/b.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"]));

        let result =
            compare_snapshots_with_graphs(&db_before, &db_after, &graph_before, &graph_after)
                .unwrap();

        assert!(result.import_edge_changes.is_empty());
        assert_eq!(result.summary.import_edges_added, 0);
        assert_eq!(result.summary.import_edges_removed, 0);
    }

    // =========================================================================
    // Move detection tests
    // =========================================================================

    #[test]
    fn test_move_detection_cross_file() {
        // Export "helper" removed from a.ts, added to b.ts -> Restructuring
        let before = make_db_with_file(1, "src/a.ts", &["helper"]);
        let after = make_db_with_file(2, "src/b.ts", &["helper"]);

        let graph_before = FileGraph::new();
        let graph_after = FileGraph::new();

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        // Both changes should be Restructuring (moved)
        assert_eq!(result.changes.len(), 2);
        for change in &result.changes {
            assert_eq!(
                change.kind,
                ChangeKind::Restructuring,
                "change for {} in {} should be Restructuring",
                change.export_name,
                change.file_path.display()
            );
        }
        assert_eq!(result.summary.restructuring_changes, 2);
        assert_eq!(result.summary.breaking_changes, 0);
        assert_eq!(result.summary.expanding_changes, 0);
    }

    #[test]
    fn test_move_detection_same_file_not_triggered() {
        // Export removed and added within the same file should NOT be move detection.
        // (This scenario: export "x" removed from a.ts, export "x" added to a.ts is not possible
        //  since within-file changes are handled differently - they'd be unchanged.)
        // Instead test: "x" removed from a.ts, "y" added to a.ts -- no move.
        let before = make_db_with_file(1, "src/a.ts", &["x"]);
        let after = make_db_with_file(1, "src/a.ts", &["y"]);

        let graph_before = FileGraph::new();
        let graph_after = FileGraph::new();

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        // Different names: no move, should be Breaking + Expanding
        let breaking: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Breaking)
            .collect();
        let expanding: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Expanding)
            .collect();
        assert_eq!(breaking.len(), 1);
        assert_eq!(expanding.len(), 1);
    }

    #[test]
    fn test_move_detection_detail_messages() {
        let before = make_db_with_file(1, "src/old.ts", &["movedFn"]);
        let after = make_db_with_file(2, "src/new.ts", &["movedFn"]);

        let graph_before = FileGraph::new();
        let graph_after = FileGraph::new();

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let old_change = result
            .changes
            .iter()
            .find(|c| c.file_path == PathBuf::from("src/old.ts"))
            .unwrap();
        let new_change = result
            .changes
            .iter()
            .find(|c| c.file_path == PathBuf::from("src/new.ts"))
            .unwrap();

        assert_eq!(old_change.detail, "moved to another file");
        assert_eq!(new_change.detail, "moved from another file");
    }

    #[test]
    fn test_no_move_detection_without_graphs() {
        // Without graphs, move detection is not active
        let before = make_db_with_file(1, "src/a.ts", &["helper"]);
        let after = make_db_with_file(2, "src/b.ts", &["helper"]);

        let result = compare_snapshots(&before, &after).unwrap();

        // Without graphs, these should stay as Breaking + Expanding
        let breaking: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Breaking)
            .collect();
        let expanding: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Expanding)
            .collect();
        assert_eq!(breaking.len(), 1);
        assert_eq!(expanding.len(), 1);
    }

    // =========================================================================
    // Summary field tests
    // =========================================================================

    #[test]
    fn test_summary_import_edge_counts() {
        let db_before = Database::in_memory().unwrap();
        let db_after = Database::in_memory().unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "a.ts"));
        graph_before.add_file(make_file_info(2, "b.ts"));
        graph_before.add_file(make_file_info(3, "c.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));
        graph_before.add_import(make_edge(1, 3, &["y"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "a.ts"));
        graph_after.add_file(make_file_info(2, "b.ts"));
        graph_after.add_file(make_file_info(4, "d.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"])); // unchanged
        graph_after.add_import(make_edge(1, 4, &["z"])); // new

        let result =
            compare_snapshots_with_graphs(&db_before, &db_after, &graph_before, &graph_after)
                .unwrap();

        // Edge 1->3 removed, edge 1->4 added, edge 1->2 unchanged
        assert_eq!(result.summary.import_edges_added, 1);
        assert_eq!(result.summary.import_edges_removed, 1);
        assert_eq!(result.import_edge_changes.len(), 2);
    }

    #[test]
    fn test_import_edge_changes_sorted_deterministically() {
        let db = Database::in_memory().unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "z.ts"));
        graph_before.add_file(make_file_info(2, "a.ts"));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "z.ts"));
        graph_after.add_file(make_file_info(2, "a.ts"));
        graph_after.add_file(make_file_info(3, "m.ts"));
        graph_after.add_import(make_edge(1, 3, &["x"]));
        graph_after.add_import(make_edge(2, 3, &["y"]));

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert_eq!(result.import_edge_changes.len(), 2);
        // Should be sorted by from_path: a.ts before z.ts
        assert_eq!(
            result.import_edge_changes[0].from_path,
            PathBuf::from("a.ts")
        );
        assert_eq!(
            result.import_edge_changes[1].from_path,
            PathBuf::from("z.ts")
        );
    }

    // =========================================================================
    // Importer-aware breaking change detection tests
    // =========================================================================

    #[test]
    fn test_breaking_with_importers_stays_breaking() {
        // b.ts exports "foo", a.ts imports "foo" from b.ts
        // Remove "foo" from b.ts -> Breaking (a.ts is affected)
        let before = make_db_with_file(1, "src/a.ts", &[]);
        // Add b.ts to before with export "foo"
        let file_b = FileRecord {
            id: FileId(2),
            path: PathBuf::from("src/b.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        before.upsert_file(&file_b).unwrap();
        let sym = Symbol {
            id: SymbolId(200),
            name: "foo".to_string(),
            qualified_name: "foo".to_string(),
            kind: SymbolKind::Function,
            file: FileId(2),
            span: Span { start: 0, end: 10 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position {
                    line: 1,
                    column: 10,
                },
            },
            parent: None,
            visibility: Visibility::Public,
            signature: None,
        };
        before.insert_symbol(&sym).unwrap();
        before
            .insert_export(&ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "foo".to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
            })
            .unwrap();

        // After: b.ts has no exports, but a.ts still imports foo from b.ts
        let after = Database::in_memory().unwrap();
        let file_a_after = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        after.upsert_file(&file_a_after).unwrap();
        let file_b_after = FileRecord {
            id: FileId(2),
            path: PathBuf::from("src/b.ts"),
            mtime: 1001,
            language: Language::TypeScript,
        };
        after.upsert_file(&file_b_after).unwrap();

        // Build graphs: a.ts imports "foo" from b.ts in new graph
        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));
        graph_before.add_file(make_file_info(2, "src/b.ts"));
        graph_before.add_import(make_edge(1, 2, &["foo"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));
        graph_after.add_file(make_file_info(2, "src/b.ts"));
        graph_after.add_import(make_edge(1, 2, &["foo"])); // still importing

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let foo_change = result
            .changes
            .iter()
            .find(|c| c.export_name == "foo")
            .unwrap();

        assert_eq!(foo_change.kind, ChangeKind::Breaking);
        assert_eq!(foo_change.confidence, "certain");
        assert_eq!(
            foo_change.affected_importers,
            vec![PathBuf::from("src/a.ts")]
        );
    }

    #[test]
    fn test_breaking_without_importers_becomes_safe() {
        // b.ts exports "foo", but nobody imports it
        // Remove "foo" from b.ts -> Safe (no consumers)
        let before = make_db_with_file(2, "src/b.ts", &["foo"]);
        let after = make_db_with_file(2, "src/b.ts", &[]);

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(2, "src/b.ts"));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(2, "src/b.ts"));

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let foo_change = result
            .changes
            .iter()
            .find(|c| c.export_name == "foo")
            .unwrap();

        assert_eq!(foo_change.kind, ChangeKind::Safe);
        assert_eq!(foo_change.confidence, "certain");
        assert!(foo_change.affected_importers.is_empty());
        assert_eq!(result.summary.breaking_changes, 0);
    }

    #[test]
    fn test_removed_file_with_importers_stays_breaking() {
        // file_a imports from file_b, then file_b is completely removed
        let before = make_db_with_file(2, "src/b.ts", &["bar"]);
        let file_a_before = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        before.upsert_file(&file_a_before).unwrap();

        let after = Database::in_memory().unwrap();
        let file_a_after = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        after.upsert_file(&file_a_after).unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));
        graph_before.add_file(make_file_info(2, "src/b.ts"));
        graph_before.add_import(make_edge(1, 2, &["bar"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));
        // b.ts removed from graph_after

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let bar_change = result
            .changes
            .iter()
            .find(|c| c.export_name == "bar")
            .unwrap();

        assert_eq!(bar_change.kind, ChangeKind::Breaking);
        assert_eq!(bar_change.confidence, "certain");
        assert_eq!(
            bar_change.affected_importers,
            vec![PathBuf::from("src/a.ts")]
        );
    }

    #[test]
    fn test_removed_file_without_importers_becomes_safe() {
        // file_b is removed but nobody imported from it
        let before = make_db_with_file(2, "src/b.ts", &["lonely"]);
        let after = Database::in_memory().unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(2, "src/b.ts"));

        let graph_after = FileGraph::new();

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let change = result
            .changes
            .iter()
            .find(|c| c.export_name == "lonely")
            .unwrap();

        assert_eq!(change.kind, ChangeKind::Safe);
        assert_eq!(change.confidence, "certain");
    }

    #[test]
    fn test_restructuring_gets_medium_confidence() {
        let before = make_db_with_file(1, "src/a.ts", &["helper"]);
        let after = make_db_with_file(2, "src/b.ts", &["helper"]);

        let graph_before = FileGraph::new();
        let graph_after = FileGraph::new();

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        for change in &result.changes {
            assert_eq!(change.kind, ChangeKind::Restructuring);
            assert_eq!(change.confidence, "medium");
        }
    }

    #[test]
    fn test_wildcard_import_counts_as_affected() {
        // a.ts does wildcard import from b.ts, b.ts removes an export -> a.ts is affected
        let before = make_db_with_file(2, "src/b.ts", &["foo"]);
        let after = make_db_with_file(2, "src/b.ts", &[]);

        let file_a = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        before.upsert_file(&file_a).unwrap();
        after.upsert_file(&file_a).unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "src/a.ts"));
        graph_before.add_file(make_file_info(2, "src/b.ts"));
        graph_before.add_import(make_edge(1, 2, &["*"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "src/a.ts"));
        graph_after.add_file(make_file_info(2, "src/b.ts"));
        graph_after.add_import(make_edge(1, 2, &["*"])); // wildcard still there

        let result =
            compare_snapshots_with_graphs(&before, &after, &graph_before, &graph_after).unwrap();

        let foo_change = result
            .changes
            .iter()
            .find(|c| c.export_name == "foo")
            .unwrap();

        assert_eq!(foo_change.kind, ChangeKind::Breaking);
        assert_eq!(
            foo_change.affected_importers,
            vec![PathBuf::from("src/a.ts")]
        );
    }

    // =========================================================================
    // Cycle introduction tracking tests
    // =========================================================================

    #[test]
    fn test_cycle_introduced() {
        let db = Database::in_memory().unwrap();

        // Before: no cycle (a -> b)
        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "a.ts"));
        graph_before.add_file(make_file_info(2, "b.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));

        // After: cycle (a -> b -> a)
        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "a.ts"));
        graph_after.add_file(make_file_info(2, "b.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"]));
        graph_after.add_import(make_edge(2, 1, &["y"])); // introduces cycle

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert_eq!(result.cycle_changes.len(), 1);
        assert_eq!(result.cycle_changes[0].change, CycleChangeKind::Introduced);
        assert_eq!(result.cycle_changes[0].length, 2);
        assert!(result.cycle_changes[0]
            .files
            .contains(&PathBuf::from("a.ts")));
        assert!(result.cycle_changes[0]
            .files
            .contains(&PathBuf::from("b.ts")));
        assert_eq!(result.summary.cycles_introduced, 1);
        assert_eq!(result.summary.cycles_resolved, 0);
    }

    #[test]
    fn test_cycle_resolved() {
        let db = Database::in_memory().unwrap();

        // Before: cycle (a <-> b)
        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "a.ts"));
        graph_before.add_file(make_file_info(2, "b.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));
        graph_before.add_import(make_edge(2, 1, &["y"]));

        // After: no cycle (a -> b only)
        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "a.ts"));
        graph_after.add_file(make_file_info(2, "b.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"]));

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert_eq!(result.cycle_changes.len(), 1);
        assert_eq!(result.cycle_changes[0].change, CycleChangeKind::Resolved);
        assert_eq!(result.cycle_changes[0].length, 2);
        assert_eq!(result.summary.cycles_introduced, 0);
        assert_eq!(result.summary.cycles_resolved, 1);
    }

    #[test]
    fn test_cycle_unchanged() {
        let db = Database::in_memory().unwrap();

        // Same cycle in both
        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "a.ts"));
        graph_before.add_file(make_file_info(2, "b.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));
        graph_before.add_import(make_edge(2, 1, &["y"]));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "a.ts"));
        graph_after.add_file(make_file_info(2, "b.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"]));
        graph_after.add_import(make_edge(2, 1, &["y"]));

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert!(result.cycle_changes.is_empty());
        assert_eq!(result.summary.cycles_introduced, 0);
        assert_eq!(result.summary.cycles_resolved, 0);
    }

    #[test]
    fn test_multiple_cycle_changes() {
        let db = Database::in_memory().unwrap();

        // Before: cycle a<->b
        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "a.ts"));
        graph_before.add_file(make_file_info(2, "b.ts"));
        graph_before.add_file(make_file_info(3, "c.ts"));
        graph_before.add_file(make_file_info(4, "d.ts"));
        graph_before.add_import(make_edge(1, 2, &["x"]));
        graph_before.add_import(make_edge(2, 1, &["y"])); // cycle a<->b

        // After: cycle a<->b is gone, new cycle c<->d
        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "a.ts"));
        graph_after.add_file(make_file_info(2, "b.ts"));
        graph_after.add_file(make_file_info(3, "c.ts"));
        graph_after.add_file(make_file_info(4, "d.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"])); // a->b (no cycle)
        graph_after.add_import(make_edge(3, 4, &["z"]));
        graph_after.add_import(make_edge(4, 3, &["w"])); // new cycle c<->d

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert_eq!(result.cycle_changes.len(), 2);
        assert_eq!(result.summary.cycles_introduced, 1);
        assert_eq!(result.summary.cycles_resolved, 1);

        // Introduced should be listed before resolved
        let introduced: Vec<_> = result
            .cycle_changes
            .iter()
            .filter(|c| c.change == CycleChangeKind::Introduced)
            .collect();
        let resolved: Vec<_> = result
            .cycle_changes
            .iter()
            .filter(|c| c.change == CycleChangeKind::Resolved)
            .collect();
        assert_eq!(introduced.len(), 1);
        assert_eq!(resolved.len(), 1);
        assert!(introduced[0].files.contains(&PathBuf::from("c.ts")));
        assert!(resolved[0].files.contains(&PathBuf::from("a.ts")));
    }

    #[test]
    fn test_no_cycle_changes_without_graphs() {
        let before = Database::in_memory().unwrap();
        let after = Database::in_memory().unwrap();

        let result = compare_snapshots(&before, &after).unwrap();
        assert!(result.cycle_changes.is_empty());
        assert_eq!(result.summary.cycles_introduced, 0);
        assert_eq!(result.summary.cycles_resolved, 0);
    }

    #[test]
    fn test_cycle_files_sorted_in_output() {
        let db = Database::in_memory().unwrap();

        let mut graph_before = FileGraph::new();
        graph_before.add_file(make_file_info(1, "z.ts"));
        graph_before.add_file(make_file_info(2, "a.ts"));

        let mut graph_after = FileGraph::new();
        graph_after.add_file(make_file_info(1, "z.ts"));
        graph_after.add_file(make_file_info(2, "a.ts"));
        graph_after.add_import(make_edge(1, 2, &["x"]));
        graph_after.add_import(make_edge(2, 1, &["y"]));

        let result = compare_snapshots_with_graphs(&db, &db, &graph_before, &graph_after).unwrap();

        assert_eq!(result.cycle_changes.len(), 1);
        // Files should be sorted: a.ts before z.ts
        assert_eq!(result.cycle_changes[0].files[0], PathBuf::from("a.ts"));
        assert_eq!(result.cycle_changes[0].files[1], PathBuf::from("z.ts"));
    }
}
