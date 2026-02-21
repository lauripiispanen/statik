use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::file_graph::FileGraph;
use crate::model::graph::SymbolGraph;
use crate::model::{FileId, SymbolId, SymbolKind, Visibility};

use super::{compute_confidence, Confidence, Limitation};

/// Scope of dead code analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadCodeScope {
    Files,
    Exports,
    Both,
    Symbols,
}

/// A dead file: a file that is never imported from any entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadFile {
    pub file_id: FileId,
    pub path: PathBuf,
    pub confidence: Confidence,
}

/// A dead export: an exported symbol that is never imported anywhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadExport {
    pub file_id: FileId,
    pub path: PathBuf,
    pub export_name: String,
    pub line: usize,
    pub confidence: Confidence,
    pub kind: String,
}

/// A dead symbol: a symbol not reachable from any entry point via intra-file references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub symbol_id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub confidence: Confidence,
}

/// Result of symbol-level dead code analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbolResult {
    pub dead_symbols: Vec<DeadSymbol>,
    pub confidence: Confidence,
    pub limitations: Vec<Limitation>,
    pub summary: DeadSymbolSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbolSummary {
    pub total_symbols: usize,
    pub dead_symbols: usize,
    pub entry_point_symbols: usize,
    pub resolved_references: usize,
    pub unresolved_references: usize,
}

/// Result of dead code analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeResult {
    pub dead_files: Vec<DeadFile>,
    pub dead_exports: Vec<DeadExport>,
    pub confidence: Confidence,
    pub limitations: Vec<Limitation>,
    pub summary: DeadCodeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeSummary {
    pub total_files: usize,
    pub dead_files: usize,
    pub total_exports: usize,
    pub dead_exports: usize,
    pub entry_points: usize,
    pub files_with_unresolvable_imports: usize,
}

/// Detect dead code in the project.
///
/// Dead files: BFS from entry points, unreachable files are dead.
/// Dead exports: exports never imported by any other file.
///
/// Precision over recall: we never report entry point exports as dead.
/// If confidence is low, we say so rather than asserting.
pub fn detect_dead_code(graph: &FileGraph, scope: DeadCodeScope) -> DeadCodeResult {
    let mut dead_files = Vec::new();
    let mut dead_exports = Vec::new();

    let entry_points = graph.entry_points();
    let all_files = graph.all_file_ids();
    let total_files = all_files.len();

    // Count total imports and unresolved for confidence calculation
    let total_imports: usize = graph.imports.values().map(|v| v.len()).sum();
    let unresolved_count = graph.unresolved.len();
    let has_wildcards = graph.files.values().any(|info| {
        info.exports
            .iter()
            .any(|e| e.is_reexport && e.exported_name == "*")
    });

    // BFS to find all reachable files from entry points
    let reachable = bfs_reachable(graph, &entry_points);

    // Pre-compute set of files with unresolved imports (avoids O(N*M) linear scans)
    let unresolved_file_set = graph.files_with_unresolved_imports();
    let files_with_unresolvable = unresolved_file_set.len();

    // Dead file detection
    if scope == DeadCodeScope::Files || scope == DeadCodeScope::Both {
        let entry_set: HashSet<FileId> = entry_points.iter().copied().collect();

        for file_id in &all_files {
            // Skip entry points -- they are roots, not dead
            if entry_set.contains(file_id) {
                continue;
            }

            if !reachable.contains(file_id) {
                let info = &graph.files[file_id];

                // Determine confidence for this specific finding
                let file_confidence = if unresolved_count == 0 {
                    Confidence::Certain
                } else if unresolved_file_set.contains(file_id) {
                    // This file itself has unresolved imports, so something
                    // might be importing it that we can't see
                    Confidence::Medium
                } else if files_with_unresolvable > 0 {
                    // Some other files have unresolved imports that might
                    // point to this file
                    Confidence::High
                } else {
                    Confidence::Certain
                };

                dead_files.push(DeadFile {
                    file_id: *file_id,
                    path: info.path.clone(),
                    confidence: file_confidence,
                });
            }
        }
    }

    // Dead export detection
    if scope == DeadCodeScope::Exports || scope == DeadCodeScope::Both {
        // Collect all imported names per file
        let mut imported_names: HashSet<(FileId, String)> = HashSet::new();
        for edges in graph.imports.values() {
            for edge in edges {
                for name in &edge.imported_names {
                    imported_names.insert((edge.to, name.clone()));
                }
            }
        }

        // Propagate imported names through re-export chains.
        // If file B has `export * from './A'` and someone imports `foo` from B,
        // then (A, "foo") should also be marked as used.
        // Similarly for `export { foo } from './A'`, propagate just `foo`.
        propagate_through_reexports(graph, &mut imported_names);

        // Check each file's exports
        let entry_set: HashSet<FileId> = entry_points.iter().copied().collect();
        for (file_id, info) in &graph.files {
            // Never report entry point exports as dead
            if entry_set.contains(file_id) {
                continue;
            }

            for export in &info.exports {
                let is_used = imported_names.contains(&(*file_id, export.exported_name.clone()));
                // Also check if "default" is imported as the exported name for default exports
                let is_default_used = export.is_default
                    && imported_names.contains(&(*file_id, "default".to_string()));

                if !is_used && !is_default_used {
                    // Don't report re-exports as dead here -- they are pass-through
                    if export.is_reexport {
                        continue;
                    }

                    let export_confidence = if unresolved_count == 0 {
                        Confidence::Certain
                    } else {
                        Confidence::High
                    };

                    dead_exports.push(DeadExport {
                        file_id: *file_id,
                        path: info.path.clone(),
                        export_name: export.exported_name.clone(),
                        line: 0, // TODO: get line from export record
                        confidence: export_confidence,
                        kind: "export".to_string(),
                    });
                }
            }
        }
    }

    // Sort results for deterministic output
    dead_files.sort_by(|a, b| a.path.cmp(&b.path));
    dead_exports.sort_by(|a, b| a.path.cmp(&b.path).then(a.export_name.cmp(&b.export_name)));

    let overall_confidence = compute_confidence(total_imports, unresolved_count, has_wildcards);

    let mut limitations = Vec::new();
    if unresolved_count > 0 {
        limitations.push(Limitation {
            description: format!("{} imports could not be resolved", unresolved_count),
            count: unresolved_count,
        });
    }
    if files_with_unresolvable > 0 {
        limitations.push(Limitation {
            description: format!(
                "{} files have unresolvable imports",
                files_with_unresolvable
            ),
            count: files_with_unresolvable,
        });
    }

    let summary = DeadCodeSummary {
        total_files,
        dead_files: dead_files.len(),
        total_exports: graph.files.values().map(|f| f.exports.len()).sum(),
        dead_exports: dead_exports.len(),
        entry_points: entry_points.len(),
        files_with_unresolvable_imports: files_with_unresolvable,
    };

    DeadCodeResult {
        dead_files,
        dead_exports,
        confidence: overall_confidence,
        limitations,
        summary,
    }
}

/// Propagate used names through re-export chains.
///
/// If file B has `export * from './A'` (resolved to FileId A) and someone
/// imports name `foo` from B, then `(A, "foo")` is also used. For named
/// re-exports `export { foo } from './A'`, only propagate `foo`.
///
/// Uses a worklist to handle chained re-exports (A re-exports from B which
/// re-exports from C).
fn propagate_through_reexports(
    graph: &FileGraph,
    imported_names: &mut HashSet<(FileId, String)>,
) {
    // Build a re-export map: for each file, which files it re-exports from and how
    // (file_id) -> Vec<(target_file_id, exported_name, is_wildcard)>
    // We derive target_file_id from import edges that match the re-export source_path.
    let mut reexport_targets: Vec<(FileId, FileId, String)> = Vec::new();

    for (file_id, info) in &graph.files {
        for export in &info.exports {
            if !export.is_reexport {
                continue;
            }
            // Find the import edge from this file that corresponds to this re-export
            if let Some(edges) = graph.imports.get(file_id) {
                for edge in edges {
                    if export.exported_name == "*"
                        && edge.imported_names.iter().any(|n| n == "*")
                    {
                        // Wildcard re-export: only match edges created by re-exports
                        // (which have "*" in imported_names), not unrelated imports
                        reexport_targets.push((*file_id, edge.to, "*".to_string()));
                    } else if edge.imported_names.contains(&export.exported_name) {
                        // Named re-export: match the export to its import edge
                        reexport_targets.push((
                            *file_id,
                            edge.to,
                            export.exported_name.clone(),
                        ));
                    }
                }
            }
        }
    }

    // Worklist: propagate until no new names are added
    let mut changed = true;
    let mut iteration = 0;
    while changed && iteration < 100 {
        // Safety limit to prevent infinite loops in pathological cases
        changed = false;
        iteration += 1;

        for (barrel_id, target_id, reexport_name) in &reexport_targets {
            if reexport_name == "*" {
                // Wildcard: any name imported from the barrel propagates to target
                let names_from_barrel: Vec<String> = imported_names
                    .iter()
                    .filter(|(fid, _)| fid == barrel_id)
                    .map(|(_, name)| name.clone())
                    .collect();

                for name in names_from_barrel {
                    if imported_names.insert((*target_id, name)) {
                        changed = true;
                    }
                }
            } else {
                // Named: if the name is imported from the barrel, propagate it
                if imported_names.contains(&(*barrel_id, reexport_name.clone())) {
                    if imported_names.insert((*target_id, reexport_name.clone())) {
                        changed = true;
                    }
                }
            }
        }
    }
}

/// BFS from entry points, following import edges forward.
/// Returns the set of all files reachable from any entry point.
/// Detect dead symbols using the symbol-level reference graph.
///
/// Entry point symbols are exported symbols from entry point files.
/// BFS through intra-file references from entry points.
/// Unreachable symbols (excluding Import/Export/Package synthetic kinds) are dead.
pub fn detect_dead_symbols(
    symbol_graph: &SymbolGraph,
    file_graph: &FileGraph,
) -> DeadSymbolResult {
    // Determine entry point file IDs from the file graph
    let entry_file_ids: HashSet<FileId> = file_graph.entry_points().into_iter().collect();

    // Entry point symbols: exported symbols in entry point files, plus all
    // symbols in entry point files that are public (conservative approach)
    let mut entry_symbols: Vec<SymbolId> = Vec::new();

    for (&file_id, symbol_ids) in &symbol_graph.file_symbols {
        if entry_file_ids.contains(&file_id) {
            // All public symbols in entry point files are entry points
            for &sym_id in symbol_ids {
                if let Some(symbol) = symbol_graph.symbols.get(&sym_id) {
                    if symbol.visibility == Visibility::Public {
                        entry_symbols.push(sym_id);
                    }
                }
            }
        } else {
            // For non-entry-point files, exported symbols that are imported by entry files
            // are entry points. But for simplicity, any exported public symbol connected
            // through the file graph is an entry.
            // We use exported symbols as seeds.
            if let Some(exports) = symbol_graph.exports.get(&file_id) {
                for export in exports {
                    entry_symbols.push(export.symbol);
                }
            }
        }
    }

    // Use the symbol graph's reachable_from to find all reachable symbols
    let reachable = symbol_graph.reachable_from(&entry_symbols);

    // Count resolved vs unresolved references
    let total_refs = symbol_graph.references.len();
    let resolved_refs = symbol_graph
        .references
        .iter()
        .filter(|r| r.target.0 < u64::MAX - 1_000_000)
        .count();
    let unresolved_refs = total_refs - resolved_refs;

    // Find dead symbols: not reachable from any entry point
    // Exclude synthetic kinds (Import, Export, Package) that are not user-defined code
    let skip_kinds = [
        SymbolKind::Import,
        SymbolKind::Export,
        SymbolKind::Package,
    ];

    let mut dead_symbols = Vec::new();
    let file_paths: std::collections::HashMap<FileId, &PathBuf> = symbol_graph
        .files
        .iter()
        .map(|(id, f)| (*id, &f.path))
        .collect();

    for symbol in symbol_graph.symbols.values() {
        if skip_kinds.contains(&symbol.kind) {
            continue;
        }
        if !reachable.contains(&symbol.id) {
            let file_path = file_paths
                .get(&symbol.file)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| format!("file:{}", symbol.file.0));

            dead_symbols.push(DeadSymbol {
                symbol_id: symbol.id,
                name: symbol.name.clone(),
                qualified_name: symbol.qualified_name.clone(),
                kind: symbol.kind.as_str().to_string(),
                file: file_path,
                line: symbol.line_span.start.line,
                confidence: if unresolved_refs == 0 {
                    Confidence::High
                } else {
                    Confidence::Medium
                },
            });
        }
    }

    // Sort by file then name
    dead_symbols.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));

    let total_symbols = symbol_graph
        .symbols
        .values()
        .filter(|s| !skip_kinds.contains(&s.kind))
        .count();

    let confidence = if unresolved_refs == 0 {
        Confidence::High
    } else {
        // Unresolved references are expected (cross-file refs not yet tracked).
        // Medium confidence: results are directionally correct but may have
        // false positives for symbols called from other files.
        Confidence::Medium
    };

    let mut limitations = Vec::new();
    if unresolved_refs > 0 {
        limitations.push(Limitation {
            description: format!(
                "{}/{} references unresolved (cross-file references not yet tracked)",
                unresolved_refs, total_refs
            ),
            count: unresolved_refs,
        });
    }

    DeadSymbolResult {
        summary: DeadSymbolSummary {
            total_symbols,
            dead_symbols: dead_symbols.len(),
            entry_point_symbols: entry_symbols.len(),
            resolved_references: resolved_refs,
            unresolved_references: unresolved_refs,
        },
        dead_symbols,
        confidence,
        limitations,
    }
}

fn bfs_reachable(graph: &FileGraph, entry_points: &[FileId]) -> HashSet<FileId> {
    let mut visited = HashSet::new();
    let mut queue: VecDeque<FileId> = entry_points.iter().copied().collect();

    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        for target in graph.direct_imports(current) {
            if !visited.contains(&target) {
                queue.push_back(target);
            }
        }
    }

    visited
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::file_graph::{FileImport, FileInfo};
    use crate::model::{
        FileRecord, Language, LineSpan, Position, RefKind, Reference, ReferenceId, Span, Symbol,
    };

    fn make_file(id: u64, path: &str, is_entry: bool) -> FileInfo {
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(path),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: is_entry,
        }
    }

    fn make_file_with_exports(
        id: u64,
        path: &str,
        is_entry: bool,
        export_names: &[&str],
    ) -> FileInfo {
        use crate::model::{ExportRecord, SymbolId};
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(path),
            language: Language::TypeScript,
            exports: export_names
                .iter()
                .enumerate()
                .map(|(i, name)| ExportRecord {
                    file: FileId(id),
                    symbol: SymbolId(id * 100 + i as u64),
                    exported_name: name.to_string(),
                    is_default: *name == "default",
                    is_reexport: false,
                    is_type_only: false,
                    source_path: None,
                })
                .collect(),
            is_entry_point: is_entry,
        }
    }

    fn make_edge(from: u64, to: u64, names: &[&str]) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: names.iter().map(|s| s.to_string()).collect(),
            is_type_only: false,
            line: 1,
        }
    }

    #[test]
    fn test_no_dead_files_in_connected_project() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/utils.ts", false));
        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert!(result.dead_files.is_empty());
        assert_eq!(result.confidence, Confidence::Certain);
    }

    #[test]
    fn test_detect_orphaned_file() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/utils.ts", false));
        graph.add_file(make_file(3, "src/orphan.ts", false)); // not imported

        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert_eq!(result.dead_files.len(), 1);
        assert_eq!(result.dead_files[0].path, PathBuf::from("src/orphan.ts"));
        assert_eq!(result.dead_files[0].confidence, Confidence::Certain);
    }

    #[test]
    fn test_entry_points_never_reported_dead() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        // Entry point with no imports is NOT dead
        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert!(result.dead_files.is_empty());
    }

    #[test]
    fn test_transitive_reachability() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/a.ts", false));
        graph.add_file(make_file(3, "src/b.ts", false));
        graph.add_file(make_file(4, "src/c.ts", false));

        // index -> a -> b -> c (all reachable)
        graph.add_import(make_edge(1, 2, &["a"]));
        graph.add_import(make_edge(2, 3, &["b"]));
        graph.add_import(make_edge(3, 4, &["c"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert!(result.dead_files.is_empty());
    }

    #[test]
    fn test_dead_export_detection() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_with_exports(1, "src/index.ts", true, &["main"]));
        graph.add_file(make_file_with_exports(
            2,
            "src/utils.ts",
            false,
            &["used_fn", "unused_fn"],
        ));

        graph.add_import(make_edge(1, 2, &["used_fn"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        assert_eq!(result.dead_exports.len(), 1);
        assert_eq!(result.dead_exports[0].export_name, "unused_fn");
    }

    #[test]
    fn test_entry_point_exports_not_dead() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_with_exports(
            1,
            "src/index.ts",
            true,
            &["main", "config"],
        ));
        // Entry point exports are never dead (may be consumed externally)
        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        assert!(result.dead_exports.is_empty());
    }

    #[test]
    fn test_both_scope() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_with_exports(1, "src/index.ts", true, &["main"]));
        graph.add_file(make_file_with_exports(
            2,
            "src/utils.ts",
            false,
            &["helper"],
        ));
        graph.add_file(make_file(3, "src/orphan.ts", false));

        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Both);
        assert_eq!(result.dead_files.len(), 1); // orphan.ts
        assert!(result.dead_exports.is_empty()); // helper is used
    }

    #[test]
    fn test_empty_graph() {
        let graph = FileGraph::new();
        let result = detect_dead_code(&graph, DeadCodeScope::Both);
        assert!(result.dead_files.is_empty());
        assert!(result.dead_exports.is_empty());
        assert_eq!(result.summary.total_files, 0);
    }

    #[test]
    fn test_circular_dependency_reachable() {
        // Files in a cycle are all reachable if any is reachable from an entry point
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/a.ts", false));
        graph.add_file(make_file(3, "src/b.ts", false));

        graph.add_import(make_edge(1, 2, &["a"]));
        graph.add_import(make_edge(2, 3, &["b"]));
        graph.add_import(make_edge(3, 2, &["a"])); // cycle: a <-> b

        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert!(result.dead_files.is_empty());
    }

    #[test]
    fn test_summary_counts() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_with_exports(1, "src/index.ts", true, &["main"]));
        graph.add_file(make_file_with_exports(
            2,
            "src/a.ts",
            false,
            &["foo", "bar"],
        ));
        graph.add_file(make_file(3, "src/orphan.ts", false));

        graph.add_import(make_edge(1, 2, &["foo"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Both);
        assert_eq!(result.summary.total_files, 3);
        assert_eq!(result.summary.dead_files, 1);
        assert_eq!(result.summary.total_exports, 3); // main + foo + bar
        assert_eq!(result.summary.dead_exports, 1); // bar
        assert_eq!(result.summary.entry_points, 1);
    }

    #[test]
    fn test_reexported_symbol_not_flagged_dead() {
        // A barrel file re-exports a symbol that is consumed by the entry point.
        // The re-export should NOT appear as a dead export.
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();

        // Entry point imports "helper" from barrel
        graph.add_file(make_file(1, "src/index.ts", true));

        // Barrel file re-exports "helper" from utils
        let barrel = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/barrel.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "helper".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./utils".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        // Re-exports should be skipped by the dead export detector
        assert!(
            result.dead_exports.is_empty(),
            "re-exported symbol 'helper' that is consumed should not be flagged as dead, got: {:?}",
            result
                .dead_exports
                .iter()
                .map(|e| &e.export_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_unconsumed_reexport_not_flagged_dead() {
        // A re-export that nobody imports should still NOT be flagged as dead.
        // The dead export detector intentionally skips re-exports because they
        // are pass-through: the original export in the source file is where
        // liveness should be checked, not the barrel re-export.
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();

        graph.add_file(make_file(1, "src/index.ts", true));

        // Barrel file re-exports "helper" but nobody imports it from here
        let barrel = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/barrel.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "helper".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./utils".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        // Entry point imports barrel for some other reason but NOT "helper"
        graph.add_import(make_edge(1, 2, &["somethingElse"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        // The unconsumed re-export should NOT appear as dead -- re-exports are skipped
        assert!(
            result.dead_exports.is_empty(),
            "unconsumed re-export 'helper' should not be flagged as dead, got: {:?}",
            result
                .dead_exports
                .iter()
                .map(|e| &e.export_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_multiple_entry_points() {
        // A project can have multiple entry points (e.g., index.ts and cli.ts).
        // Files reachable from ANY entry point should not be flagged dead.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/cli.ts", true));
        graph.add_file(make_file(3, "src/shared.ts", false));
        graph.add_file(make_file(4, "src/web-only.ts", false));
        graph.add_file(make_file(5, "src/cli-only.ts", false));
        graph.add_file(make_file(6, "src/orphan.ts", false));

        graph.add_import(make_edge(1, 3, &["shared"])); // index -> shared
        graph.add_import(make_edge(1, 4, &["web"])); // index -> web-only
        graph.add_import(make_edge(2, 3, &["shared"])); // cli -> shared
        graph.add_import(make_edge(2, 5, &["cli"])); // cli -> cli-only

        let result = detect_dead_code(&graph, DeadCodeScope::Files);
        assert_eq!(result.dead_files.len(), 1, "only orphan.ts should be dead");
        assert_eq!(result.dead_files[0].path, PathBuf::from("src/orphan.ts"));
    }

    #[test]
    fn test_wildcard_reexport_propagates_used_names() {
        // Entry -> Barrel (export * from './utils') -> Utils (export helper)
        // If entry imports "helper" from barrel, utils.helper should be used.
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();

        // Entry point imports "helper" from barrel
        graph.add_file(make_file(1, "src/index.ts", true));

        // Barrel file: export * from './utils'
        let barrel = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/barrel.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "*".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./utils".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        // Utils file: export function helper()
        graph.add_file(make_file_with_exports(3, "src/utils.ts", false, &["helper", "unused_fn"]));

        // Entry imports "helper" from barrel
        graph.add_import(make_edge(1, 2, &["helper"]));
        // Barrel re-export creates edge to utils
        graph.add_import(make_edge(2, 3, &["*"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        let dead_names: Vec<&str> = result.dead_exports.iter().map(|e| e.export_name.as_str()).collect();
        assert!(
            !dead_names.contains(&"helper"),
            "helper should NOT be dead (used via barrel re-export), dead: {:?}",
            dead_names
        );
        assert!(
            dead_names.contains(&"unused_fn"),
            "unused_fn should be dead, dead: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_named_reexport_propagates_used_names() {
        // Entry -> Barrel (export { helper } from './utils') -> Utils
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));

        // Barrel: export { helper } from './utils'
        let barrel = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/barrel.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "helper".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./utils".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        graph.add_file(make_file_with_exports(3, "src/utils.ts", false, &["helper", "unused_fn"]));

        graph.add_import(make_edge(1, 2, &["helper"]));
        graph.add_import(make_edge(2, 3, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        let dead_names: Vec<&str> = result.dead_exports.iter().map(|e| e.export_name.as_str()).collect();
        assert!(
            !dead_names.contains(&"helper"),
            "helper should NOT be dead (used via named re-export), dead: {:?}",
            dead_names
        );
        assert!(
            dead_names.contains(&"unused_fn"),
            "unused_fn should be dead, dead: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_chained_reexport_propagation() {
        // Entry -> A (export * from './B') -> B (export * from './C') -> C (export foo)
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));

        // A: export * from './B'
        let file_a = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/a.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "*".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./b".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(file_a);

        // B: export * from './C'
        let file_b = FileInfo {
            id: FileId(3),
            path: PathBuf::from("src/b.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(3),
                symbol: SymbolId(300),
                exported_name: "*".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./c".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(file_b);

        // C: export function foo
        graph.add_file(make_file_with_exports(4, "src/c.ts", false, &["foo"]));

        graph.add_import(make_edge(1, 2, &["foo"]));
        graph.add_import(make_edge(2, 3, &["*"]));
        graph.add_import(make_edge(3, 4, &["*"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports);
        let dead_names: Vec<&str> = result.dead_exports.iter().map(|e| e.export_name.as_str()).collect();
        assert!(
            !dead_names.contains(&"foo"),
            "foo should NOT be dead (used via chained re-exports), dead: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_has_wildcards_affects_confidence() {
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));

        let barrel = FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/barrel.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "*".to_string(),
                is_default: false,
                is_reexport: true,
                is_type_only: false,
                source_path: Some("./utils".to_string()),
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);
        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Both);
        // With wildcards and no unresolved, confidence should be High (not Certain)
        assert_eq!(result.confidence, Confidence::High);
    }

    #[test]
    fn test_detect_dead_symbols_basic() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        // File 1 is entry point with two symbols: main (public) and helper (public)
        // main calls helper. dead_fn is never called.
        file_graph.add_file(make_file(1, "src/index.ts", true));

        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                Symbol {
                    id: SymbolId(1),
                    name: "main".to_string(),
                    qualified_name: "main".to_string(),
                    kind: SymbolKind::Function,
                    file: FileId(1),
                    span: Span { start: 0, end: 10 },
                    line_span: LineSpan {
                        start: Position { line: 1, column: 0 },
                        end: Position { line: 1, column: 10 },
                    },
                    parent: None,
                    visibility: Visibility::Public,
                    signature: None,
                },
                Symbol {
                    id: SymbolId(2),
                    name: "helper".to_string(),
                    qualified_name: "helper".to_string(),
                    kind: SymbolKind::Function,
                    file: FileId(1),
                    span: Span { start: 20, end: 30 },
                    line_span: LineSpan {
                        start: Position { line: 2, column: 0 },
                        end: Position { line: 2, column: 10 },
                    },
                    parent: None,
                    visibility: Visibility::Public,
                    signature: None,
                },
                Symbol {
                    id: SymbolId(3),
                    name: "dead_fn".to_string(),
                    qualified_name: "dead_fn".to_string(),
                    kind: SymbolKind::Function,
                    file: FileId(1),
                    span: Span { start: 40, end: 50 },
                    line_span: LineSpan {
                        start: Position { line: 3, column: 0 },
                        end: Position { line: 3, column: 10 },
                    },
                    parent: None,
                    visibility: Visibility::Private,
                    signature: None,
                },
            ],
            references: vec![Reference {
                id: ReferenceId(1),
                source: SymbolId(1),
                target: SymbolId(2),
                kind: RefKind::Call,
                file: FileId(1),
                span: Span { start: 5, end: 15 },
                line_span: LineSpan {
                    start: Position { line: 1, column: 5 },
                    end: Position { line: 1, column: 15 },
                },
            }],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };

        sym_graph.add_file(FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/index.ts"),
            mtime: 0,
            language: Language::TypeScript,
        });
        sym_graph.add_parse_result(result);

        let dead_result = detect_dead_symbols(&sym_graph, &file_graph);

        // dead_fn should be dead (private, never referenced)
        let dead_names: Vec<&str> = dead_result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            dead_names.contains(&"dead_fn"),
            "dead_fn should be dead, got: {:?}",
            dead_names
        );

        // main and helper should NOT be dead
        assert!(
            !dead_names.contains(&"main"),
            "main should NOT be dead"
        );
        assert!(
            !dead_names.contains(&"helper"),
            "helper should NOT be dead (called by main)"
        );
    }

    fn make_sym(
        id: u64,
        name: &str,
        kind: SymbolKind,
        file: u64,
        vis: Visibility,
    ) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
            file: FileId(file),
            span: Span { start: 0, end: 10 },
            line_span: LineSpan {
                start: Position { line: id as usize, column: 0 },
                end: Position { line: id as usize, column: 10 },
            },
            parent: None,
            visibility: vis,
            signature: None,
        }
    }

    fn make_ref(id: u64, source: u64, target: u64, kind: RefKind, file: u64) -> Reference {
        Reference {
            id: ReferenceId(id),
            source: SymbolId(source),
            target: SymbolId(target),
            kind,
            file: FileId(file),
            span: Span { start: 0, end: 5 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position { line: 1, column: 5 },
            },
        }
    }

    fn make_file_record(id: u64, path: &str) -> FileRecord {
        FileRecord {
            id: FileId(id),
            path: PathBuf::from(path),
            mtime: 0,
            language: Language::TypeScript,
        }
    }

    #[test]
    fn test_dead_symbols_empty_graph() {
        use crate::model::graph::SymbolGraph;

        let sym_graph = SymbolGraph::new();
        let file_graph = FileGraph::new();

        let result = detect_dead_symbols(&sym_graph, &file_graph);
        assert!(result.dead_symbols.is_empty());
        assert_eq!(result.summary.total_symbols, 0);
        assert_eq!(result.summary.entry_point_symbols, 0);
    }

    #[test]
    fn test_dead_symbols_multi_file_private_unreachable() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        // File 1 (entry): public fn main
        // File 2 (non-entry): exported fn api_handler, private fn internal_helper (called by api_handler),
        //                      private fn dead_internal (never called)
        file_graph.add_file(make_file(1, "src/index.ts", true));
        file_graph.add_file(make_file(2, "src/utils.ts", false));
        file_graph.add_import(make_edge(1, 2, &["api_handler"]));

        sym_graph.add_file(make_file_record(1, "src/index.ts"));
        sym_graph.add_file(make_file_record(2, "src/utils.ts"));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        // File 2: exported symbol acts as entry point for symbol-level analysis
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_sym(10, "api_handler", SymbolKind::Function, 2, Visibility::Public),
                make_sym(11, "internal_helper", SymbolKind::Function, 2, Visibility::Private),
                make_sym(12, "dead_internal", SymbolKind::Function, 2, Visibility::Private),
            ],
            references: vec![
                make_ref(1, 10, 11, RefKind::Call, 2), // api_handler calls internal_helper
            ],
            imports: vec![],
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(10),
                exported_name: "api_handler".to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            }],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph);
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(
            dead_names.contains(&"dead_internal"),
            "dead_internal should be dead (private, never called), dead: {:?}",
            dead_names
        );
        assert!(
            !dead_names.contains(&"api_handler"),
            "api_handler should NOT be dead (exported)"
        );
        assert!(
            !dead_names.contains(&"internal_helper"),
            "internal_helper should NOT be dead (called by api_handler)"
        );
    }

    #[test]
    fn test_dead_symbols_skips_synthetic_kinds() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                make_sym(2, "importSym", SymbolKind::Import, 1, Visibility::Public),
                make_sym(3, "exportSym", SymbolKind::Export, 1, Visibility::Public),
                make_sym(4, "pkgSym", SymbolKind::Package, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph);
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        // Synthetic kinds should not appear in dead symbols list at all
        assert!(!dead_names.contains(&"importSym"));
        assert!(!dead_names.contains(&"exportSym"));
        assert!(!dead_names.contains(&"pkgSym"));
        // total_symbols should exclude synthetic kinds
        assert_eq!(result.summary.total_symbols, 1); // only main
    }

    #[test]
    fn test_dead_symbols_transitive_reachability() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // main -> a -> b -> c (chain), d is isolated
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                make_sym(2, "a", SymbolKind::Function, 1, Visibility::Private),
                make_sym(3, "b", SymbolKind::Function, 1, Visibility::Private),
                make_sym(4, "c", SymbolKind::Function, 1, Visibility::Private),
                make_sym(5, "d", SymbolKind::Function, 1, Visibility::Private),
            ],
            references: vec![
                make_ref(1, 1, 2, RefKind::Call, 1),
                make_ref(2, 2, 3, RefKind::Call, 1),
                make_ref(3, 3, 4, RefKind::Call, 1),
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph);
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert_eq!(dead_names, vec!["d"], "only d should be dead, got: {:?}", dead_names);
    }

    #[test]
    fn test_dead_symbols_confidence_with_unresolved() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Create a reference with a placeholder target (unresolved)
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                make_sym(2, "maybe_dead", SymbolKind::Function, 1, Visibility::Private),
            ],
            references: vec![
                // Unresolved reference: target has a placeholder ID
                Reference {
                    id: ReferenceId(1),
                    source: SymbolId(1),
                    target: SymbolId(u64::MAX),
                    kind: RefKind::Call,
                    file: FileId(1),
                    span: Span { start: 0, end: 5 },
                    line_span: LineSpan {
                        start: Position { line: 1, column: 0 },
                        end: Position { line: 1, column: 5 },
                    },
                },
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph);

        // With unresolved references, confidence should be Medium
        assert_eq!(result.confidence, Confidence::Medium);
        assert_eq!(result.summary.unresolved_references, 1);
        assert!(!result.limitations.is_empty());
    }

    #[test]
    fn test_dead_symbols_inheritance_reachability() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // main uses UserClass, UserClass extends BaseClass
        // BaseClass should be reachable via inheritance
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                make_sym(2, "UserClass", SymbolKind::Class, 1, Visibility::Public),
                make_sym(3, "BaseClass", SymbolKind::Class, 1, Visibility::Private),
                make_sym(4, "UnusedClass", SymbolKind::Class, 1, Visibility::Private),
            ],
            references: vec![
                make_ref(1, 1, 2, RefKind::Call, 1),          // main -> UserClass
                make_ref(2, 2, 3, RefKind::Inheritance, 1),    // UserClass extends BaseClass
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph);
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(!dead_names.contains(&"BaseClass"), "BaseClass reached via inheritance");
        assert!(dead_names.contains(&"UnusedClass"), "UnusedClass should be dead");
    }
}
