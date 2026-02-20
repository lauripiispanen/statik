use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    ExportRecord, FileId, FileRecord, ImportRecord, ParseResult, RefKind, Reference, Symbol,
    SymbolId, SymbolKind,
};

/// In-memory symbol graph for fast traversal and analysis queries.
#[derive(Debug, Default)]
pub struct SymbolGraph {
    pub symbols: HashMap<SymbolId, Symbol>,
    pub files: HashMap<FileId, FileRecord>,
    /// Outgoing references: source -> [(target, ref_kind, reference)]
    pub references_from: HashMap<SymbolId, Vec<(SymbolId, RefKind)>>,
    /// Incoming references: target -> [(source, ref_kind)]
    pub references_to: HashMap<SymbolId, Vec<(SymbolId, RefKind)>>,
    /// All symbols defined in a file
    pub file_symbols: HashMap<FileId, Vec<SymbolId>>,
    /// All references (full records)
    pub references: Vec<Reference>,
    /// Imports by file
    pub imports: HashMap<FileId, Vec<ImportRecord>>,
    /// Exports by file
    pub exports: HashMap<FileId, Vec<ExportRecord>>,
}

impl SymbolGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, file: FileRecord) {
        self.file_symbols.entry(file.id).or_default();
        self.files.insert(file.id, file);
    }

    pub fn add_parse_result(&mut self, result: ParseResult) {
        let file_id = result.file_id;

        for symbol in result.symbols {
            let symbol_id = symbol.id;
            self.file_symbols
                .entry(file_id)
                .or_default()
                .push(symbol_id);
            self.symbols.insert(symbol_id, symbol);
        }

        for reference in result.references {
            self.references_from
                .entry(reference.source)
                .or_default()
                .push((reference.target, reference.kind));
            self.references_to
                .entry(reference.target)
                .or_default()
                .push((reference.source, reference.kind));
            self.references.push(reference);
        }

        if !result.imports.is_empty() {
            self.imports
                .entry(file_id)
                .or_default()
                .extend(result.imports);
        }

        if !result.exports.is_empty() {
            self.exports
                .entry(file_id)
                .or_default()
                .extend(result.exports);
        }
    }

    pub fn remove_file(&mut self, file_id: FileId) {
        if let Some(symbol_ids) = self.file_symbols.remove(&file_id) {
            for sid in &symbol_ids {
                self.symbols.remove(sid);
                self.references_from.remove(sid);
                self.references_to.remove(sid);
            }
            // Clean up references that point to/from removed symbols
            let removed: HashSet<_> = symbol_ids.into_iter().collect();
            for refs in self.references_from.values_mut() {
                refs.retain(|(target, _)| !removed.contains(target));
            }
            for refs in self.references_to.values_mut() {
                refs.retain(|(source, _)| !removed.contains(source));
            }
            self.references
                .retain(|r| !removed.contains(&r.source) && !removed.contains(&r.target));
        }
        self.files.remove(&file_id);
        self.imports.remove(&file_id);
        self.exports.remove(&file_id);
    }

    pub fn get_symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(&id)
    }

    pub fn find_symbols_by_name(&self, name: &str) -> Vec<&Symbol> {
        self.symbols.values().filter(|s| s.name == name).collect()
    }

    pub fn find_symbols_by_kind(&self, kind: SymbolKind) -> Vec<&Symbol> {
        self.symbols.values().filter(|s| s.kind == kind).collect()
    }

    pub fn symbols_in_file(&self, file_id: FileId) -> Vec<&Symbol> {
        self.file_symbols
            .get(&file_id)
            .map(|ids| ids.iter().filter_map(|id| self.symbols.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get all symbols that reference the given symbol (who uses this?).
    pub fn callers(&self, target: SymbolId) -> Vec<(SymbolId, RefKind)> {
        self.references_to.get(&target).cloned().unwrap_or_default()
    }

    /// Get all symbols that the given symbol references (what does this use?).
    pub fn callees(&self, source: SymbolId) -> Vec<(SymbolId, RefKind)> {
        self.references_from
            .get(&source)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the transitive closure of dependents (who depends on this, transitively).
    pub fn transitive_dependents(&self, start: SymbolId) -> HashSet<SymbolId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            if let Some(refs) = self.references_to.get(&current) {
                for (source, _) in refs {
                    if visited.insert(*source) {
                        queue.push_back(*source);
                    }
                }
            }
        }

        visited.remove(&start);
        visited
    }

    /// Get the transitive closure of dependencies (what does this depend on, transitively).
    pub fn transitive_dependencies(&self, start: SymbolId) -> HashSet<SymbolId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            if let Some(refs) = self.references_from.get(&current) {
                for (target, _) in refs {
                    if visited.insert(*target) {
                        queue.push_back(*target);
                    }
                }
            }
        }

        visited.remove(&start);
        visited
    }

    /// Find all symbols reachable from a set of entry points.
    /// Only includes symbols that actually exist in the graph.
    pub fn reachable_from(&self, entry_points: &[SymbolId]) -> HashSet<SymbolId> {
        let mut visited = HashSet::new();
        let mut queue: VecDeque<SymbolId> = entry_points
            .iter()
            .copied()
            .filter(|id| self.symbols.contains_key(id))
            .collect();

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(refs) = self.references_from.get(&current) {
                for (target, _) in refs {
                    if !visited.contains(target) && self.symbols.contains_key(target) {
                        queue.push_back(*target);
                    }
                }
            }
        }

        visited
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn reference_count(&self) -> usize {
        self.references.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn make_symbol(id: u64, name: &str, kind: SymbolKind, file_id: u64) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
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
        }
    }

    fn make_reference(id: u64, source: u64, target: u64, kind: RefKind, file_id: u64) -> Reference {
        Reference {
            id: ReferenceId(id),
            source: SymbolId(source),
            target: SymbolId(target),
            kind,
            file: FileId(file_id),
            span: Span { start: 0, end: 5 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position { line: 1, column: 5 },
            },
        }
    }

    #[test]
    fn test_add_and_query_symbols() {
        let mut graph = SymbolGraph::new();
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "foo", SymbolKind::Function, 1),
                make_symbol(2, "Bar", SymbolKind::Class, 1),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };

        graph.add_parse_result(result);

        assert_eq!(graph.symbol_count(), 2);
        assert_eq!(graph.find_symbols_by_name("foo").len(), 1);
        assert_eq!(graph.find_symbols_by_kind(SymbolKind::Class).len(), 1);
        assert_eq!(graph.symbols_in_file(FileId(1)).len(), 2);
        assert_eq!(graph.symbols_in_file(FileId(99)).len(), 0);
    }

    #[test]
    fn test_references_and_callers() {
        let mut graph = SymbolGraph::new();
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "main", SymbolKind::Function, 1),
                make_symbol(2, "helper", SymbolKind::Function, 1),
                make_symbol(3, "util", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1), // main calls helper
                make_reference(2, 1, 3, RefKind::Call, 1), // main calls util
                make_reference(3, 2, 3, RefKind::Call, 1), // helper calls util
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };

        graph.add_parse_result(result);

        // main calls helper and util
        let callees = graph.callees(SymbolId(1));
        assert_eq!(callees.len(), 2);

        // util is called by main and helper
        let callers = graph.callers(SymbolId(3));
        assert_eq!(callers.len(), 2);

        // helper is called only by main
        let callers = graph.callers(SymbolId(2));
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].0, SymbolId(1));
    }

    #[test]
    fn test_transitive_dependents() {
        let mut graph = SymbolGraph::new();
        // a -> b -> c (a calls b, b calls c)
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "a", SymbolKind::Function, 1),
                make_symbol(2, "b", SymbolKind::Function, 1),
                make_symbol(3, "c", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 2, 3, RefKind::Call, 1),
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        // Transitive dependents of c: b and a (both transitively depend on c)
        let deps = graph.transitive_dependents(SymbolId(3));
        assert!(deps.contains(&SymbolId(1)));
        assert!(deps.contains(&SymbolId(2)));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_transitive_dependencies() {
        let mut graph = SymbolGraph::new();
        // a -> b -> c
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "a", SymbolKind::Function, 1),
                make_symbol(2, "b", SymbolKind::Function, 1),
                make_symbol(3, "c", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 2, 3, RefKind::Call, 1),
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        // Transitive dependencies of a: b and c
        let deps = graph.transitive_dependencies(SymbolId(1));
        assert!(deps.contains(&SymbolId(2)));
        assert!(deps.contains(&SymbolId(3)));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_reachable_from_entry_points() {
        let mut graph = SymbolGraph::new();
        // main -> a -> b, main -> c, d is isolated
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "main", SymbolKind::Function, 1),
                make_symbol(2, "a", SymbolKind::Function, 1),
                make_symbol(3, "b", SymbolKind::Function, 1),
                make_symbol(4, "c", SymbolKind::Function, 1),
                make_symbol(5, "d", SymbolKind::Function, 1), // isolated
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 2, 3, RefKind::Call, 1),
                make_reference(3, 1, 4, RefKind::Call, 1),
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        let reachable = graph.reachable_from(&[SymbolId(1)]);
        assert!(reachable.contains(&SymbolId(1))); // entry point itself
        assert!(reachable.contains(&SymbolId(2)));
        assert!(reachable.contains(&SymbolId(3)));
        assert!(reachable.contains(&SymbolId(4)));
        assert!(!reachable.contains(&SymbolId(5))); // d is dead code
    }

    #[test]
    fn test_remove_file_cleans_up_references() {
        let mut graph = SymbolGraph::new();
        // File 1: symbol A, File 2: symbol B. A calls B.
        let result1 = ParseResult {
            file_id: FileId(1),
            symbols: vec![make_symbol(1, "a", SymbolKind::Function, 1)],
            references: vec![make_reference(1, 1, 2, RefKind::Call, 1)],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        let result2 = ParseResult {
            file_id: FileId(2),
            symbols: vec![make_symbol(2, "b", SymbolKind::Function, 2)],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result1);
        graph.add_parse_result(result2);

        assert_eq!(graph.symbol_count(), 2);

        graph.remove_file(FileId(2));

        assert_eq!(graph.symbol_count(), 1);
        assert!(graph.get_symbol(SymbolId(2)).is_none());
        // References to removed symbol should be cleaned
        let callees = graph.callees(SymbolId(1));
        assert!(callees.is_empty());
    }

    #[test]
    fn test_circular_dependency_does_not_loop_forever() {
        let mut graph = SymbolGraph::new();
        // a -> b -> c -> a (cycle)
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "a", SymbolKind::Function, 1),
                make_symbol(2, "b", SymbolKind::Function, 1),
                make_symbol(3, "c", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 2, 3, RefKind::Call, 1),
                make_reference(3, 3, 1, RefKind::Call, 1), // cycle back to a
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        // transitive_dependents should terminate and find all nodes in the cycle
        let deps = graph.transitive_dependents(SymbolId(1));
        assert!(deps.contains(&SymbolId(2)));
        assert!(deps.contains(&SymbolId(3)));
        assert_eq!(deps.len(), 2);

        // transitive_dependencies should also terminate
        let deps = graph.transitive_dependencies(SymbolId(1));
        assert!(deps.contains(&SymbolId(2)));
        assert!(deps.contains(&SymbolId(3)));
        assert_eq!(deps.len(), 2);

        // reachable_from should find all nodes in the cycle
        let reachable = graph.reachable_from(&[SymbolId(1)]);
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_self_referencing_symbol() {
        let mut graph = SymbolGraph::new();
        // Recursive function: a calls itself
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![make_symbol(1, "factorial", SymbolKind::Function, 1)],
            references: vec![make_reference(1, 1, 1, RefKind::Call, 1)],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        let callers = graph.callers(SymbolId(1));
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].0, SymbolId(1));

        // Transitive dependents of a self-referencing symbol should NOT include itself
        let deps = graph.transitive_dependents(SymbolId(1));
        assert!(deps.is_empty());

        // Transitive dependencies of a self-referencing symbol should NOT include itself
        let deps = graph.transitive_dependencies(SymbolId(1));
        assert!(deps.is_empty());
    }

    #[test]
    fn test_empty_graph_queries() {
        let graph = SymbolGraph::new();

        assert_eq!(graph.symbol_count(), 0);
        assert_eq!(graph.file_count(), 0);
        assert_eq!(graph.reference_count(), 0);
        assert!(graph.find_symbols_by_name("anything").is_empty());
        assert!(graph.find_symbols_by_kind(SymbolKind::Function).is_empty());
        assert!(graph.symbols_in_file(FileId(1)).is_empty());
        assert!(graph.callers(SymbolId(1)).is_empty());
        assert!(graph.callees(SymbolId(1)).is_empty());
        assert!(graph.transitive_dependents(SymbolId(1)).is_empty());
        assert!(graph.transitive_dependencies(SymbolId(1)).is_empty());
        // reachable_from filters out entry points that don't exist in the graph
        assert!(graph.reachable_from(&[SymbolId(1)]).is_empty());
        assert!(graph.reachable_from(&[]).is_empty());
    }

    #[test]
    fn test_remove_source_file_cleans_incoming_references() {
        let mut graph = SymbolGraph::new();
        // File 1 has A (calls B), File 2 has B.
        // Remove file 1 (the caller). B's callers list should be empty.
        let result1 = ParseResult {
            file_id: FileId(1),
            symbols: vec![make_symbol(1, "a", SymbolKind::Function, 1)],
            references: vec![make_reference(1, 1, 2, RefKind::Call, 1)],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        let result2 = ParseResult {
            file_id: FileId(2),
            symbols: vec![make_symbol(2, "b", SymbolKind::Function, 2)],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result1);
        graph.add_parse_result(result2);

        assert_eq!(graph.callers(SymbolId(2)).len(), 1);

        graph.remove_file(FileId(1));

        assert_eq!(graph.symbol_count(), 1);
        assert!(graph.get_symbol(SymbolId(1)).is_none());
        // Incoming references to B from the removed file should be cleaned
        assert!(graph.callers(SymbolId(2)).is_empty());
    }

    #[test]
    fn test_multiple_entry_points_in_reachable() {
        let mut graph = SymbolGraph::new();
        // Two disjoint subgraphs: main1 -> a, main2 -> b, c is isolated
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "main1", SymbolKind::Function, 1),
                make_symbol(2, "a", SymbolKind::Function, 1),
                make_symbol(3, "main2", SymbolKind::Function, 1),
                make_symbol(4, "b", SymbolKind::Function, 1),
                make_symbol(5, "c", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 3, 4, RefKind::Call, 1),
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        let reachable = graph.reachable_from(&[SymbolId(1), SymbolId(3)]);
        assert_eq!(reachable.len(), 4); // main1, a, main2, b
        assert!(!reachable.contains(&SymbolId(5))); // c is still dead
    }

    #[test]
    fn test_multi_file_graph_merge() {
        let mut graph = SymbolGraph::new();

        // File 1
        let result1 = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "main", SymbolKind::Function, 1),
                make_symbol(2, "helper", SymbolKind::Function, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1),
                make_reference(2, 1, 3, RefKind::Call, 1), // cross-file: main calls util
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        // File 2
        let result2 = ParseResult {
            file_id: FileId(2),
            symbols: vec![
                make_symbol(3, "util", SymbolKind::Function, 2),
                make_symbol(4, "dead_fn", SymbolKind::Function, 2),
            ],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };

        graph.add_parse_result(result1);
        graph.add_parse_result(result2);

        assert_eq!(graph.symbol_count(), 4);
        assert_eq!(graph.file_count(), 0); // add_file was never called, only add_parse_result
        assert_eq!(graph.symbols_in_file(FileId(1)).len(), 2);
        assert_eq!(graph.symbols_in_file(FileId(2)).len(), 2);

        // Cross-file reference works: main calls util
        let callees = graph.callees(SymbolId(1));
        assert_eq!(callees.len(), 2);
        let callers = graph.callers(SymbolId(3));
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].0, SymbolId(1));

        // dead_fn has no callers
        assert!(graph.callers(SymbolId(4)).is_empty());

        // Reachability from main reaches helper and util, but not dead_fn
        let reachable = graph.reachable_from(&[SymbolId(1)]);
        assert!(reachable.contains(&SymbolId(1)));
        assert!(reachable.contains(&SymbolId(2)));
        assert!(reachable.contains(&SymbolId(3)));
        assert!(!reachable.contains(&SymbolId(4)));
    }

    #[test]
    fn test_different_ref_kinds_tracked_correctly() {
        let mut graph = SymbolGraph::new();
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![
                make_symbol(1, "main", SymbolKind::Function, 1),
                make_symbol(2, "UserClass", SymbolKind::Class, 1),
                make_symbol(3, "BaseClass", SymbolKind::Class, 1),
            ],
            references: vec![
                make_reference(1, 1, 2, RefKind::Call, 1), // main instantiates UserClass
                make_reference(2, 2, 3, RefKind::Inheritance, 1), // UserClass extends BaseClass
                make_reference(3, 1, 3, RefKind::TypeUsage, 1), // main uses BaseClass as type
            ],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        // Check that ref kinds are preserved
        let callees = graph.callees(SymbolId(1));
        assert_eq!(callees.len(), 2);
        let call_ref = callees.iter().find(|(id, _)| *id == SymbolId(2)).unwrap();
        assert_eq!(call_ref.1, RefKind::Call);
        let type_ref = callees.iter().find(|(id, _)| *id == SymbolId(3)).unwrap();
        assert_eq!(type_ref.1, RefKind::TypeUsage);

        // BaseClass has callers from both main (TypeUsage) and UserClass (Inheritance)
        let callers = graph.callers(SymbolId(3));
        assert_eq!(callers.len(), 2);
    }

    #[test]
    fn test_remove_nonexistent_file_is_safe() {
        let mut graph = SymbolGraph::new();
        let result = ParseResult {
            file_id: FileId(1),
            symbols: vec![make_symbol(1, "a", SymbolKind::Function, 1)],
            references: vec![],
            imports: vec![],
            exports: vec![],
            type_references: vec![],
            annotations: vec![],
        };
        graph.add_parse_result(result);

        // Removing a file that doesn't exist should not panic or corrupt the graph
        graph.remove_file(FileId(999));
        assert_eq!(graph.symbol_count(), 1);
    }
}
