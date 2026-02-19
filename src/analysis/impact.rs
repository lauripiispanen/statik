use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::file_graph::FileGraph;
use crate::model::FileId;

use super::Confidence;

/// A file affected by a change, with depth information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedFile {
    pub file_id: FileId,
    pub path: PathBuf,
    pub depth: usize,
}

/// Result of impact analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactResult {
    /// The file being analyzed.
    pub target_file: FileId,
    pub target_path: PathBuf,
    /// All files affected, grouped by depth from the target.
    pub affected: Vec<AffectedFile>,
    /// Affected files grouped by depth for display.
    pub by_depth: HashMap<usize, Vec<AffectedFile>>,
    pub confidence: Confidence,
    pub summary: ImpactSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactSummary {
    pub direct_dependents: usize,
    pub total_affected: usize,
    pub max_depth: usize,
}

/// Analyze the blast radius of changing a file.
///
/// Performs reverse BFS on the imported_by edges (reverse dependency direction).
/// Collects all transitively affected files, grouped by depth.
/// Optionally limited by max_depth.
pub fn analyze_impact(
    graph: &FileGraph,
    target: FileId,
    max_depth: Option<usize>,
) -> Option<ImpactResult> {
    let file_info = graph.files.get(&target)?;
    let target_path = file_info.path.clone();
    let max_depth_limit = max_depth.unwrap_or(usize::MAX);

    // Reverse BFS: follow imported_by edges
    let mut visited = HashSet::new();
    visited.insert(target);
    let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();
    let mut affected = Vec::new();
    let mut by_depth: HashMap<usize, Vec<AffectedFile>> = HashMap::new();

    // Seed with direct importers
    for importer in graph.direct_importers(target) {
        if visited.insert(importer) {
            queue.push_back((importer, 1));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        let path = graph
            .files
            .get(&current)
            .map(|f| f.path.clone())
            .unwrap_or_default();

        let affected_file = AffectedFile {
            file_id: current,
            path,
            depth,
        };

        by_depth
            .entry(depth)
            .or_default()
            .push(affected_file.clone());
        affected.push(affected_file);

        if depth < max_depth_limit {
            for importer in graph.direct_importers(current) {
                if visited.insert(importer) {
                    queue.push_back((importer, depth + 1));
                }
            }
        }
    }

    // Sort by depth, then path
    affected.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.path.cmp(&b.path)));
    for files in by_depth.values_mut() {
        files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    let direct_dependents = graph.direct_importers(target).len();
    let actual_max_depth = affected.last().map(|a| a.depth).unwrap_or(0);

    let confidence = if graph.unresolved.is_empty() {
        Confidence::Certain
    } else {
        // With unresolved imports, there might be additional dependents
        // we can't see, so the blast radius might be larger
        Confidence::High
    };

    Some(ImpactResult {
        target_file: target,
        target_path,
        affected,
        by_depth,
        confidence,
        summary: ImpactSummary {
            direct_dependents,
            total_affected: visited.len() - 1, // exclude target itself
            max_depth: actual_max_depth,
        },
    })
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

    fn make_edge(from: u64, to: u64) -> FileImport {
        FileImport {
            from: FileId(from),
            to: FileId(to),
            imported_names: vec!["x".to_string()],
            is_type_only: false,
            line: 1,
        }
    }

    #[test]
    fn test_no_dependents() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "leaf.ts"));

        let result = analyze_impact(&graph, FileId(1), None).unwrap();
        assert!(result.affected.is_empty());
        assert_eq!(result.summary.total_affected, 0);
    }

    #[test]
    fn test_direct_dependents_only() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "lib.ts"));
        graph.add_file(make_file(2, "a.ts"));
        graph.add_file(make_file(3, "b.ts"));
        graph.add_import(make_edge(2, 1)); // a imports lib
        graph.add_import(make_edge(3, 1)); // b imports lib

        let result = analyze_impact(&graph, FileId(1), None).unwrap();
        assert_eq!(result.affected.len(), 2);
        assert_eq!(result.summary.direct_dependents, 2);
        assert!(result.affected.iter().all(|a| a.depth == 1));
    }

    #[test]
    fn test_transitive_impact() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "core.ts"));
        graph.add_file(make_file(2, "utils.ts"));
        graph.add_file(make_file(3, "app.ts"));
        graph.add_import(make_edge(2, 1)); // utils imports core
        graph.add_import(make_edge(3, 2)); // app imports utils

        let result = analyze_impact(&graph, FileId(1), None).unwrap();
        assert_eq!(result.affected.len(), 2);
        assert_eq!(result.summary.total_affected, 2);
        assert_eq!(result.summary.max_depth, 2);

        let depth1: Vec<_> = result.affected.iter().filter(|a| a.depth == 1).collect();
        assert_eq!(depth1.len(), 1);
        assert_eq!(depth1[0].path, PathBuf::from("utils.ts"));

        let depth2: Vec<_> = result.affected.iter().filter(|a| a.depth == 2).collect();
        assert_eq!(depth2.len(), 1);
        assert_eq!(depth2[0].path, PathBuf::from("app.ts"));
    }

    #[test]
    fn test_max_depth_limits_results() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "core.ts"));
        graph.add_file(make_file(2, "a.ts"));
        graph.add_file(make_file(3, "b.ts"));
        graph.add_file(make_file(4, "c.ts"));
        graph.add_import(make_edge(2, 1));
        graph.add_import(make_edge(3, 2));
        graph.add_import(make_edge(4, 3));

        let result = analyze_impact(&graph, FileId(1), Some(1)).unwrap();
        assert_eq!(result.affected.len(), 1); // only direct dependent
        assert_eq!(result.summary.total_affected, 1);
    }

    #[test]
    fn test_nonexistent_file_returns_none() {
        let graph = FileGraph::new();
        let result = analyze_impact(&graph, FileId(999), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_diamond_dependency_no_duplicates() {
        // c depends on both a and b, both depend on core
        // Changing core: a, b, c are all affected, c only counted once
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "core.ts"));
        graph.add_file(make_file(2, "a.ts"));
        graph.add_file(make_file(3, "b.ts"));
        graph.add_file(make_file(4, "c.ts"));
        graph.add_import(make_edge(2, 1)); // a imports core
        graph.add_import(make_edge(3, 1)); // b imports core
        graph.add_import(make_edge(4, 2)); // c imports a
        graph.add_import(make_edge(4, 3)); // c imports b

        let result = analyze_impact(&graph, FileId(1), None).unwrap();
        assert_eq!(result.summary.total_affected, 3); // a, b, c (each counted once)
    }

    #[test]
    fn test_circular_dependency_terminates() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 1)); // cycle

        let result = analyze_impact(&graph, FileId(1), None).unwrap();
        assert_eq!(result.summary.total_affected, 1); // only b
    }

    #[test]
    fn test_by_depth_grouping() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "core.ts"));
        graph.add_file(make_file(2, "a.ts"));
        graph.add_file(make_file(3, "b.ts"));
        graph.add_file(make_file(4, "c.ts"));
        graph.add_import(make_edge(2, 1));
        graph.add_import(make_edge(3, 1));
        graph.add_import(make_edge(4, 2));

        let result = analyze_impact(&graph, FileId(1), None).unwrap();

        assert_eq!(result.by_depth.get(&1).map(|v| v.len()).unwrap_or(0), 2); // a, b at depth 1
        assert_eq!(result.by_depth.get(&2).map(|v| v.len()).unwrap_or(0), 1); // c at depth 2
    }
}
