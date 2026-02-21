use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::file_graph::FileGraph;
use crate::model::FileId;

use super::Confidence;

/// A circular dependency cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cycle {
    /// Files involved in the cycle, in import order.
    pub files: Vec<CycleFile>,
    /// Length of the cycle (number of files).
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleFile {
    pub file_id: FileId,
    pub path: PathBuf,
}

/// Result of circular dependency detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleResult {
    pub cycles: Vec<Cycle>,
    pub confidence: Confidence,
    pub summary: CycleSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleSummary {
    pub total_files: usize,
    pub files_in_cycles: usize,
    pub cycle_count: usize,
    pub shortest_cycle: usize,
    pub longest_cycle: usize,
}

/// Detect circular dependencies using Tarjan's SCC algorithm.
///
/// A strongly connected component (SCC) with more than one node
/// represents a circular dependency. SCCs are reported as cycles,
/// ordered by length (shortest first, most actionable).
pub fn detect_cycles(graph: &FileGraph) -> CycleResult {
    let file_ids: Vec<FileId> = graph.all_file_ids();

    // Run Tarjan's algorithm
    let sccs = tarjan_scc(graph, &file_ids);

    // Filter to SCCs with more than one node (those are cycles)
    let mut cycles: Vec<Cycle> = sccs
        .into_iter()
        .filter(|scc| scc.len() > 1)
        .map(|scc| {
            let files: Vec<CycleFile> = scc
                .iter()
                .map(|id| {
                    let path = graph
                        .files
                        .get(id)
                        .map(|f| f.path.clone())
                        .unwrap_or_default();
                    CycleFile { file_id: *id, path }
                })
                .collect();
            let length = files.len();
            Cycle { files, length }
        })
        .collect();

    // Sort by cycle length (shortest first -- most actionable)
    cycles.sort_by_key(|c| c.length);

    let files_in_cycles: usize = cycles.iter().map(|c| c.length).sum();
    let shortest = cycles.first().map(|c| c.length).unwrap_or(0);
    let longest = cycles.last().map(|c| c.length).unwrap_or(0);

    // Cycle detection confidence is always certain --
    // the algorithm is exact on the graph we have.
    // Unresolved imports mean missing edges, so there could be
    // additional cycles we can't see.
    let confidence = if graph.unresolved.is_empty() {
        Confidence::Certain
    } else {
        Confidence::High
    };

    CycleResult {
        summary: CycleSummary {
            total_files: graph.file_count(),
            files_in_cycles,
            cycle_count: cycles.len(),
            shortest_cycle: shortest,
            longest_cycle: longest,
        },
        cycles,
        confidence,
    }
}

/// Tarjan's strongly connected components algorithm.
///
/// Returns a list of SCCs, where each SCC is a list of FileIds.
/// Single-node SCCs (no self-loop) are included but will be
/// filtered out by the caller.
fn tarjan_scc(graph: &FileGraph, nodes: &[FileId]) -> Vec<Vec<FileId>> {
    struct TarjanState {
        index_counter: usize,
        stack: Vec<FileId>,
        on_stack: HashMap<FileId, bool>,
        index: HashMap<FileId, usize>,
        lowlink: HashMap<FileId, usize>,
        result: Vec<Vec<FileId>>,
    }

    fn strongconnect(state: &mut TarjanState, graph: &FileGraph, v: FileId) {
        state.index.insert(v, state.index_counter);
        state.lowlink.insert(v, state.index_counter);
        state.index_counter += 1;
        state.stack.push(v);
        state.on_stack.insert(v, true);

        // Visit successors (files this file imports)
        for w in graph.direct_imports(v) {
            if !state.index.contains_key(&w) {
                // w has not been visited
                strongconnect(state, graph, w);
                let w_low = state.lowlink[&w];
                let v_low = state.lowlink[&v];
                if w_low < v_low {
                    state.lowlink.insert(v, w_low);
                }
            } else if *state.on_stack.get(&w).unwrap_or(&false) {
                // w is on the stack and hence in the current SCC
                let w_idx = state.index[&w];
                let v_low = state.lowlink[&v];
                if w_idx < v_low {
                    state.lowlink.insert(v, w_idx);
                }
            }
        }

        // If v is a root node, pop the SCC
        if state.lowlink[&v] == state.index[&v] {
            let mut scc = Vec::new();
            loop {
                let w = state.stack.pop().unwrap();
                state.on_stack.insert(w, false);
                scc.push(w);
                if w == v {
                    break;
                }
            }
            state.result.push(scc);
        }
    }

    let mut state = TarjanState {
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashMap::new(),
        index: HashMap::new(),
        lowlink: HashMap::new(),
        result: Vec::new(),
    };

    for &node in nodes {
        if !state.index.contains_key(&node) {
            strongconnect(&mut state, graph, node);
        }
    }

    state.result
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
            is_mod_declaration: false,
            line: 1,
        }
    }

    #[test]
    fn test_no_cycles() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 3));

        let result = detect_cycles(&graph);
        assert!(result.cycles.is_empty());
        assert_eq!(result.summary.cycle_count, 0);
    }

    #[test]
    fn test_simple_two_node_cycle() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 1)); // cycle

        let result = detect_cycles(&graph);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].length, 2);
    }

    #[test]
    fn test_three_node_cycle() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 3));
        graph.add_import(make_edge(3, 1)); // cycle: 1 -> 2 -> 3 -> 1

        let result = detect_cycles(&graph);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].length, 3);
    }

    #[test]
    fn test_multiple_disjoint_cycles() {
        let mut graph = FileGraph::new();
        // Cycle 1: a <-> b
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 1));

        // Cycle 2: c <-> d
        graph.add_file(make_file(3, "c.ts"));
        graph.add_file(make_file(4, "d.ts"));
        graph.add_import(make_edge(3, 4));
        graph.add_import(make_edge(4, 3));

        let result = detect_cycles(&graph);
        assert_eq!(result.cycles.len(), 2);
        assert_eq!(result.summary.files_in_cycles, 4);
    }

    #[test]
    fn test_cycles_sorted_by_length() {
        let mut graph = FileGraph::new();
        // 3-node cycle: a -> b -> c -> a
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 3));
        graph.add_import(make_edge(3, 1));

        // 2-node cycle: d <-> e
        graph.add_file(make_file(4, "d.ts"));
        graph.add_file(make_file(5, "e.ts"));
        graph.add_import(make_edge(4, 5));
        graph.add_import(make_edge(5, 4));

        let result = detect_cycles(&graph);
        assert_eq!(result.cycles.len(), 2);
        assert!(result.cycles[0].length <= result.cycles[1].length);
    }

    #[test]
    fn test_empty_graph() {
        let graph = FileGraph::new();
        let result = detect_cycles(&graph);
        assert!(result.cycles.is_empty());
        assert_eq!(result.summary.cycle_count, 0);
        assert_eq!(result.summary.total_files, 0);
    }

    #[test]
    fn test_acyclic_diamond_dependency() {
        // Diamond: a -> b, a -> c, b -> d, c -> d (no cycle)
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_file(make_file(4, "d.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(1, 3));
        graph.add_import(make_edge(2, 4));
        graph.add_import(make_edge(3, 4));

        let result = detect_cycles(&graph);
        assert!(result.cycles.is_empty());
    }

    #[test]
    fn test_summary_fields() {
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts")); // not in a cycle
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 1));
        graph.add_import(make_edge(1, 3));

        let result = detect_cycles(&graph);
        assert_eq!(result.summary.total_files, 3);
        assert_eq!(result.summary.files_in_cycles, 2);
        assert_eq!(result.summary.cycle_count, 1);
        assert_eq!(result.summary.shortest_cycle, 2);
        assert_eq!(result.summary.longest_cycle, 2);
    }

    #[test]
    fn test_self_import_not_reported_as_cycle() {
        // A file importing itself is a degenerate case.
        // Tarjan's SCC returns it as a single-node SCC, which is filtered
        // since we only report SCCs with > 1 node. Verify this behavior.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_import(make_edge(1, 1)); // self-import

        let result = detect_cycles(&graph);
        assert!(
            result.cycles.is_empty(),
            "self-import should not be reported as a cycle"
        );
        assert_eq!(result.summary.files_in_cycles, 0);
    }

    #[test]
    fn test_self_loop_with_real_cycle() {
        // Verify that a self-loop on one node does not interfere with
        // detection of a real multi-node cycle in the same graph.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));

        // Self-loop on a
        graph.add_import(make_edge(1, 1));
        // Real cycle: b <-> c
        graph.add_import(make_edge(2, 3));
        graph.add_import(make_edge(3, 2));
        // Connect a to the cycle
        graph.add_import(make_edge(1, 2));

        let result = detect_cycles(&graph);
        assert_eq!(
            result.cycles.len(),
            1,
            "should find exactly the b<->c cycle, not the self-loop"
        );
        assert_eq!(result.cycles[0].length, 2);
        assert_eq!(result.summary.files_in_cycles, 2);
    }

    #[test]
    fn test_cycle_with_tail_node() {
        // a -> b -> c -> a is a cycle; b -> d is a tail.
        // Only {a, b, c} should be in the cycle, not d.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "a.ts"));
        graph.add_file(make_file(2, "b.ts"));
        graph.add_file(make_file(3, "c.ts"));
        graph.add_file(make_file(4, "d.ts"));
        graph.add_import(make_edge(1, 2));
        graph.add_import(make_edge(2, 3));
        graph.add_import(make_edge(3, 1)); // cycle: a -> b -> c -> a
        graph.add_import(make_edge(2, 4)); // tail: b -> d

        let result = detect_cycles(&graph);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].length, 3);
        assert_eq!(result.summary.files_in_cycles, 3);
        // d should not be in any cycle
        let all_cycle_ids: Vec<FileId> = result.cycles[0].files.iter().map(|f| f.file_id).collect();
        assert!(
            !all_cycle_ids.contains(&FileId(4)),
            "tail node d should not be included in the cycle"
        );
    }

    #[test]
    fn test_mod_declaration_edges_excluded_from_cycles() {
        // Simulate mod.rs barrel file pattern:
        // mod.rs --mod--> a.rs, mod.rs --mod--> b.rs
        // a.rs --use--> b.rs, b.rs --use--> a.rs (real cycle)
        // Without filtering, mod.rs would be in the SCC with a and b.
        let mut graph = FileGraph::new();
        graph.add_file(make_file(1, "src/mod.rs"));
        graph.add_file(make_file(2, "src/a.rs"));
        graph.add_file(make_file(3, "src/b.rs"));

        // mod declarations (should be excluded)
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(2),
            imported_names: vec!["a".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 1,
        });
        graph.add_import(FileImport {
            from: FileId(1),
            to: FileId(3),
            imported_names: vec!["b".to_string()],
            is_type_only: false,
            is_mod_declaration: true,
            line: 2,
        });

        // Real cycle: a <-> b
        graph.add_import(make_edge(2, 3)); // a -> b
        graph.add_import(make_edge(3, 2)); // b -> a

        // With mod edges: mod.rs is in the SCC (false positive)
        let result_with_mod = detect_cycles(&graph);
        let scc_sizes: Vec<usize> = result_with_mod.cycles.iter().map(|c| c.length).collect();
        assert!(
            scc_sizes.iter().any(|&s| s >= 2),
            "should find at least one cycle"
        );

        // Without mod edges: only a <-> b cycle
        let filtered = graph.without_mod_declaration_edges();
        let result_without_mod = detect_cycles(&filtered);
        assert_eq!(
            result_without_mod.cycles.len(),
            1,
            "should find exactly one cycle"
        );
        assert_eq!(
            result_without_mod.cycles[0].length, 2,
            "cycle should be 2 files (a <-> b)"
        );
        let cycle_ids: Vec<FileId> = result_without_mod.cycles[0]
            .files
            .iter()
            .map(|f| f.file_id)
            .collect();
        assert!(
            !cycle_ids.contains(&FileId(1)),
            "mod.rs should NOT be in the cycle"
        );
    }
}
