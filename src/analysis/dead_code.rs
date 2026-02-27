use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::analysis::linker::LinkingResult;
use crate::model::file_graph::FileGraph;
use crate::model::graph::SymbolGraph;
use crate::model::{FileId, RefKind, SymbolId, SymbolKind, Visibility};

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
pub fn detect_dead_code(
    graph: &FileGraph,
    scope: DeadCodeScope,
    seed_all_file_ids: &HashSet<FileId>,
) -> DeadCodeResult {
    let mut dead_files = Vec::new();
    let mut dead_exports = Vec::new();

    let mut entry_points = graph.entry_points();
    // Treat seed_all files as additional entry points for BFS so they
    // and their imports become reachable (test fixtures, test dirs, etc.).
    for &fid in seed_all_file_ids {
        if !entry_points.contains(&fid) {
            entry_points.push(fid);
        }
    }
    let all_files = graph.all_file_ids();
    let total_files = all_files.len();

    // Count total imports and truly unresolved for confidence calculation.
    // External imports (third-party libs) are correctly classified and should not reduce confidence.
    let total_imports: usize = graph.imports.values().map(|v| v.len()).sum();
    let unresolved_count = graph.truly_unresolved_count();
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

                // Determine confidence for this specific finding.
                // A file's own unresolved outgoing imports (e.g. java.util.*)
                // don't affect whether it's reachable — only incoming edges
                // matter. Confidence is reduced only at the graph level: if
                // other files have unresolved imports, one of them might
                // reference this file.
                let file_confidence = if unresolved_count == 0 {
                    Confidence::Certain
                } else if files_with_unresolvable > 0 {
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

        // Module-path imports: when the imported name matches the target file's
        // stem, mark all exports as used (e.g. Rust's `use crate::cli::commands`).
        for edges in graph.imports.values() {
            for edge in edges {
                if let Some(target_info) = graph.files.get(&edge.to) {
                    if super::supports_module_stem_import(target_info.language) {
                        let file_stem = target_info
                            .path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        for name in &edge.imported_names {
                            if name == file_stem {
                                for export in &target_info.exports {
                                    imported_names
                                        .insert((edge.to, export.exported_name.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Propagate imported names through re-export chains.
        // If file B has `export * from './A'` and someone imports `foo` from B,
        // then (A, "foo") should also be marked as used.
        // Similarly for `export { foo } from './A'`, propagate just `foo`.
        propagate_through_reexports(graph, &mut imported_names);

        // Build a map of module re-export names per file: exports that correspond
        // to `pub mod foo;` declarations. These create an import edge with
        // is_mod_declaration=true to the module file. If the target file is reachable,
        // the re-export is structurally necessary and should not be flagged dead.
        let mut mod_reexport_targets: std::collections::HashMap<(FileId, String), FileId> =
            std::collections::HashMap::new();
        for edges in graph.imports.values() {
            for edge in edges {
                if edge.is_mod_declaration {
                    for name in &edge.imported_names {
                        mod_reexport_targets.insert((edge.from, name.clone()), edge.to);
                    }
                }
            }
        }

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

                    // Skip mod re-exports (`pub mod foo;`) when the target module
                    // file is reachable — the declaration is structurally necessary.
                    if let Some(target_file) =
                        mod_reexport_targets.get(&(*file_id, export.exported_name.clone()))
                    {
                        if reachable.contains(target_file) {
                            continue;
                        }
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
                        line: export.line,
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
fn propagate_through_reexports(graph: &FileGraph, imported_names: &mut HashSet<(FileId, String)>) {
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
                    if export.exported_name == "*" && edge.imported_names.iter().any(|n| n == "*") {
                        // Wildcard re-export: only match edges created by re-exports
                        // (which have "*" in imported_names), not unrelated imports
                        reexport_targets.push((*file_id, edge.to, "*".to_string()));
                    } else if edge.imported_names.contains(&export.exported_name) {
                        // Named re-export: match the export to its import edge
                        reexport_targets.push((*file_id, edge.to, export.exported_name.clone()));
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
                if imported_names.contains(&(*barrel_id, reexport_name.clone()))
                    && imported_names.insert((*target_id, reexport_name.clone()))
                {
                    changed = true;
                }
            }
        }
    }
}

/// Detect dead symbols using the symbol-level reference graph.
///
/// Entry point symbols are exported symbols from entry point files.
/// For non-entry files, only exports that are actually imported (per linker results)
/// are seeded as entry points. BFS through intra-file references from entry points.
/// Unreachable symbols (excluding Import/Export/Package synthetic kinds) are dead.
///
/// `seed_all_file_ids` contains files where ALL symbols should be seeded as alive,
/// regardless of visibility. This covers test directories (from language semantics)
/// and user-configured patterns (e.g., test fixtures).
pub fn detect_dead_symbols(
    symbol_graph: &SymbolGraph,
    file_graph: &FileGraph,
    linker_result: &LinkingResult,
    seed_all_file_ids: &HashSet<FileId>,
) -> DeadSymbolResult {
    // Determine entry point file IDs from the file graph
    let entry_file_ids: HashSet<FileId> = file_graph.entry_points().into_iter().collect();

    // Build set of symbols that are targets of cross-file linker references.
    // These are the exports actually imported by other files.
    let linker_targets: HashSet<SymbolId> = linker_result
        .references
        .iter()
        .map(|xref| xref.target_symbol)
        .collect();

    // Entry point symbols: all public symbols in entry point files,
    // plus exports in non-entry files that are actually imported (linker targets).
    let mut entry_symbols: Vec<SymbolId> = Vec::new();

    // Build a lookup for checking if a symbol is inside a "tests" module
    let is_in_test_module = |sym: &crate::model::Symbol| -> bool {
        let mut parent = sym.parent;
        while let Some(pid) = parent {
            if let Some(p) = symbol_graph.symbols.get(&pid) {
                if p.kind == SymbolKind::Module && p.name == "tests" {
                    return true;
                }
                parent = p.parent;
            } else {
                break;
            }
        }
        false
    };

    for (&file_id, symbol_ids) in &symbol_graph.file_symbols {
        for &sym_id in symbol_ids {
            if let Some(symbol) = symbol_graph.symbols.get(&sym_id) {
                // Symbols inside `mod tests` blocks are always test infrastructure,
                // regardless of whether the file is an entry point.
                if is_in_test_module(symbol) {
                    entry_symbols.push(sym_id);
                    continue;
                }
            }
        }

        if seed_all_file_ids.contains(&file_id) {
            // Files in seed-all directories (test dirs, fixtures, etc.):
            // ALL symbols are considered alive regardless of visibility.
            for &sym_id in symbol_ids {
                entry_symbols.push(sym_id);
            }
        } else if entry_file_ids.contains(&file_id) {
            // All non-private symbols in entry point files are entry points.
            // Additionally: main() at file scope is always seeded (private in Rust).
            for &sym_id in symbol_ids {
                if let Some(symbol) = symbol_graph.symbols.get(&sym_id) {
                    if symbol.visibility != Visibility::Private {
                        entry_symbols.push(sym_id);
                    } else if symbol.name == "main"
                        && symbol.parent.is_none()
                        && matches!(symbol.kind, SymbolKind::Function)
                    {
                        entry_symbols.push(sym_id);
                    }
                }
            }
        } else {
            // Only exports that are actually imported by other files (linker targets)
            // are seeded as entry points. Exports nobody imports are dead.
            if let Some(exports) = symbol_graph.exports.get(&file_id) {
                for export in exports {
                    if linker_targets.contains(&export.symbol) {
                        entry_symbols.push(export.symbol);
                    }
                }
            }
        }
    }

    let cross_file_edges = linker_targets.len();


    // BFS from entry points through intra-file references to find all reachable symbols
    let mut reachable = symbol_graph.reachable_from(&entry_symbols);

    // Post-BFS convergence loop: repeatedly apply propagation rules until
    // no new symbols are reached. This handles chains like:
    // ParserRegistry alive → with_defaults() seeded → BFS reaches RustParser::new()
    // → RustParser alive → languages() seeded → etc.
    loop {
        let prev_size = reachable.len();

        // 1. Seed all children of alive parents.
        // When any symbol is alive, its children (methods, variants, associated types,
        // trait impl methods, etc.) should be alive too. This handles enum variants,
        // trait methods, struct methods from impl blocks, and Display/Default/etc impls.
        let mut new_seeds: Vec<SymbolId> = Vec::new();
        for sym in symbol_graph.symbols.values() {
            if let Some(parent_id) = sym.parent {
                if reachable.contains(&parent_id) && !reachable.contains(&sym.id) {
                    reachable.insert(sym.id);
                    new_seeds.push(sym.id);
                }
            }
        }
        // Follow outgoing references from newly seeded children
        if !new_seeds.is_empty() {
            let extra = symbol_graph.reachable_from(&new_seeds);
            reachable.extend(extra);
        }

        // 2. Inheritance propagation for trait/interface dispatch.
        // When a trait/interface is alive, find all structs/classes that implement it
        // (via RefKind::Inheritance references) and seed them plus their children.
        // This handles dynamic dispatch patterns (e.g., &dyn LanguageParser).
        let alive_trait_names: HashSet<&str> = reachable
            .iter()
            .filter_map(|id| {
                symbol_graph.symbols.get(id).and_then(|s| {
                    if s.kind == SymbolKind::Interface {
                        Some(s.name.as_str())
                    } else {
                        None
                    }
                })
            })
            .collect();

        if !alive_trait_names.is_empty() {
            let mut inherit_seeds: Vec<SymbolId> = Vec::new();
            for reference in &symbol_graph.references {
                if reference.kind == RefKind::Inheritance {
                    if let Some(target_name) = &reference.target_name {
                        if alive_trait_names.contains(target_name.as_str()) {
                            if !reachable.contains(&reference.source) {
                                inherit_seeds.push(reference.source);
                            }
                            for sym in symbol_graph.symbols.values() {
                                if sym.parent == Some(reference.source)
                                    && !reachable.contains(&sym.id)
                                {
                                    inherit_seeds.push(sym.id);
                                }
                            }
                        }
                    }
                }
            }

            if !inherit_seeds.is_empty() {
                for &id in &inherit_seeds {
                    reachable.insert(id);
                }
                let extra_reachable = symbol_graph.reachable_from(&inherit_seeds);
                reachable.extend(extra_reachable);
            }
        }

        // 3. Reverse parent propagation.
        // If any child symbol is alive, its parent type symbol should be alive too.
        for sym in symbol_graph.symbols.values() {
            if let Some(parent_id) = sym.parent {
                if reachable.contains(&sym.id) && !reachable.contains(&parent_id) {
                    reachable.insert(parent_id);
                }
            }
        }

        if reachable.len() == prev_size {
            break;
        }
    }

    // Count references
    let intra_resolved = symbol_graph
        .references
        .iter()
        .filter(|r| r.target.0 < u64::MAX - 1_000_000)
        .count();

    // Find dead symbols: not reachable from any entry point
    // Exclude synthetic kinds (Import, Export, Package) that are not user-defined code.
    // Module symbols (`pub mod foo;`) are structural declarations — dead modules
    // are caught at the file level instead.
    let skip_kinds = [
        SymbolKind::Import,
        SymbolKind::Export,
        SymbolKind::Package,
        SymbolKind::Module,
    ];

    // Determine confidence based on linker quality.
    // If all cross-file imports were resolved (unresolved == 0), we have High confidence.
    // If some imports couldn't be resolved, we have Medium confidence.
    let linker_unresolved = linker_result.unresolved;
    let overall_confidence = if linker_unresolved == 0 {
        Confidence::High
    } else {
        Confidence::Medium
    };

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
                confidence: overall_confidence,
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

    let mut limitations = Vec::new();
    if linker_unresolved > 0 {
        limitations.push(Limitation {
            description: format!(
                "{} cross-file imports could not be resolved to symbols",
                linker_unresolved
            ),
            count: linker_unresolved,
        });
    }

    DeadSymbolResult {
        summary: DeadSymbolSummary {
            total_symbols,
            dead_symbols: dead_symbols.len(),
            entry_point_symbols: entry_symbols.len(),
            resolved_references: intra_resolved + cross_file_edges,
            unresolved_references: linker_unresolved,
        },
        dead_symbols,
        confidence: overall_confidence,
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
    use crate::analysis::linker::LinkingResult;
    use crate::model::file_graph::{FileImport, FileInfo, UnresolvedImport, UnresolvedReason};
    use crate::model::{
        FileRecord, Language, LineSpan, Position, RefKind, Reference, ReferenceId, Span, Symbol,
    };

    fn empty_linker() -> LinkingResult {
        LinkingResult {
            resolved: 0,
            unresolved: 0,
            references: vec![],
        }
    }

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
                    line: i + 1,
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
            is_mod_declaration: false,
            line: 1,
        }
    }

    #[test]
    fn test_no_dead_files_in_connected_project() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/utils.ts", false));
        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
        assert_eq!(result.dead_files.len(), 1);
        assert_eq!(result.dead_files[0].path, PathBuf::from("src/orphan.ts"));
        assert_eq!(result.dead_files[0].confidence, Confidence::Certain);
    }

    #[test]
    fn test_entry_points_never_reported_dead() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        // Entry point with no imports is NOT dead
        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
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
        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Both, &HashSet::new());
        assert_eq!(result.dead_files.len(), 1); // orphan.ts
        assert!(result.dead_exports.is_empty()); // helper is used
    }

    #[test]
    fn test_empty_graph() {
        let graph = FileGraph::new();
        let result = detect_dead_code(&graph, DeadCodeScope::Both, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Both, &HashSet::new());
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        // Entry point imports barrel for some other reason but NOT "helper"
        graph.add_import(make_edge(1, 2, &["somethingElse"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
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

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        // Utils file: export function helper()
        graph.add_file(make_file_with_exports(
            3,
            "src/utils.ts",
            false,
            &["helper", "unused_fn"],
        ));

        // Entry imports "helper" from barrel
        graph.add_import(make_edge(1, 2, &["helper"]));
        // Barrel re-export creates edge to utils
        graph.add_import(make_edge(2, 3, &["*"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);

        graph.add_file(make_file_with_exports(
            3,
            "src/utils.ts",
            false,
            &["helper", "unused_fn"],
        ));

        graph.add_import(make_edge(1, 2, &["helper"]));
        graph.add_import(make_edge(2, 3, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
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
                line: 0,
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(file_b);

        // C: export function foo
        graph.add_file(make_file_with_exports(4, "src/c.ts", false, &["foo"]));

        graph.add_import(make_edge(1, 2, &["foo"]));
        graph.add_import(make_edge(2, 3, &["*"]));
        graph.add_import(make_edge(3, 4, &["*"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
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
                line: 0,
            }],
            is_entry_point: false,
        };
        graph.add_file(barrel);
        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Both, &HashSet::new());
        // With wildcards and no unresolved, confidence should be High (not Certain)
        assert_eq!(result.confidence, Confidence::High);
    }

    #[test]
    fn test_dead_file_with_own_unresolved_outgoing_imports_still_high_confidence() {
        // A dead file that has unresolved outgoing imports (e.g. `import java.util.HashMap`)
        // should NOT have reduced confidence. Its own outgoing edges are irrelevant
        // to whether it's reachable — only incoming edges matter.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/used.ts", false));
        graph.add_file(make_file(3, "src/orphan.ts", false)); // dead

        graph.add_import(make_edge(1, 2, &["helper"]));

        // The orphan file has an external outgoing import (e.g. third-party lib)
        graph.add_unresolved(UnresolvedImport {
            file: FileId(3),
            import_path: "java.util.HashMap".to_string(),
            reason: UnresolvedReason::External("java.util".to_string()),
            line: 1,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
        assert_eq!(result.dead_files.len(), 1);
        assert_eq!(result.dead_files[0].path, PathBuf::from("src/orphan.ts"));
        // External imports are correctly classified third-party libs, not broken imports.
        // They should not reduce confidence at all.
        assert_eq!(
            result.dead_files[0].confidence,
            Confidence::Certain,
            "External imports should not reduce confidence"
        );
    }

    #[test]
    fn test_dead_file_confidence_high_when_other_files_have_unresolved() {
        // If some other file has unresolved imports, a dead file should be High
        // confidence (not Certain), because the unresolved import might point here.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/used.ts", false));
        graph.add_file(make_file(3, "src/orphan.ts", false)); // dead

        graph.add_import(make_edge(1, 2, &["helper"]));

        // A different file (the used one) has an unresolved import
        graph.add_unresolved(UnresolvedImport {
            file: FileId(2),
            import_path: "some-missing-module".to_string(),
            reason: UnresolvedReason::FileNotFound("some-missing-module".to_string()),
            line: 5,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
        assert_eq!(result.dead_files.len(), 1);
        assert_eq!(
            result.dead_files[0].confidence,
            Confidence::High,
            "Should be High when other files have unresolved imports"
        );
    }

    #[test]
    fn test_dead_file_confidence_certain_when_no_unresolved() {
        // With no unresolved imports anywhere, dead files should be Certain.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "src/used.ts", false));
        graph.add_file(make_file(3, "src/orphan.ts", false)); // dead

        graph.add_import(make_edge(1, 2, &["helper"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Files, &HashSet::new());
        assert_eq!(result.dead_files.len(), 1);
        assert_eq!(
            result.dead_files[0].confidence,
            Confidence::Certain,
            "Should be Certain when no unresolved imports exist"
        );
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
                        end: Position {
                            line: 1,
                            column: 10,
                        },
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
                        end: Position {
                            line: 2,
                            column: 10,
                        },
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
                        end: Position {
                            line: 3,
                            column: 10,
                        },
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
                    end: Position {
                        line: 1,
                        column: 15,
                    },
                },
                target_name: None,
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

        let dead_result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());

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
        assert!(!dead_names.contains(&"main"), "main should NOT be dead");
        assert!(
            !dead_names.contains(&"helper"),
            "helper should NOT be dead (called by main)"
        );
    }

    fn make_sym(id: u64, name: &str, kind: SymbolKind, file: u64, vis: Visibility) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
            file: FileId(file),
            span: Span { start: 0, end: 10 },
            line_span: LineSpan {
                start: Position {
                    line: id as usize,
                    column: 0,
                },
                end: Position {
                    line: id as usize,
                    column: 10,
                },
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
            target_name: None,
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

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        assert!(result.dead_symbols.is_empty());
        assert_eq!(result.summary.total_symbols, 0);
        assert_eq!(result.summary.entry_point_symbols, 0);
    }

    #[test]
    fn test_dead_symbols_multi_file_private_unreachable() {
        use crate::analysis::linker::{CrossFileRef, LinkingResult};
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
            symbols: vec![make_sym(
                1,
                "main",
                SymbolKind::Function,
                1,
                Visibility::Public,
            )],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        // File 2: exported symbol becomes entry point when linker confirms it's imported
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_sym(
                    10,
                    "api_handler",
                    SymbolKind::Function,
                    2,
                    Visibility::Public,
                ),
                make_sym(
                    11,
                    "internal_helper",
                    SymbolKind::Function,
                    2,
                    Visibility::Private,
                ),
                make_sym(
                    12,
                    "dead_internal",
                    SymbolKind::Function,
                    2,
                    Visibility::Private,
                ),
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
                line: 1,
            }],
            type_references: vec![],
            annotations: vec![],
        });

        // Linker confirms that file 1 imports api_handler from file 2
        let linker = LinkingResult {
            resolved: 1,
            unresolved: 0,
            references: vec![CrossFileRef {
                source_file: FileId(1),
                target_file: FileId(2),
                target_symbol: SymbolId(10),
                imported_name: "api_handler".to_string(),
                line: 1,
                confidence: crate::analysis::Confidence::High,
            }],
        };

        let result = detect_dead_symbols(&sym_graph, &file_graph, &linker, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();

        assert!(
            dead_names.contains(&"dead_internal"),
            "dead_internal should be dead (private, never called), dead: {:?}",
            dead_names
        );
        assert!(
            !dead_names.contains(&"api_handler"),
            "api_handler should NOT be dead (linker target)"
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

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();

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

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();

        assert_eq!(
            dead_names,
            vec!["d"],
            "only d should be dead, got: {:?}",
            dead_names
        );
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
                make_sym(
                    2,
                    "maybe_dead",
                    SymbolKind::Function,
                    1,
                    Visibility::Private,
                ),
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
                    target_name: None,
                },
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());

        // With an empty linker (no cross-file imports), confidence is High
        // because there's nothing unresolved at the cross-file level.
        // Placeholder-target refs are intra-file parser artifacts, not import failures.
        assert_eq!(result.confidence, Confidence::High);
        assert_eq!(result.summary.unresolved_references, 0);

        // With unresolved linker imports, confidence reflects that
        let linker_with_unresolved = LinkingResult {
            resolved: 1,
            unresolved: 3,
            references: vec![],
        };
        let result2 = detect_dead_symbols(&sym_graph, &file_graph, &linker_with_unresolved, &HashSet::new());
        assert_eq!(result2.confidence, Confidence::Medium);
        assert_eq!(result2.summary.unresolved_references, 3);
        assert!(!result2.limitations.is_empty());
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
                make_ref(1, 1, 2, RefKind::Call, 1),        // main -> UserClass
                make_ref(2, 2, 3, RefKind::Inheritance, 1), // UserClass extends BaseClass
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();

        assert!(
            !dead_names.contains(&"BaseClass"),
            "BaseClass reached via inheritance"
        );
        assert!(
            dead_names.contains(&"UnusedClass"),
            "UnusedClass should be dead"
        );
    }

    #[test]
    fn test_trait_methods_alive_when_trait_alive() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Trait with two methods: one default impl, one signature-only
        let mut method1 = make_sym(2, "default_method", SymbolKind::Method, 1, Visibility::Public);
        method1.parent = Some(SymbolId(1));
        let mut method2 = make_sym(3, "required_method", SymbolKind::Method, 1, Visibility::Public);
        method2.parent = Some(SymbolId(1));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "MyTrait", SymbolKind::Interface, 1, Visibility::Public),
                method1,
                method2,
                make_sym(4, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        // Trait is alive (public in entry file), so both methods should be alive
        assert!(!dead_names.contains(&"MyTrait"), "MyTrait should be alive");
        assert!(!dead_names.contains(&"default_method"), "default_method should be alive when trait is alive");
        assert!(!dead_names.contains(&"required_method"), "required_method should be alive when trait is alive");
    }

    #[test]
    fn test_trait_methods_dead_when_trait_dead() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        // Put the dead trait in a non-entry file so public visibility doesn't auto-seed it
        file_graph.add_file(make_file(1, "src/index.ts", true));
        file_graph.add_file(make_file(2, "src/utils.ts", false));
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

        // Unreachable trait in non-entry file — no linker reference targets it
        let mut method = make_sym(12, "trait_method", SymbolKind::Method, 2, Visibility::Public);
        method.parent = Some(SymbolId(11));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_sym(11, "DeadTrait", SymbolKind::Interface, 2, Visibility::Public),
                method,
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(dead_names.contains(&"DeadTrait"), "DeadTrait should be dead");
        assert!(dead_names.contains(&"trait_method"), "trait_method should be dead when trait is dead");
    }

    #[test]
    fn test_parent_alive_when_child_alive() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Struct with a method that is alive (via call from main).
        // The struct itself has no direct incoming edge.
        let mut method = make_sym(2, "do_thing", SymbolKind::Method, 1, Visibility::Private);
        method.parent = Some(SymbolId(1));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "MyStruct", SymbolKind::Struct, 1, Visibility::Private),
                method,
                make_sym(3, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![
                make_ref(1, 3, 2, RefKind::Call, 1), // main -> do_thing
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(!dead_names.contains(&"do_thing"), "do_thing should be alive (called by main)");
        assert!(!dead_names.contains(&"MyStruct"), "MyStruct should be alive (child is alive)");
    }

    #[test]
    fn test_grandparent_propagation() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Module -> Struct -> Method, method is alive
        let mut my_struct = make_sym(2, "MyStruct", SymbolKind::Struct, 1, Visibility::Private);
        my_struct.parent = Some(SymbolId(1));
        let mut method = make_sym(3, "do_thing", SymbolKind::Method, 1, Visibility::Private);
        method.parent = Some(SymbolId(2));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "my_mod", SymbolKind::Module, 1, Visibility::Private),
                my_struct,
                method,
                make_sym(4, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![
                make_ref(1, 4, 3, RefKind::Call, 1), // main -> do_thing
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(!dead_names.contains(&"do_thing"), "do_thing should be alive");
        assert!(!dead_names.contains(&"MyStruct"), "MyStruct should be alive (child is alive)");
        assert!(!dead_names.contains(&"my_mod"), "my_mod should be alive (grandchild is alive)");
    }

    #[test]
    fn test_parent_dead_when_no_alive_children() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        file_graph.add_file(make_file(2, "src/utils.ts", false));
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

        // Unreachable struct with dead children in non-entry file
        let mut method = make_sym(12, "dead_method", SymbolKind::Method, 2, Visibility::Public);
        method.parent = Some(SymbolId(11));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_sym(11, "UnusedStruct", SymbolKind::Struct, 2, Visibility::Public),
                method,
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(dead_names.contains(&"UnusedStruct"), "UnusedStruct should be dead");
        assert!(dead_names.contains(&"dead_method"), "dead_method should be dead");
    }

    #[test]
    fn test_module_symbols_excluded_from_dead() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "my_mod", SymbolKind::Module, 1, Visibility::Private),
                make_sym(2, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        // Module symbols should never appear in dead_symbols results
        assert!(!dead_names.contains(&"my_mod"), "Module symbols should be excluded from dead reporting");
    }

    #[test]
    fn test_mod_reexport_alive_when_target_reachable() {
        // `pub mod foo;` export should NOT be dead when foo.rs is reachable
        use crate::model::file_graph::FileImport;

        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/main.rs", true));
        graph.add_file(make_file_with_exports(2, "src/lib.rs", false, &["foo"]));
        graph.add_file(make_file(3, "src/foo.rs", false));

        // main -> lib (imports "foo")
        graph.add_import(make_edge(1, 2, &["foo"]));
        // lib has `pub mod foo;` which creates a mod declaration import to foo.rs
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["foo".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_export_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();

        assert!(
            !dead_export_names.contains(&"foo"),
            "mod re-export 'foo' should NOT be dead when foo.rs is reachable"
        );
    }

    #[test]
    fn test_mod_reexport_dead_when_target_unreachable() {
        // `pub mod foo;` export IS dead when the declaring file is itself unreachable
        use crate::model::file_graph::FileImport;

        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/main.rs", true));
        // lib.rs is NOT imported by anyone — it's an orphan file with a mod declaration
        graph.add_file(make_file_with_exports(2, "src/lib.rs", false, &["foo"]));
        graph.add_file(make_file(3, "src/foo.rs", false));

        // lib has `pub mod foo;` but lib itself is unreachable, so foo.rs is too
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["foo".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_export_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();

        assert!(
            dead_export_names.contains(&"foo"),
            "mod re-export 'foo' SHOULD be dead when lib.rs is unreachable, got: {:?}",
            dead_export_names
        );
    }

    #[test]
    fn test_seed_all_files_not_reported_dead() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "tests/fixture.ts", false)); // orphan normally

        let seed_all: HashSet<FileId> = [FileId(2)].into_iter().collect();
        let result = detect_dead_code(&graph, DeadCodeScope::Files, &seed_all);
        assert!(
            result.dead_files.is_empty(),
            "seed_all files should not be reported dead, got: {:?}",
            result.dead_files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_seed_all_exports_not_reported_dead() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file_with_exports(
            2,
            "tests/fixture.ts",
            false,
            &["helper"],
        ));

        let seed_all: HashSet<FileId> = [FileId(2)].into_iter().collect();
        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &seed_all);
        let dead_export_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
        assert!(
            !dead_export_names.contains(&"helper"),
            "exports from seed_all files should not be reported dead"
        );
    }

    #[test]
    fn test_seed_all_imports_make_targets_reachable() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/index.ts", true));
        graph.add_file(make_file(2, "tests/fixture.ts", false));
        graph.add_file(make_file(3, "src/utils.ts", false)); // only reachable from fixture

        graph.add_import(make_edge(2, 3, &["util"]));

        let seed_all: HashSet<FileId> = [FileId(2)].into_iter().collect();
        let result = detect_dead_code(&graph, DeadCodeScope::Files, &seed_all);
        assert!(
            result.dead_files.is_empty(),
            "files imported by seed_all files should be reachable, got dead: {:?}",
            result.dead_files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_constant_alive_when_referenced() {
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // main references MY_CONST via a FieldAccess reference (identifier reference)
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                make_sym(2, "MY_CONST", SymbolKind::Constant, 1, Visibility::Private),
            ],
            references: vec![
                make_ref(1, 1, 2, RefKind::FieldAccess, 1), // main -> MY_CONST
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(!dead_names.contains(&"MY_CONST"), "MY_CONST should be alive (referenced by main)");
    }

    #[test]
    fn test_constant_dead_when_unreferenced() {
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
                make_sym(2, "UNUSED_CONST", SymbolKind::Constant, 1, Visibility::Private),
            ],
            references: vec![], // no references to UNUSED_CONST
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(dead_names.contains(&"UNUSED_CONST"), "UNUSED_CONST should be dead (no references)");
    }

    #[test]
    fn test_rust_module_path_import_marks_exports_used() {
        // In Rust, `use crate::cli::commands` imports the module name "commands"
        // from the file commands.rs. When code does `commands::run_deps()`,
        // the imported_name is "commands" but exports are "run_deps" etc.
        // The module-name import should mark all exports as used.
        use crate::model::{ExportRecord, SymbolId};

        let mut graph = FileGraph::new();

        // main.rs is the entry point
        graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/main.rs"),
            language: Language::Rust,
            exports: vec![],
            is_entry_point: true,
        });

        // commands.rs exports run_deps, build_file_graph
        graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/cli/commands.rs"),
            language: Language::Rust,
            exports: vec![
                ExportRecord {
                    file: FileId(2),
                    symbol: SymbolId(200),
                    exported_name: "run_deps".to_string(),
                    is_default: false,
                    is_reexport: false,
                    is_type_only: false,
                    source_path: None,
                    line: 10,
                },
                ExportRecord {
                    file: FileId(2),
                    symbol: SymbolId(201),
                    exported_name: "build_file_graph".to_string(),
                    is_default: false,
                    is_reexport: false,
                    is_type_only: false,
                    source_path: None,
                    line: 20,
                },
            ],
            is_entry_point: false,
        });

        // main.rs imports "commands" (module name) from commands.rs
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["commands".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();

        assert!(
            !dead_names.contains(&"run_deps"),
            "run_deps should NOT be dead (module-path import), dead: {:?}",
            dead_names
        );
        assert!(
            !dead_names.contains(&"build_file_graph"),
            "build_file_graph should NOT be dead (module-path import), dead: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_rust_module_path_import_does_not_affect_typescript() {
        // Module-path logic should only apply to Rust files
        use crate::model::{ExportRecord, SymbolId};

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
            path: PathBuf::from("src/utils.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(200),
                exported_name: "helper".to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 1,
            }],
            is_entry_point: false,
        });

        // Importing "utils" (file stem) should NOT mark "helper" as used in TS
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["utils".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
        assert!(
            dead_names.contains(&"helper"),
            "helper should be dead in TS (module-path import logic is Rust-only), dead: {:?}",
            dead_names
        );
    }

    // ---- Cross-file linking tests ----

    /// Helper: build a SymbolGraph + FileGraph + LinkingResult from a test scenario,
    /// then run detect_dead_symbols and return the dead/alive symbol names.
    fn run_cross_file_scenario(
        file_graph_setup: impl FnOnce(&mut FileGraph),
        symbol_graph_setup: impl FnOnce(&mut crate::model::graph::SymbolGraph),
        linker_refs: Vec<crate::analysis::linker::CrossFileRef>,
    ) -> (Vec<String>, Vec<String>) {
        use crate::analysis::linker::LinkingResult;
        use crate::model::graph::SymbolGraph;

        let mut file_graph = FileGraph::new();
        file_graph_setup(&mut file_graph);

        let mut sym_graph = SymbolGraph::new();
        symbol_graph_setup(&mut sym_graph);

        let linker_result = LinkingResult {
            resolved: linker_refs.len(),
            unresolved: 0,
            references: linker_refs,
        };

        let result = detect_dead_symbols(&sym_graph, &file_graph, &linker_result, &HashSet::new());

        let skip_kinds = [SymbolKind::Import, SymbolKind::Export, SymbolKind::Package];
        let dead_names: Vec<String> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let alive_names: Vec<String> = sym_graph
            .symbols
            .values()
            .filter(|s| !skip_kinds.contains(&s.kind) && !dead_names.contains(&s.name))
            .map(|s| s.name.clone())
            .collect();

        (alive_names, dead_names)
    }

    #[test]
    fn test_cross_file_multi_hop_chain() {
        // A (entry) -> B -> C: all alive via cross-file linker targets.
        // Linker resolves: A imports process from B, B imports compute from C.
        // Both process and compute are linker targets so they get seeded as entry points.
        use crate::analysis::linker::CrossFileRef;
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                fg.add_file(make_file(3, "src/c.ts", false));
                fg.add_import(make_edge(1, 2, &["process"]));
                fg.add_import(make_edge(2, 3, &["compute"]));
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                sg.add_file(make_file_record(3, "src/c.ts"));
                // File A: main (public, entry)
                sg.add_parse_result(ParseResult {
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
                // File B: process (exported) calls internal_b
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "process", SymbolKind::Function, 2, Visibility::Public),
                        make_sym(11, "internal_b", SymbolKind::Function, 2, Visibility::Private),
                    ],
                    references: vec![make_ref(1, 10, 11, RefKind::Call, 2)],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2),
                        symbol: SymbolId(10),
                        exported_name: "process".to_string(),
                        is_default: false,
                        is_reexport: false,
                        is_type_only: false,
                        source_path: None,
                        line: 1,
                    }],
                    type_references: vec![],
                    annotations: vec![],
                });
                // File C: compute (exported)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(3),
                    symbols: vec![
                        make_sym(20, "compute", SymbolKind::Function, 3, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(3),
                        symbol: SymbolId(20),
                        exported_name: "compute".to_string(),
                        is_default: false,
                        is_reexport: false,
                        is_type_only: false,
                        source_path: None,
                        line: 1,
                    }],
                    type_references: vec![],
                    annotations: vec![],
                });
            },
            vec![
                // Linker: A imports process from B
                CrossFileRef {
                    source_file: FileId(1),
                    target_file: FileId(2),
                    target_symbol: SymbolId(10),
                    imported_name: "process".to_string(),
                    line: 1,
                    confidence: crate::analysis::Confidence::High,
                },
                // Linker: B imports compute from C
                CrossFileRef {
                    source_file: FileId(2),
                    target_file: FileId(3),
                    target_symbol: SymbolId(20),
                    imported_name: "compute".to_string(),
                    line: 1,
                    confidence: crate::analysis::Confidence::High,
                },
            ],
        );

        assert!(alive.contains(&"main".to_string()), "main should be alive");
        assert!(alive.contains(&"process".to_string()), "process should be alive (linker target)");
        assert!(alive.contains(&"internal_b".to_string()), "internal_b should be alive (called by process)");
        assert!(alive.contains(&"compute".to_string()), "compute should be alive (linker target)");
        assert!(dead.is_empty(), "no non-synthetic symbols should be dead");
    }

    #[test]
    fn test_cross_file_diamond_dependency() {
        // A (entry) -> B and C, both -> D: all alive
        use crate::analysis::linker::CrossFileRef;
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                fg.add_file(make_file(3, "src/c.ts", false));
                fg.add_file(make_file(4, "src/d.ts", false));
                fg.add_import(make_edge(1, 2, &["b_fn"]));
                fg.add_import(make_edge(1, 3, &["c_fn"]));
                fg.add_import(make_edge(2, 4, &["d_fn"]));
                fg.add_import(make_edge(3, 4, &["d_fn"]));
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                sg.add_file(make_file_record(3, "src/c.ts"));
                sg.add_file(make_file_record(4, "src/d.ts"));
                // A: main only
                sg.add_parse_result(ParseResult {
                    file_id: FileId(1),
                    symbols: vec![
                        make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
                });
                // B: b_fn (exported)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "b_fn", SymbolKind::Function, 2, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2), symbol: SymbolId(10),
                        exported_name: "b_fn".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
                // C: c_fn (exported)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(3),
                    symbols: vec![
                        make_sym(20, "c_fn", SymbolKind::Function, 3, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(3), symbol: SymbolId(20),
                        exported_name: "c_fn".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
                // D: d_fn (the diamond bottom)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(4),
                    symbols: vec![
                        make_sym(30, "d_fn", SymbolKind::Function, 4, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(4), symbol: SymbolId(30),
                        exported_name: "d_fn".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
            },
            vec![
                CrossFileRef {
                    source_file: FileId(1), target_file: FileId(2),
                    target_symbol: SymbolId(10), imported_name: "b_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
                CrossFileRef {
                    source_file: FileId(1), target_file: FileId(3),
                    target_symbol: SymbolId(20), imported_name: "c_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
                CrossFileRef {
                    source_file: FileId(2), target_file: FileId(4),
                    target_symbol: SymbolId(30), imported_name: "d_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
                CrossFileRef {
                    source_file: FileId(3), target_file: FileId(4),
                    target_symbol: SymbolId(30), imported_name: "d_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
            ],
        );

        assert!(alive.contains(&"main".to_string()), "main alive");
        assert!(alive.contains(&"b_fn".to_string()), "b_fn alive (A -> B)");
        assert!(alive.contains(&"c_fn".to_string()), "c_fn alive (A -> C)");
        assert!(alive.contains(&"d_fn".to_string()), "d_fn alive (diamond bottom)");
        assert!(dead.is_empty(), "no non-synthetic symbols should be dead");
    }

    #[test]
    fn test_cross_file_unexported_export_is_dead() {
        // B exports process, process calls helper. B is never imported by any entry point.
        // With precise seeding, only linker-resolved exports are entry points.
        // Since nobody imports B, process is NOT seeded, so process, helper, and dead_fn are all dead.
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                // No import edge from A to B
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                // A: main only
                sg.add_parse_result(ParseResult {
                    file_id: FileId(1),
                    symbols: vec![
                        make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
                });
                // B: process (exported) calls helper (private), dead_fn (private, never called)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "process", SymbolKind::Function, 2, Visibility::Public),
                        make_sym(11, "helper", SymbolKind::Function, 2, Visibility::Private),
                        make_sym(12, "dead_fn", SymbolKind::Function, 2, Visibility::Private),
                    ],
                    references: vec![make_ref(1, 10, 11, RefKind::Call, 2)],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2), symbol: SymbolId(10),
                        exported_name: "process".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
            },
            vec![], // No linker refs -- B is never imported
        );

        // Precise seeding: nobody imports process, so it's not an entry point.
        // All symbols in B are unreachable.
        assert!(dead.contains(&"process".to_string()), "process dead (exported but never imported)");
        assert!(dead.contains(&"helper".to_string()), "helper dead (caller process is also dead)");
        assert!(dead.contains(&"dead_fn".to_string()), "dead_fn dead (private, never called)");
        assert!(alive.contains(&"main".to_string()), "main alive (entry point)");
    }

    #[test]
    fn test_cross_file_imported_export_stays_alive() {
        // B exports process, A imports it via linker. process and its helper are alive.
        // dead_fn is still dead because nobody calls it.
        use crate::analysis::linker::CrossFileRef;
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                fg.add_import(make_edge(1, 2, &["process"]));
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                sg.add_parse_result(ParseResult {
                    file_id: FileId(1),
                    symbols: vec![
                        make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
                });
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "process", SymbolKind::Function, 2, Visibility::Public),
                        make_sym(11, "helper", SymbolKind::Function, 2, Visibility::Private),
                        make_sym(12, "dead_fn", SymbolKind::Function, 2, Visibility::Private),
                    ],
                    references: vec![make_ref(1, 10, 11, RefKind::Call, 2)],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2), symbol: SymbolId(10),
                        exported_name: "process".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
            },
            vec![CrossFileRef {
                source_file: FileId(1), target_file: FileId(2),
                target_symbol: SymbolId(10), imported_name: "process".to_string(),
                line: 1, confidence: crate::analysis::Confidence::High,
            }],
        );

        assert!(alive.contains(&"main".to_string()), "main alive");
        assert!(alive.contains(&"process".to_string()), "process alive (imported via linker)");
        assert!(alive.contains(&"helper".to_string()), "helper alive (called by process)");
        assert!(dead.contains(&"dead_fn".to_string()), "dead_fn dead (never called)");
    }

    #[test]
    fn test_cross_file_mixed_alive_dead_same_file() {
        // File B has: exported_fn (used by A via linker), helper (called by exported_fn), dead_fn (never called)
        use crate::analysis::linker::CrossFileRef;
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                fg.add_import(make_edge(1, 2, &["exported_fn"]));
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                // A: main only
                sg.add_parse_result(ParseResult {
                    file_id: FileId(1),
                    symbols: vec![
                        make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
                });
                // B: exported_fn calls helper, dead_fn is isolated
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "exported_fn", SymbolKind::Function, 2, Visibility::Public),
                        make_sym(11, "helper", SymbolKind::Function, 2, Visibility::Private),
                        make_sym(12, "dead_fn", SymbolKind::Function, 2, Visibility::Private),
                    ],
                    references: vec![make_ref(2, 10, 11, RefKind::Call, 2)],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2), symbol: SymbolId(10),
                        exported_name: "exported_fn".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
            },
            vec![
                // Linker: A imports exported_fn from B
                CrossFileRef {
                    source_file: FileId(1), target_file: FileId(2),
                    target_symbol: SymbolId(10), imported_name: "exported_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
            ],
        );

        assert!(alive.contains(&"main".to_string()), "main alive");
        assert!(alive.contains(&"exported_fn".to_string()), "exported_fn alive (linker target)");
        assert!(alive.contains(&"helper".to_string()), "helper alive (called by exported_fn)");
        assert!(dead.contains(&"dead_fn".to_string()), "dead_fn should be dead (never called)");
    }

    #[test]
    fn test_cross_file_circular_references() {
        // A (entry) -> B, B -> A (circular): both alive
        use crate::analysis::linker::CrossFileRef;
        use crate::model::*;

        let (alive, dead) = run_cross_file_scenario(
            |fg| {
                fg.add_file(make_file(1, "src/a.ts", true));
                fg.add_file(make_file(2, "src/b.ts", false));
                fg.add_import(make_edge(1, 2, &["b_fn"]));
                fg.add_import(make_edge(2, 1, &["a_fn"]));
            },
            |sg| {
                sg.add_file(make_file_record(1, "src/a.ts"));
                sg.add_file(make_file_record(2, "src/b.ts"));
                // A: a_fn (public, entry)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(1),
                    symbols: vec![
                        make_sym(1, "a_fn", SymbolKind::Function, 1, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
                });
                // B: b_fn (exported)
                sg.add_parse_result(ParseResult {
                    file_id: FileId(2),
                    symbols: vec![
                        make_sym(10, "b_fn", SymbolKind::Function, 2, Visibility::Public),
                    ],
                    references: vec![],
                    imports: vec![],
                    exports: vec![ExportRecord {
                        file: FileId(2), symbol: SymbolId(10),
                        exported_name: "b_fn".to_string(),
                        is_default: false, is_reexport: false, is_type_only: false,
                        source_path: None, line: 1,
                    }],
                    type_references: vec![], annotations: vec![],
                });
            },
            vec![
                CrossFileRef {
                    source_file: FileId(1), target_file: FileId(2),
                    target_symbol: SymbolId(10), imported_name: "b_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
                CrossFileRef {
                    source_file: FileId(2), target_file: FileId(1),
                    target_symbol: SymbolId(1), imported_name: "a_fn".to_string(),
                    line: 1, confidence: crate::analysis::Confidence::High,
                },
            ],
        );

        assert!(alive.contains(&"a_fn".to_string()), "a_fn alive (entry + circular)");
        assert!(alive.contains(&"b_fn".to_string()), "b_fn alive (linker target from entry)");
        assert!(dead.is_empty(), "no non-synthetic symbols should be dead");
    }

    #[test]
    fn test_cross_file_confidence_reflects_linker_quality() {
        use crate::analysis::linker::LinkingResult;
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/a.ts", true));
        sym_graph.add_file(make_file_record(1, "src/a.ts"));
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![], exports: vec![], type_references: vec![], annotations: vec![],
        });

        // All resolved, no unresolved -> High
        let good_linker = LinkingResult {
            resolved: 10,
            unresolved: 0,
            references: vec![],
        };
        let result = detect_dead_symbols(&sym_graph, &file_graph, &good_linker, &HashSet::new());
        assert_eq!(result.confidence, Confidence::High);
        assert!(result.limitations.is_empty());

        // Some unresolved -> Medium
        let bad_linker = LinkingResult {
            resolved: 5,
            unresolved: 3,
            references: vec![],
        };
        let result = detect_dead_symbols(&sym_graph, &file_graph, &bad_linker, &HashSet::new());
        assert_eq!(result.confidence, Confidence::Medium);
        assert!(!result.limitations.is_empty());
        assert_eq!(result.summary.unresolved_references, 3);
    }

    #[test]
    fn test_end_to_end_real_linker_with_dead_symbols() {
        // This test uses the REAL linker (link_cross_file_symbols) instead of
        // hand-constructing a LinkingResult. It verifies the full chain:
        //
        // Scenario:
        //   File A (entry point): has function `main()` which calls `do_stuff()` (imported from B)
        //   File B (non-entry):   exports `do_stuff()` which calls private `helper()` in the same file
        //                         also has `dead_fn()` which is never called
        //
        // Expected: main, do_stuff, helper all alive. dead_fn is dead.
        //
        // Chain that must work:
        //   1. FileGraph has export "do_stuff" with SymbolId(10) on File B
        //   2. FileGraph has import edge A->B with imported_name "do_stuff"
        //   3. Linker resolves import to CrossFileRef { target_symbol: SymbolId(10) }
        //   4. detect_dead_symbols sees SymbolId(10) in linker_targets
        //   5. SymbolGraph.exports[FileId(2)] has ExportRecord with symbol: SymbolId(10)
        //   6. SymbolId(10) is seeded as entry point
        //   7. BFS follows reference SymbolId(10) -> SymbolId(11) (do_stuff calls helper)
        //   8. helper is marked alive
        use crate::analysis::linker::link_cross_file_symbols;
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut file_graph = FileGraph::new();
        let mut sym_graph = SymbolGraph::new();

        // File A (entry point) - has main(), imports do_stuff from B
        file_graph.add_file(FileInfo {
            id: FileId(1),
            path: PathBuf::from("src/index.ts"),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: true,
        });

        // File B (non-entry) - exports do_stuff
        file_graph.add_file(FileInfo {
            id: FileId(2),
            path: PathBuf::from("src/service.ts"),
            language: Language::TypeScript,
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(10),
                exported_name: "do_stuff".to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 1,
            }],
            is_entry_point: false,
        });

        // Import edge: A imports "do_stuff" from B
        file_graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["do_stuff".to_string()],
            is_type_only: false,
            is_mod_declaration: false,
            line: 1,
        });

        // Now build SymbolGraph with matching symbols
        sym_graph.add_file(make_file_record(1, "src/index.ts"));
        sym_graph.add_file(make_file_record(2, "src/service.ts"));

        // File A: just main (public)
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![make_sym(
                1,
                "main",
                SymbolKind::Function,
                1,
                Visibility::Public,
            )],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        // File B: do_stuff (exported, public), helper (private, called by do_stuff),
        //         dead_fn (private, never called)
        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_sym(10, "do_stuff", SymbolKind::Function, 2, Visibility::Public),
                make_sym(11, "helper", SymbolKind::Function, 2, Visibility::Private),
                make_sym(12, "dead_fn", SymbolKind::Function, 2, Visibility::Private),
            ],
            references: vec![
                make_ref(1, 10, 11, RefKind::Call, 2), // do_stuff calls helper
            ],
            imports: vec![],
            exports: vec![ExportRecord {
                file: FileId(2),
                symbol: SymbolId(10),
                exported_name: "do_stuff".to_string(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 1,
            }],
            type_references: vec![],
            annotations: vec![],
        });

        // Step 1: Run the REAL linker on the file graph
        let linker_result = link_cross_file_symbols(&file_graph);

        // Verify linker resolved the import
        assert_eq!(
            linker_result.resolved, 1,
            "linker should resolve 1 import, got resolved={} unresolved={}",
            linker_result.resolved, linker_result.unresolved
        );
        assert_eq!(
            linker_result.references.len(),
            1,
            "linker should produce 1 CrossFileRef"
        );
        let xref = &linker_result.references[0];
        assert_eq!(
            xref.target_symbol,
            SymbolId(10),
            "linker should resolve to SymbolId(10) (do_stuff)"
        );
        assert_eq!(xref.source_file, FileId(1));
        assert_eq!(xref.target_file, FileId(2));

        // Step 2: Feed linker result into detect_dead_symbols
        let result = detect_dead_symbols(&sym_graph, &file_graph, &linker_result, &HashSet::new());

        let dead_names: Vec<&str> = result
            .dead_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        let alive_count = result.summary.total_symbols - result.summary.dead_symbols;

        // main should be alive (public in entry point)
        assert!(
            !dead_names.contains(&"main"),
            "main should be alive (entry point public symbol)"
        );
        // do_stuff should be alive (linker target, exported and imported)
        assert!(
            !dead_names.contains(&"do_stuff"),
            "do_stuff should be alive (linker resolved it as import target)"
        );
        // helper should be alive (called by do_stuff via intra-file reference)
        assert!(
            !dead_names.contains(&"helper"),
            "helper should be alive (called by do_stuff via intra-file BFS), dead: {:?}",
            dead_names
        );
        // dead_fn should be dead (never called by anything)
        assert!(
            dead_names.contains(&"dead_fn"),
            "dead_fn should be dead (never called), dead: {:?}",
            dead_names
        );
        // Summary: 4 total symbols, 1 dead, 3 alive
        assert_eq!(result.summary.total_symbols, 4);
        assert_eq!(result.summary.dead_symbols, 1);
        assert_eq!(alive_count, 3);
    }

    #[test]
    fn test_impl_method_alive_when_type_alive() {
        // When a struct/type is alive, methods from its `impl` block (which have
        // parent = the type) should also be alive — even if the method's kind is
        // not EnumVariant and the parent's kind is not Interface.
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Struct "Foo" is alive (public in entry file).
        // impl Display for Foo { fn fmt() } — fmt has parent=Foo.
        let mut fmt_method = make_sym(2, "fmt", SymbolKind::Method, 1, Visibility::Public);
        fmt_method.parent = Some(SymbolId(1));

        // impl Foo { fn as_str() } — as_str has parent=Foo.
        let mut as_str_method = make_sym(3, "as_str", SymbolKind::Method, 1, Visibility::Public);
        as_str_method.parent = Some(SymbolId(1));

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "Foo", SymbolKind::Struct, 1, Visibility::Public),
                fmt_method,
                as_str_method,
                make_sym(4, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(!dead_names.contains(&"Foo"), "Foo should be alive (public in entry file)");
        assert!(!dead_names.contains(&"fmt"), "fmt should be alive when parent Foo is alive");
        assert!(!dead_names.contains(&"as_str"), "as_str should be alive when parent Foo is alive");
    }

    #[test]
    fn test_trait_impl_method_alive_via_inheritance_chain() {
        // When a struct is alive and implements a trait, the convergence loop
        // should seed: struct alive → child methods seeded → BFS follows refs
        // → trait becomes alive → inheritance propagation seeds other implementors.
        use crate::model::graph::SymbolGraph;
        use crate::model::*;

        let mut sym_graph = SymbolGraph::new();
        let mut file_graph = FileGraph::new();

        file_graph.add_file(make_file(1, "src/index.ts", true));
        sym_graph.add_file(make_file_record(1, "src/index.ts"));

        // Alive struct Registry with method with_defaults() that references Parser trait
        let mut with_defaults = make_sym(2, "with_defaults", SymbolKind::Method, 1, Visibility::Public);
        with_defaults.parent = Some(SymbolId(1));

        // Parser trait with method parse()
        let mut trait_method = make_sym(4, "parse", SymbolKind::Method, 1, Visibility::Public);
        trait_method.parent = Some(SymbolId(3));

        // RustParser implements Parser, has method do_parse()
        let mut impl_method = make_sym(6, "do_parse", SymbolKind::Method, 1, Visibility::Public);
        impl_method.parent = Some(SymbolId(5));

        let mut inherit_ref = make_ref(3, 5, 3, RefKind::Inheritance, 1);
        inherit_ref.target_name = Some("Parser".to_string());

        sym_graph.add_parse_result(ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_sym(1, "Registry", SymbolKind::Struct, 1, Visibility::Public),
                with_defaults,
                make_sym(3, "Parser", SymbolKind::Interface, 1, Visibility::Public),
                trait_method,
                make_sym(5, "RustParser", SymbolKind::Struct, 1, Visibility::Private),
                impl_method,
                make_sym(7, "main", SymbolKind::Function, 1, Visibility::Public),
            ],
            references: vec![
                make_ref(1, 7, 1, RefKind::Call, 1),    // main -> Registry
                make_ref(2, 2, 5, RefKind::Call, 1),     // with_defaults -> RustParser
                inherit_ref,                              // RustParser implements Parser
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        });

        let result = detect_dead_symbols(&sym_graph, &file_graph, &empty_linker(), &HashSet::new());
        let dead_names: Vec<&str> = result.dead_symbols.iter().map(|s| s.name.as_str()).collect();

        // Registry is alive (public in entry file)
        assert!(!dead_names.contains(&"Registry"), "Registry should be alive");
        // with_defaults is alive (child of alive Registry, seeded by convergence loop)
        assert!(!dead_names.contains(&"with_defaults"), "with_defaults should be alive (child of alive Registry)");
        // RustParser is alive (referenced by with_defaults via BFS)
        assert!(!dead_names.contains(&"RustParser"),
            "RustParser should be alive (referenced by with_defaults), dead: {:?}", dead_names);
        // do_parse is alive (child of alive RustParser, seeded by next convergence iteration)
        assert!(!dead_names.contains(&"do_parse"),
            "do_parse should be alive (child of alive RustParser), dead: {:?}", dead_names);
        // Parser is alive (public in entry file), parse is its child
        assert!(!dead_names.contains(&"parse"),
            "parse should be alive (child of alive Parser), dead: {:?}", dead_names);
    }

    #[test]
    fn test_pub_mod_reexport_not_dead_when_child_is_used() {
        // Scenario: analysis/mod.rs exports "cycles" via `pub mod cycles;`
        // (is_mod_declaration edge). Another file imports from analysis/cycles.rs
        // directly using `analysis::cycles::detect_cycles`. The "cycles" export
        // in mod.rs should NOT be flagged as dead, even though nobody imports the
        // name "cycles" from mod.rs directly.
        let mut graph = FileGraph::new();

        // Entry point: src/main.rs
        graph.add_file(make_file(1, "src/main.rs", true));
        // mod.rs barrel file that re-exports child modules
        graph.add_file(make_file_with_exports(
            2,
            "src/analysis/mod.rs",
            false,
            &["cycles", "dead_code"],
        ));
        // Child module file (used)
        graph.add_file(make_file_with_exports(
            3,
            "src/analysis/cycles.rs",
            false,
            &["detect_cycles"],
        ));
        // Another child module file (also reachable via mod declaration)
        graph.add_file(make_file_with_exports(
            4,
            "src/analysis/dead_code.rs",
            false,
            &["detect_dead_code"],
        ));

        // mod.rs declares its children via pub mod (is_mod_declaration edges)
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["cycles".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(4),
            imported_names: vec!["dead_code".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 2,
        });

        // main.rs imports from mod.rs (which makes mod.rs reachable)
        graph.add_import(make_edge(1, 2, &["analysis"]));
        // main.rs also imports directly from cycles.rs
        graph.add_import(make_edge(1, 3, &["detect_cycles"]));

        let result = detect_dead_code(&graph, DeadCodeScope::Exports, &HashSet::new());
        let dead_export_pairs: Vec<(&str, &str)> = result
            .dead_exports
            .iter()
            .map(|e| (e.path.to_str().unwrap_or(""), e.export_name.as_str()))
            .collect();

        // "cycles" export in mod.rs should NOT be dead:
        // The mod_declaration edge target (cycles.rs) is reachable, so the
        // re-export is structurally necessary.
        assert!(
            !dead_export_pairs.contains(&("src/analysis/mod.rs", "cycles")),
            "pub mod cycles export should not be dead when cycles.rs is reachable. Dead: {:?}",
            dead_export_pairs
        );

        // "dead_code" export in mod.rs should also NOT be dead:
        // dead_code.rs is reachable via the mod_declaration edge from mod.rs
        // (which is itself reachable from main.rs).
        assert!(
            !dead_export_pairs.contains(&("src/analysis/mod.rs", "dead_code")),
            "pub mod dead_code export should not be dead when mod.rs is reachable (mod declaration makes child reachable). Dead: {:?}",
            dead_export_pairs
        );
    }

    #[test]
    fn test_pub_mod_reexport_dead_when_parent_unreachable() {
        // If mod.rs itself is not reachable, then its pub mod exports
        // should be dead (along with the child modules).
        let mut graph = FileGraph::new();

        // Entry point: src/main.rs
        graph.add_file(make_file(1, "src/main.rs", true));
        // Unreachable mod.rs (not imported by anyone)
        graph.add_file(make_file_with_exports(
            2,
            "src/orphan/mod.rs",
            false,
            &["child"],
        ));
        // Child of unreachable parent
        graph.add_file(make_file_with_exports(
            3,
            "src/orphan/child.rs",
            false,
            &["something"],
        ));

        // mod.rs declares child via pub mod
        graph.add_import(FileImport {
            from: FileId(2),
            to: FileId(3),
            imported_names: vec!["child".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });

        // No imports from main.rs to mod.rs -- everything is dead
        let result = detect_dead_code(&graph, DeadCodeScope::Both, &HashSet::new());

        // Both mod.rs and child.rs should be dead files
        assert_eq!(result.dead_files.len(), 2);

        // Exports should also be dead
        let dead_names: Vec<&str> = result
            .dead_exports
            .iter()
            .map(|e| e.export_name.as_str())
            .collect();
        assert!(
            dead_names.contains(&"something"),
            "Child export should be dead when parent is unreachable. Dead: {:?}",
            dead_names
        );
    }
}
