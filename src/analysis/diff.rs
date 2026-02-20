use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::Database;

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
}

/// Result of comparing two index snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub changes: Vec<ExportChange>,
    pub summary: DiffSummary,
}

/// Compare two database snapshots and produce a structural diff.
///
/// `db_before` is the baseline (e.g., the old version).
/// `db_after` is the current state (e.g., after code changes).
pub fn compare_snapshots(db_before: &Database, db_after: &Database) -> anyhow::Result<DiffResult> {
    let files_before = db_before.all_files()?;
    let files_after = db_after.all_files()?;

    let paths_before: HashMap<PathBuf, _> = files_before
        .iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    let paths_after: HashMap<PathBuf, _> = files_after
        .iter()
        .map(|f| (f.path.clone(), f))
        .collect();

    let all_paths: HashSet<&PathBuf> = paths_before.keys().chain(paths_after.keys()).collect();

    let mut changes = Vec::new();
    let mut files_added = 0usize;
    let mut files_removed = 0usize;
    let mut files_changed = 0usize;
    let mut files_unchanged = 0usize;

    for path in all_paths {
        match (paths_before.get(path), paths_after.get(path)) {
            (None, Some(after_file)) => {
                // File added
                files_added += 1;
                let exports = db_after.get_exports_by_file(after_file.id)?;
                for export in &exports {
                    changes.push(ExportChange {
                        kind: ChangeKind::Expanding,
                        file_path: path.clone(),
                        export_name: export.exported_name.clone(),
                        detail: "new file".to_string(),
                    });
                }
            }
            (Some(before_file), None) => {
                // File removed
                files_removed += 1;
                let exports = db_before.get_exports_by_file(before_file.id)?;
                for export in &exports {
                    changes.push(ExportChange {
                        kind: ChangeKind::Breaking,
                        file_path: path.clone(),
                        export_name: export.exported_name.clone(),
                        detail: "file removed".to_string(),
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
                    changes.push(ExportChange {
                        kind: ChangeKind::Breaking,
                        file_path: path.clone(),
                        export_name: name.clone(),
                        detail: "export removed".to_string(),
                    });
                    file_changed = true;
                }

                // Added exports (in after but not before)
                for name in names_after.difference(&names_before) {
                    changes.push(ExportChange {
                        kind: ChangeKind::Expanding,
                        file_path: path.clone(),
                        export_name: name.clone(),
                        detail: "export added".to_string(),
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

    // Sort for deterministic output
    changes.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.export_name.cmp(&b.export_name))
    });

    let breaking_changes = changes.iter().filter(|c| c.kind == ChangeKind::Breaking).count();
    let expanding_changes = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Expanding)
        .count();
    let restructuring_changes = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Restructuring)
        .count();

    Ok(DiffResult {
        changes,
        summary: DiffSummary {
            files_added,
            files_removed,
            files_changed,
            files_unchanged,
            breaking_changes,
            expanding_changes,
            restructuring_changes,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ExportRecord, FileId, FileRecord, Language, LineSpan, Position, Span, Symbol, SymbolId,
        SymbolKind, Visibility,
    };

    fn make_db_with_file(
        file_id: u64,
        path: &str,
        export_names: &[&str],
    ) -> Database {
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
                    end: Position { line: 1, column: 10 },
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
            };
            db.insert_export(&export).unwrap();
        }

        db
    }

    #[test]
    fn test_no_changes() {
        let before = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);
        let after = make_db_with_file(1, "src/utils.ts", &["foo", "bar"]);

        let result = compare_snapshots(&before, &after).unwrap();
        assert!(result.changes.is_empty());
        assert_eq!(result.summary.files_unchanged, 1);
        assert_eq!(result.summary.breaking_changes, 0);
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
}
