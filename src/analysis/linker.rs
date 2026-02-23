use std::collections::HashSet;

use crate::analysis::Confidence;
use crate::model::file_graph::{FileGraph, FileImport};
use crate::model::{FileId, Language, SymbolId};

/// A cross-file reference linking an import site in one file to an exported symbol in another.
#[derive(Debug, Clone)]
pub struct CrossFileRef {
    /// File containing the import statement.
    pub source_file: FileId,
    /// File containing the exported symbol definition.
    pub target_file: FileId,
    /// SymbolId of the exported symbol in the target file.
    pub target_symbol: SymbolId,
    /// The name used in the import (e.g. "foo" in `import { foo } from './bar'`).
    pub imported_name: String,
    /// Line number of the import statement.
    pub line: usize,
    /// How confident we are in this link.
    pub confidence: Confidence,
}

/// Result of the cross-file linking pass.
#[derive(Debug)]
pub struct LinkingResult {
    pub resolved: usize,
    pub unresolved: usize,
    pub references: Vec<CrossFileRef>,
}

/// Resolve an exported name from a file, following re-export chains.
///
/// Returns the (FileId, SymbolId) of the original definition.
/// Uses `visited` to detect cycles in re-export chains.
fn resolve_export(
    file_id: FileId,
    name: &str,
    is_default: bool,
    file_graph: &FileGraph,
    visited: &mut HashSet<(FileId, String)>,
) -> Option<(FileId, SymbolId)> {
    let key = (file_id, name.to_string());
    if !visited.insert(key) {
        return None; // cycle detected
    }

    let file_info = file_graph.get_file(file_id)?;

    // Look for a matching export in this file
    for export in &file_info.exports {
        let name_matches = if is_default {
            export.is_default
        } else {
            export.exported_name == name
        };

        if !name_matches {
            continue;
        }

        if export.is_reexport {
            // Follow the re-export chain: find which file this re-exports from
            if let Some(source_path) = &export.source_path {
                // Find the target file for this re-export via file graph edges
                if let Some(edges) = file_graph.import_edges(file_id) {
                    for edge in edges {
                        if edge_matches_source_path(edge, file_id, source_path, file_graph) {
                            let result =
                                resolve_export(edge.to, name, is_default, file_graph, visited);
                            if result.is_some() {
                                return result;
                            }
                        }
                    }
                }
            }
            // If we can't follow the chain, return the re-export symbol itself
            return Some((file_id, export.symbol));
        }

        // Direct export: return it
        return Some((file_id, export.symbol));
    }

    // Check wildcard re-exports: export * from './other'
    for export in &file_info.exports {
        if export.exported_name == "*" && export.is_reexport {
            if let Some(source_path) = &export.source_path {
                if let Some(edges) = file_graph.import_edges(file_id) {
                    for edge in edges {
                        if edge_matches_source_path(edge, file_id, source_path, file_graph) {
                            let result =
                                resolve_export(edge.to, name, is_default, file_graph, visited);
                            if result.is_some() {
                                return result;
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Check if a file graph edge corresponds to a particular source_path from a re-export.
///
/// We match by checking if the edge's target file path ends with the source_path
/// on a path-segment boundary (after stripping leading "./" or "../" prefixes
/// and ignoring extensions).
fn edge_matches_source_path(
    edge: &FileImport,
    _from_file: FileId,
    source_path: &str,
    file_graph: &FileGraph,
) -> bool {
    if let Some(target_info) = file_graph.get_file(edge.to) {
        let target_path_str = target_info.path.to_string_lossy();

        // Strip all leading "./" and "../" prefixes
        let mut clean_source = source_path;
        loop {
            if let Some(rest) = clean_source.strip_prefix("./") {
                clean_source = rest;
            } else if let Some(rest) = clean_source.strip_prefix("../") {
                clean_source = rest;
            } else {
                break;
            }
        }

        // Strip the final extension from the target path (only the last one)
        let target_stem = strip_final_extension(&target_path_str);

        // Check segment-boundary match: clean_source must match a suffix
        // of the target path at a '/' boundary (or match the whole path).
        if path_segments_end_with(target_stem, clean_source)
            || path_segments_end_with(&target_path_str, clean_source)
        {
            return true;
        }

        // Index file resolution: "./utils" should match "utils/index.ts"
        let index_pattern = format!("{}/index", clean_source);
        path_segments_end_with(target_stem, &index_pattern)
    } else {
        false
    }
}

/// Strip the final file extension (e.g. ".ts", ".d.ts") from a path string.
fn strip_final_extension(path: &str) -> &str {
    // Handle compound extensions like ".d.ts"
    if let Some(stripped) = path.strip_suffix(".d.ts") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".d.mts") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".d.cts") {
        return stripped;
    }
    // Single extensions: find the last '.' after the last '/'
    let after_last_slash = path.rfind('/').map(|i| i + 1).unwrap_or(0);
    if let Some(dot_pos) = path[after_last_slash..].rfind('.') {
        &path[..after_last_slash + dot_pos]
    } else {
        path
    }
}

/// Check if `haystack` ends with `needle` on a path-segment boundary.
/// That is, `needle` must match a suffix of `haystack` such that the character
/// before the match (if any) is '/'.
fn path_segments_end_with(haystack: &str, needle: &str) -> bool {
    if haystack == needle {
        return true;
    }
    if let Some(rest) = haystack.strip_suffix(needle) {
        // The character immediately before the match must be '/'
        rest.ends_with('/')
    } else {
        false
    }
}

/// Run the cross-file symbol linking pass.
///
/// For each file import edge in the FileGraph, matches imported names
/// against the target file's exports to produce cross-file references.
/// Re-export chains are followed with cycle detection.
///
/// This is an in-memory-only operation; results are not persisted to the database.
pub fn link_cross_file_symbols(
    file_graph: &FileGraph,
) -> LinkingResult {
    let mut references = Vec::new();
    let mut resolved = 0usize;
    let mut unresolved = 0usize;

    // Iterate over all import edges
    for (_file_id, edges) in file_graph.all_import_edges() {
        for edge in edges {
            // Skip mod declaration edges (structural, not symbol-level)
            if edge.is_mod_declaration {
                continue;
            }

            for name in &edge.imported_names {
                // Skip synthetic imports that aren't real symbol references
                if name.starts_with("@annotation:") || name.starts_with("@type-ref:") {
                    continue;
                }

                // Skip side-effect imports (empty name or "*" with no imported_names)
                if name.is_empty() {
                    continue;
                }

                let is_default = name == "default";
                let is_namespace = name == "*";

                if is_namespace {
                    // Namespace import: create refs to all exports of the target
                    if let Some(target_info) = file_graph.get_file(edge.to) {
                        for export in &target_info.exports {
                            if export.exported_name == "*" {
                                continue; // skip wildcard re-exports themselves
                            }
                            let mut visited = HashSet::new();
                            if let Some((def_file, def_symbol)) = resolve_export(
                                edge.to,
                                &export.exported_name,
                                export.is_default,
                                file_graph,
                                &mut visited,
                            ) {
                                references.push(CrossFileRef {
                                    source_file: edge.from,
                                    target_file: def_file,
                                    target_symbol: def_symbol,
                                    imported_name: export.exported_name.clone(),
                                    line: edge.line,
                                    confidence: Confidence::Medium,
                                });
                                resolved += 1;
                            } else {
                                unresolved += 1;
                            }
                        }
                    }
                    continue;
                }

                let mut visited = HashSet::new();
                if let Some((def_file, def_symbol)) =
                    resolve_export(edge.to, name, is_default, file_graph, &mut visited)
                {
                    let confidence = if visited.len() > 1 {
                        Confidence::Medium
                    } else {
                        Confidence::High
                    };
                    references.push(CrossFileRef {
                        source_file: edge.from,
                        target_file: def_file,
                        target_symbol: def_symbol,
                        imported_name: name.clone(),
                        line: edge.line,
                        confidence,
                    });
                    resolved += 1;
                } else if let Some(target_info) = file_graph.get_file(edge.to) {
                    // Module-path imports: when the imported name matches the target
                    // file's stem, resolve to all exports (e.g. Rust's `use crate::foo`).
                    if super::supports_module_stem_import(target_info.language) {
                        let file_stem = target_info
                            .path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        if name == file_stem {
                            let mut found_any = false;
                            for export in &target_info.exports {
                                if export.exported_name == "*" {
                                    continue;
                                }
                                let mut vis = HashSet::new();
                                if let Some((def_file, def_symbol)) = resolve_export(
                                    edge.to,
                                    &export.exported_name,
                                    export.is_default,
                                    file_graph,
                                    &mut vis,
                                ) {
                                    references.push(CrossFileRef {
                                        source_file: edge.from,
                                        target_file: def_file,
                                        target_symbol: def_symbol,
                                        imported_name: export.exported_name.clone(),
                                        line: edge.line,
                                        confidence: Confidence::Medium,
                                    });
                                    resolved += 1;
                                    found_any = true;
                                }
                            }
                            if !found_any {
                                unresolved += 1;
                            }
                        } else {
                            unresolved += 1;
                        }
                    } else {
                        unresolved += 1;
                    }
                } else {
                    unresolved += 1;
                }
            }
        }
    }

    // Deduplicate references from grouped use imports (e.g. `use crate::model::{A, B, C}`)
    // where the same (source_file, target_symbol, line) can appear multiple times.
    {
        let mut seen = HashSet::new();
        references.retain(|r| seen.insert((r.source_file, r.target_symbol, r.line)));
    }

    LinkingResult {
        resolved,
        unresolved,
        references,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::file_graph::{FileGraph, FileImport, FileInfo};
    use crate::model::{ExportRecord, FileId, Language, SymbolId};

    fn make_export(
        file: FileId,
        symbol: SymbolId,
        name: &str,
        is_default: bool,
        is_reexport: bool,
        source_path: Option<&str>,
    ) -> ExportRecord {
        ExportRecord {
            file,
            symbol,
            exported_name: name.to_string(),
            is_default,
            is_reexport,
            is_type_only: false,
            source_path: source_path.map(|s| s.to_string()),
            line: 1,
        }
    }

    fn make_file_info(
        id: FileId,
        path: &str,
        exports: Vec<ExportRecord>,
        is_entry: bool,
    ) -> FileInfo {
        FileInfo {
            id,
            path: std::path::PathBuf::from(path),
            language: Language::TypeScript,
            exports,
            is_entry_point: is_entry,
        }
    }

    fn make_edge(
        from: FileId,
        to: FileId,
        names: Vec<&str>,
        line: usize,
    ) -> FileImport {
        FileImport {
            from,
            to,
            imported_names: names.into_iter().map(|s| s.to_string()).collect(),
            is_type_only: false,
            is_mod_declaration: false,
            line,
        }
    }

    /// Helper: build a simple graph and run the linker, returning the result.
    fn build_and_link(
        files: Vec<FileInfo>,
        edges: Vec<FileImport>,
    ) -> LinkingResult {
        let mut graph = FileGraph::new();
        for f in files {
            graph.add_file(f);
        }
        for e in edges {
            graph.add_import(e);
        }

        link_cross_file_symbols(&graph)
    }

    #[test]
    fn test_basic_cross_file_link() {
        // A imports "foo" from B, B exports "foo"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(FileId(2), SymbolId(100), "foo", false, false, None)],
                    false,
                ),
            ],
            vec![make_edge(FileId(1), FileId(2), vec!["foo"], 5)],
        );

        assert_eq!(result.resolved, 1);
        assert_eq!(result.unresolved, 0);
        assert_eq!(result.references.len(), 1);

        let r = &result.references[0];
        assert_eq!(r.source_file, FileId(1));
        assert_eq!(r.target_file, FileId(2));
        assert_eq!(r.target_symbol, SymbolId(100));
        assert_eq!(r.imported_name, "foo");
        assert_eq!(r.line, 5);
        assert_eq!(r.confidence, Confidence::High);
    }

    #[test]
    fn test_named_reexport_chain() {
        // A imports "foo" from B (barrel), B re-exports "foo" from C, C defines "foo"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/barrel.ts",
                    vec![make_export(
                        FileId(2),
                        SymbolId(200),
                        "foo",
                        false,
                        true,
                        Some("./c"),
                    )],
                    false,
                ),
                make_file_info(
                    FileId(3),
                    "src/c.ts",
                    vec![make_export(FileId(3), SymbolId(300), "foo", false, false, None)],
                    false,
                ),
            ],
            vec![
                make_edge(FileId(1), FileId(2), vec!["foo"], 1),
                make_edge(FileId(2), FileId(3), vec!["foo"], 1),
            ],
        );

        // The A->B edge resolves "foo" through the re-export chain to C,
        // and the B->C edge also resolves "foo" directly to C's export.
        let refs_from_a: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(1))
            .collect();
        assert_eq!(refs_from_a.len(), 1);
        // A's import should resolve to C's symbol, not B's re-export
        assert_eq!(refs_from_a[0].target_file, FileId(3));
        assert_eq!(refs_from_a[0].target_symbol, SymbolId(300));
    }

    #[test]
    fn test_wildcard_reexport() {
        // A imports "foo" from B, B does `export * from './c'`, C exports "foo"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(
                        FileId(2),
                        SymbolId(200),
                        "*",
                        false,
                        true,
                        Some("./c"),
                    )],
                    false,
                ),
                make_file_info(
                    FileId(3),
                    "src/c.ts",
                    vec![make_export(FileId(3), SymbolId(300), "foo", false, false, None)],
                    false,
                ),
            ],
            vec![
                make_edge(FileId(1), FileId(2), vec!["foo"], 1),
                make_edge(FileId(2), FileId(3), vec!["*"], 1),
            ],
        );

        // A->B: "foo" resolves through wildcard re-export to C
        let refs_from_a: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(1))
            .collect();
        assert_eq!(refs_from_a.len(), 1);
        assert_eq!(refs_from_a[0].target_file, FileId(3));
        assert_eq!(refs_from_a[0].target_symbol, SymbolId(300));
    }

    #[test]
    fn test_circular_reexport_no_infinite_loop() {
        // A imports "foo" from B, B re-exports "foo" from C, C re-exports "foo" from B (cycle)
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(
                        FileId(2),
                        SymbolId(200),
                        "foo",
                        false,
                        true,
                        Some("./c"),
                    )],
                    false,
                ),
                make_file_info(
                    FileId(3),
                    "src/c.ts",
                    vec![make_export(
                        FileId(3),
                        SymbolId(300),
                        "foo",
                        false,
                        true,
                        Some("./b"),
                    )],
                    false,
                ),
            ],
            vec![
                make_edge(FileId(1), FileId(2), vec!["foo"], 1),
                make_edge(FileId(2), FileId(3), vec!["foo"], 1),
                make_edge(FileId(3), FileId(2), vec!["foo"], 1),
            ],
        );

        // Should not panic or hang. Each edge is processed independently.
        // The cycle means re-export chains hit the visited set and fall back.
        // Just verify it completes and produces some result without infinite loop.
        let total = result.resolved + result.unresolved;
        assert!(total >= 1, "should process at least one import name");
    }

    #[test]
    fn test_default_import() {
        // A imports "default" from B, B has a default export
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(
                        FileId(2),
                        SymbolId(100),
                        "default",
                        true,
                        false,
                        None,
                    )],
                    false,
                ),
            ],
            vec![make_edge(FileId(1), FileId(2), vec!["default"], 3)],
        );

        assert_eq!(result.resolved, 1);
        assert_eq!(result.references.len(), 1);
        assert_eq!(result.references[0].target_symbol, SymbolId(100));
    }

    #[test]
    fn test_missing_export_graceful_skip() {
        // A imports "bar" from B, but B only exports "foo"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(FileId(2), SymbolId(100), "foo", false, false, None)],
                    false,
                ),
            ],
            vec![make_edge(FileId(1), FileId(2), vec!["bar"], 1)],
        );

        assert_eq!(result.resolved, 0);
        assert_eq!(result.unresolved, 1);
        assert!(result.references.is_empty());
    }

    #[test]
    fn test_multiple_importers_same_symbol() {
        // A and C both import "foo" from B
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![make_export(FileId(2), SymbolId(100), "foo", false, false, None)],
                    false,
                ),
                make_file_info(FileId(3), "src/c.ts", vec![], false),
            ],
            vec![
                make_edge(FileId(1), FileId(2), vec!["foo"], 1),
                make_edge(FileId(3), FileId(2), vec!["foo"], 5),
            ],
        );

        assert_eq!(result.resolved, 2);
        assert_eq!(result.references.len(), 2);

        let from_a: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(1))
            .collect();
        let from_c: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(3))
            .collect();
        assert_eq!(from_a.len(), 1);
        assert_eq!(from_c.len(), 1);
        assert_eq!(from_a[0].target_symbol, SymbolId(100));
        assert_eq!(from_c[0].target_symbol, SymbolId(100));
    }

    #[test]
    fn test_namespace_import_creates_refs_to_all_exports() {
        // A does `import * as B from './b'`, B exports "foo" and "bar"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/b.ts",
                    vec![
                        make_export(FileId(2), SymbolId(100), "foo", false, false, None),
                        make_export(FileId(2), SymbolId(101), "bar", false, false, None),
                    ],
                    false,
                ),
            ],
            vec![make_edge(FileId(1), FileId(2), vec!["*"], 1)],
        );

        assert_eq!(result.resolved, 2);
        assert_eq!(result.references.len(), 2);

        let names: HashSet<&str> = result
            .references
            .iter()
            .map(|r| r.imported_name.as_str())
            .collect();
        assert!(names.contains("foo"));
        assert!(names.contains("bar"));

        // All should have Medium confidence for namespace imports
        for r in &result.references {
            assert_eq!(r.confidence, Confidence::Medium);
        }
    }

    #[test]
    fn test_mod_declaration_edges_skipped() {
        // Rust mod declarations should be skipped
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/lib.rs", vec![], true));
        graph.add_file(make_file_info(
            FileId(2),
            "src/foo.rs",
            vec![make_export(FileId(2), SymbolId(100), "bar", false, false, None)],
            false,
        ));
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["foo".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });

        let result = link_cross_file_symbols(&graph);

        assert_eq!(result.resolved, 0);
        assert_eq!(result.unresolved, 0);
        assert!(result.references.is_empty());
    }

    #[test]
    fn test_annotation_imports_skipped() {
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.java", vec![], false),
                make_file_info(FileId(2), "src/b.java", vec![], false),
            ],
            vec![make_edge(
                FileId(1),
                FileId(2),
                vec!["@annotation:SpringBootApplication"],
                1,
            )],
        );

        assert_eq!(result.resolved, 0);
        assert_eq!(result.unresolved, 0);
    }

    #[test]
    fn test_strip_final_extension() {
        assert_eq!(strip_final_extension("src/foo.ts"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.d.ts"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.d.mts"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.d.cts"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.js"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.tsx"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.jsx"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.rs"), "src/foo");
        assert_eq!(strip_final_extension("src/foo.java"), "src/foo");
        assert_eq!(strip_final_extension("src/foo"), "src/foo");
        assert_eq!(strip_final_extension("foo"), "foo");
        assert_eq!(strip_final_extension("foo.bar.ts"), "foo.bar");
    }

    #[test]
    fn test_path_segments_end_with() {
        // Exact match
        assert!(path_segments_end_with("src/foo", "src/foo"));
        // Segment-boundary suffix match
        assert!(path_segments_end_with("src/foo", "foo"));
        assert!(path_segments_end_with("a/b/c", "b/c"));
        assert!(path_segments_end_with("a/b/c", "c"));

        // Must NOT match in the middle of a segment
        assert!(!path_segments_end_with("src/abc", "b"));
        assert!(!path_segments_end_with("src/abc", "bc"));
        assert!(!path_segments_end_with("src/abc", "c"));
        assert!(!path_segments_end_with("src/foobar", "bar"));

        // No match at all
        assert!(!path_segments_end_with("src/foo", "baz"));
    }

    #[test]
    fn test_edge_matches_source_path_basic() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/a.ts", vec![], false));
        graph.add_file(make_file_info(FileId(2), "src/b.ts", vec![], false));

        let edge = make_edge(FileId(1), FileId(2), vec!["foo"], 1);

        // "./b" should match "src/b.ts"
        assert!(edge_matches_source_path(&edge, FileId(1), "./b", &graph));
        // "b" should match "src/b.ts"
        assert!(edge_matches_source_path(&edge, FileId(1), "b", &graph));
        // "./b.ts" should match "src/b.ts"
        assert!(edge_matches_source_path(&edge, FileId(1), "./b.ts", &graph));
    }

    #[test]
    fn test_edge_matches_source_path_no_false_positives() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/a.ts", vec![], false));
        graph.add_file(make_file_info(FileId(2), "src/abc.ts", vec![], false));

        let edge = make_edge(FileId(1), FileId(2), vec!["foo"], 1);

        // "b" should NOT match "src/abc.ts" (substring but not segment-boundary)
        assert!(!edge_matches_source_path(&edge, FileId(1), "b", &graph));
        assert!(!edge_matches_source_path(&edge, FileId(1), "./b", &graph));
        // "bc" should NOT match "src/abc.ts"
        assert!(!edge_matches_source_path(&edge, FileId(1), "bc", &graph));
    }

    #[test]
    fn test_edge_matches_source_path_multiple_dotdot() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/a/b/c.ts", vec![], false));
        graph.add_file(make_file_info(FileId(2), "src/d.ts", vec![], false));

        let edge = make_edge(FileId(1), FileId(2), vec!["foo"], 1);

        // "../../d" should match "src/d.ts" after stripping both "../"
        assert!(edge_matches_source_path(
            &edge,
            FileId(1),
            "../../d",
            &graph
        ));
    }

    #[test]
    fn test_edge_matches_source_path_index_file() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/a.ts", vec![], false));
        graph.add_file(make_file_info(
            FileId(2),
            "src/utils/index.ts",
            vec![],
            false,
        ));

        let edge = make_edge(FileId(1), FileId(2), vec!["foo"], 1);

        // "./utils" should match "src/utils/index.ts"
        assert!(edge_matches_source_path(
            &edge,
            FileId(1),
            "./utils",
            &graph
        ));
        // "utils" should also match
        assert!(edge_matches_source_path(
            &edge,
            FileId(1),
            "utils",
            &graph
        ));
    }

    #[test]
    fn test_reexport_chain_gets_medium_confidence() {
        // A imports "foo" from B (barrel), B re-exports "foo" from C, C defines "foo"
        let result = build_and_link(
            vec![
                make_file_info(FileId(1), "src/a.ts", vec![], false),
                make_file_info(
                    FileId(2),
                    "src/barrel.ts",
                    vec![make_export(
                        FileId(2),
                        SymbolId(200),
                        "foo",
                        false,
                        true,
                        Some("./c"),
                    )],
                    false,
                ),
                make_file_info(
                    FileId(3),
                    "src/c.ts",
                    vec![make_export(FileId(3), SymbolId(300), "foo", false, false, None)],
                    false,
                ),
            ],
            vec![
                make_edge(FileId(1), FileId(2), vec!["foo"], 1),
                make_edge(FileId(2), FileId(3), vec!["foo"], 1),
            ],
        );

        // A's import goes through a re-export chain, so should be Medium confidence
        let refs_from_a: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(1))
            .collect();
        assert_eq!(refs_from_a.len(), 1);
        assert_eq!(refs_from_a[0].confidence, Confidence::Medium);

        // B's direct import to C should be High confidence
        let refs_from_b: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(2))
            .collect();
        assert_eq!(refs_from_b.len(), 1);
        assert_eq!(refs_from_b[0].confidence, Confidence::High);
    }

    #[test]
    fn test_grouped_use_import_deduplicated() {
        // Simulate a grouped use import like `use crate::model::{A, B}` where
        // multiple edge imported_names resolve to the same target symbol.
        // Two edges from the same file/line importing the same name should be deduped.
        let mut graph = FileGraph::new();
        graph.add_file(make_file_info(FileId(1), "src/a.rs", vec![], false));
        graph.add_file(make_file_info(
            FileId(2),
            "src/b.rs",
            vec![make_export(FileId(2), SymbolId(100), "Foo", false, false, None)],
            false,
        ));

        // Two separate edges from the same line importing "Foo" (simulates grouped use)
        graph.add_import(make_edge(FileId(1), FileId(2), vec!["Foo"], 8));
        graph.add_import(make_edge(FileId(1), FileId(2), vec!["Foo"], 8));

        let result = link_cross_file_symbols(&graph);

        // Should be deduplicated to 1 reference
        let refs_from_a: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source_file == FileId(1) && r.imported_name == "Foo")
            .collect();
        assert_eq!(refs_from_a.len(), 1);
    }
}
