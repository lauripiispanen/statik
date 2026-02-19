use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::file_graph::FileGraph;
use crate::model::FileId;

use super::Confidence;

/// Direction for dependency analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// What this file imports (upstream dependencies).
    Imports,
    /// What imports this file (downstream dependents).
    ImportedBy,
    /// Both directions.
    Both,
}

/// A dependency in the result tree with depth info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepNode {
    pub file_id: FileId,
    pub path: PathBuf,
    pub depth: usize,
    pub imported_names: Vec<String>,
}

/// Result of dependency analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepsResult {
    /// The file being analyzed.
    pub target_file: FileId,
    pub target_path: PathBuf,
    /// Upstream dependencies (files this file imports).
    pub imports: Vec<DepNode>,
    /// Downstream dependents (files that import this file).
    pub imported_by: Vec<DepNode>,
    pub confidence: Confidence,
    pub summary: DepsSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepsSummary {
    pub direct_imports: usize,
    pub transitive_imports: usize,
    pub direct_importers: usize,
    pub transitive_importers: usize,
}

/// Analyze dependency chain for a given file.
///
/// Returns the upstream (imports) and downstream (imported_by)
/// dependency chains, optionally limited by max_depth.
pub fn analyze_deps(
    graph: &FileGraph,
    target: FileId,
    direction: Direction,
    transitive: bool,
    max_depth: Option<usize>,
) -> Option<DepsResult> {
    let file_info = graph.files.get(&target)?;
    let target_path = file_info.path.clone();
    let max_depth = max_depth.unwrap_or(usize::MAX);

    let mut imports = Vec::new();
    let mut imported_by = Vec::new();

    if direction == Direction::Imports || direction == Direction::Both {
        imports = if transitive {
            bfs_deps(graph, target, true, max_depth)
        } else {
            direct_deps(graph, target, true)
        };
    }

    if direction == Direction::ImportedBy || direction == Direction::Both {
        imported_by = if transitive {
            bfs_deps(graph, target, false, max_depth)
        } else {
            direct_deps(graph, target, false)
        };
    }

    let confidence = if graph.unresolved.is_empty() {
        Confidence::Certain
    } else if graph.has_unresolved_imports(target) {
        Confidence::Medium
    } else {
        Confidence::High
    };

    let direct_import_count = graph.direct_imports(target).len();
    let direct_importer_count = graph.direct_importers(target).len();

    Some(DepsResult {
        target_file: target,
        target_path,
        summary: DepsSummary {
            direct_imports: direct_import_count,
            transitive_imports: imports.len(),
            direct_importers: direct_importer_count,
            transitive_importers: imported_by.len(),
        },
        imports,
        imported_by,
        confidence,
    })
}

/// Get direct dependencies with imported name information.
fn direct_deps(graph: &FileGraph, file: FileId, forward: bool) -> Vec<DepNode> {
    let edges = if forward {
        graph.imports.get(&file)
    } else {
        graph.imported_by.get(&file)
    };

    let mut seen = HashSet::new();
    let mut result = Vec::new();

    if let Some(edges) = edges {
        for edge in edges {
            let neighbor = if forward { edge.to } else { edge.from };
            if seen.insert(neighbor) {
                let path = graph
                    .files
                    .get(&neighbor)
                    .map(|f| f.path.clone())
                    .unwrap_or_default();

                // Collect all imported names for this edge
                let mut names: Vec<String> = edges
                    .iter()
                    .filter(|e| if forward { e.to } else { e.from } == neighbor)
                    .flat_map(|e| e.imported_names.clone())
                    .collect();
                names.sort();
                names.dedup();

                result.push(DepNode {
                    file_id: neighbor,
                    path,
                    depth: 1,
                    imported_names: names,
                });
            }
        }
    }

    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

/// BFS for transitive dependencies, collecting depth information.
fn bfs_deps(graph: &FileGraph, start: FileId, forward: bool, max_depth: usize) -> Vec<DepNode> {
    let mut visited = HashSet::new();
    visited.insert(start);
    let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();
    let mut result = Vec::new();

    // Seed with direct neighbors
    let neighbors = if forward {
        graph.direct_imports(start)
    } else {
        graph.direct_importers(start)
    };

    for n in neighbors {
        if visited.insert(n) {
            queue.push_back((n, 1));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        let path = graph
            .files
            .get(&current)
            .map(|f| f.path.clone())
            .unwrap_or_default();

        // Collect imported names between start and this file at depth 1,
        // or between the parent and this file at deeper levels
        let imported_names = collect_edge_names(graph, start, current, forward);

        result.push(DepNode {
            file_id: current,
            path,
            depth,
            imported_names,
        });

        if depth < max_depth {
            let next = if forward {
                graph.direct_imports(current)
            } else {
                graph.direct_importers(current)
            };
            for n in next {
                if visited.insert(n) {
                    queue.push_back((n, depth + 1));
                }
            }
        }
    }

    result.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.path.cmp(&b.path)));
    result
}

/// Collect the imported names on edges between two files.
fn collect_edge_names(
    _graph: &FileGraph,
    _from: FileId,
    _to: FileId,
    _forward: bool,
) -> Vec<String> {
    // For simplicity, we don't track the exact path of names through
    // transitive chains. Direct edges have names; deeper edges just
    // indicate transitive reachability.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::file_graph::{FileImport, FileInfo};
    use crate::model::Language;

    fn make_file(id: u64, path: &str) -> FileInfo {
        FileInfo {
            id: FileId(id),
            path: PathBuf::from(path),
            language: Language::TypeScript,
            exports: vec![],
            is_entry_point: false,
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
    fn test_direct_imports() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2, &["foo"]));
        graph.add_import(make_edge(1, 3, &["bar"]));

        let result = analyze_deps(&graph, FileId(1), Direction::Imports, false, None).unwrap();
        assert_eq!(result.imports.len(), 2);
        assert_eq!(result.summary.direct_imports, 2);
    }

    #[test]
    fn test_direct_importers() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(2, 1, &["foo"]));
        graph.add_import(make_edge(3, 1, &["bar"]));

        let result = analyze_deps(&graph, FileId(1), Direction::ImportedBy, false, None).unwrap();
        assert_eq!(result.imported_by.len(), 2);
        assert_eq!(result.summary.direct_importers, 2);
    }

    #[test]
    fn test_transitive_imports() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2, &["foo"]));
        graph.add_import(make_edge(2, 3, &["bar"]));

        let result = analyze_deps(&graph, FileId(1), Direction::Imports, true, None).unwrap();
        assert_eq!(result.imports.len(), 2); // b and c
        assert_eq!(result.imports[0].depth, 1); // b is direct
        assert_eq!(result.imports[1].depth, 2); // c is transitive
    }

    #[test]
    fn test_max_depth_limits_traversal() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_file(make_file(4, "d.ts"));
        graph.add_import(make_edge(1, 2, &["x"]));
        graph.add_import(make_edge(2, 3, &["y"]));
        graph.add_import(make_edge(3, 4, &["z"]));

        let result = analyze_deps(&graph, FileId(1), Direction::Imports, true, Some(2)).unwrap();
        // Should only find b (depth 1) and c (depth 2), not d (depth 3)
        assert_eq!(result.imports.len(), 2);
    }

    #[test]
    fn test_both_directions() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2, &["x"]));
        graph.add_import(make_edge(3, 2, &["y"]));

        let result = analyze_deps(&graph, FileId(2), Direction::Both, false, None).unwrap();
        assert_eq!(result.imports.len(), 0); // b doesn't import anything
        assert_eq!(result.imported_by.len(), 2); // a and c import b
    }

    #[test]
    fn test_nonexistent_file_returns_none() {
        let graph = FileGraph::new();
        let result = analyze_deps(&graph, FileId(999), Direction::Both, false, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_circular_deps_handled() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_import(make_edge(1, 2, &["x"]));
        graph.add_import(make_edge(2, 1, &["y"]));

        // Should not infinite loop
        let result = analyze_deps(&graph, FileId(1), Direction::Imports, true, None).unwrap();
        assert_eq!(result.imports.len(), 1); // just b (a is already visited)
    }

    #[test]
    fn test_isolated_file() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "lonely.ts"));

        let result = analyze_deps(&graph, FileId(1), Direction::Both, true, None).unwrap();
        assert!(result.imports.is_empty());
        assert!(result.imported_by.is_empty());
        assert_eq!(result.summary.direct_imports, 0);
        assert_eq!(result.summary.direct_importers, 0);
    }
}
