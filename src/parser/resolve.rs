use std::collections::HashMap;

use crate::model::{Reference, Symbol, SymbolId};

/// Resolve placeholder reference targets to actual symbols defined in the same file.
///
/// References with placeholder targets (`id >= u64::MAX - 1_000_000`) are matched
/// against file-local symbols by name. When a name is unique, it resolves directly.
/// When ambiguous (multiple symbols share a name), scoped resolution uses the
/// source symbol's parent to disambiguate.
pub fn resolve_intra_file_refs(
    symbols: &[Symbol],
    references: &mut [Reference],
    ref_target_names: &[String],
) {
    // Build name -> symbol ID lookup. If a name maps to multiple symbols, mark as ambiguous.
    let mut name_to_id: HashMap<&str, Option<SymbolId>> = HashMap::new();
    for symbol in symbols {
        match name_to_id.get(symbol.name.as_str()) {
            None => {
                name_to_id.insert(&symbol.name, Some(symbol.id));
            }
            Some(Some(_)) => {
                // Found duplicate: mark as ambiguous
                name_to_id.insert(&symbol.name, None);
            }
            Some(None) => {}
        }
    }

    // Build scoped lookup: (name, parent) -> Vec<SymbolId> for ambiguous names
    let mut scoped: HashMap<(&str, Option<SymbolId>), Vec<SymbolId>> = HashMap::new();
    for symbol in symbols {
        if name_to_id.get(symbol.name.as_str()) == Some(&None) {
            scoped
                .entry((&symbol.name, symbol.parent))
                .or_default()
                .push(symbol.id);
        }
    }

    // Build source -> parent lookup
    let sym_parent: HashMap<SymbolId, Option<SymbolId>> = symbols
        .iter()
        .map(|s| (s.id, s.parent))
        .collect();

    // Resolve references with placeholder targets
    for (i, reference) in references.iter_mut().enumerate() {
        if reference.target.0 >= u64::MAX - 1_000_000 {
            if let Some(target_name) = ref_target_names.get(i) {
                match name_to_id.get(target_name.as_str()) {
                    Some(Some(resolved_id)) => {
                        // Unique resolution: directly set target
                        reference.target = *resolved_id;
                    }
                    Some(None) => {
                        // Ambiguous: try scoped resolution via parent
                        let source_parent = sym_parent
                            .get(&reference.source)
                            .copied()
                            .flatten();
                        if let Some(candidates) =
                            scoped.get(&(target_name.as_str(), source_parent))
                        {
                            if candidates.len() == 1 {
                                // Scoped resolution succeeds: exactly one candidate in same parent
                                reference.target = candidates[0];
                            }
                        }
                    }
                    None => {} // Cannot resolve: leave as placeholder
                }
            }
        }
    }
}
