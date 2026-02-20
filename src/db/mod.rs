use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::{
    ExportRecord, FileId, FileRecord, ImportRecord, Language, LineSpan, Position, RefKind,
    Reference, ReferenceId, Span, Symbol, SymbolId, SymbolKind, Visibility,
};

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("failed to open database")?;
        let db = Self { conn };
        db.initialize()?;
        Ok(db)
    }

    /// Create an in-memory database (for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory database")?;
        let db = Self { conn };
        db.initialize()?;
        Ok(db)
    }

    fn initialize(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime INTEGER NOT NULL,
                language TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                qualified_name TEXT NOT NULL,
                kind TEXT NOT NULL,
                span_start INTEGER NOT NULL,
                span_end INTEGER NOT NULL,
                line_start INTEGER NOT NULL,
                col_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                col_end INTEGER NOT NULL,
                parent_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
                visibility TEXT NOT NULL,
                signature TEXT
            );

            CREATE TABLE IF NOT EXISTS refs (
                id INTEGER PRIMARY KEY,
                source_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                target_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                span_start INTEGER NOT NULL,
                span_end INTEGER NOT NULL,
                line_start INTEGER NOT NULL,
                col_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                col_end INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                source_path TEXT NOT NULL,
                imported_name TEXT NOT NULL,
                local_name TEXT NOT NULL,
                span_start INTEGER NOT NULL,
                span_end INTEGER NOT NULL,
                line_start INTEGER NOT NULL,
                col_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                col_end INTEGER NOT NULL,
                is_default INTEGER NOT NULL DEFAULT 0,
                is_namespace INTEGER NOT NULL DEFAULT 0,
                is_type_only INTEGER NOT NULL DEFAULT 0,
                is_side_effect INTEGER NOT NULL DEFAULT 0,
                is_dynamic INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS exports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                exported_name TEXT NOT NULL,
                is_default INTEGER NOT NULL DEFAULT 0,
                is_reexport INTEGER NOT NULL DEFAULT 0,
                is_type_only INTEGER NOT NULL DEFAULT 0,
                source_path TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_refs_source ON refs(source_id);
            CREATE INDEX IF NOT EXISTS idx_refs_target ON refs(target_id);
            CREATE INDEX IF NOT EXISTS idx_imports_file ON imports(file_id);
            CREATE INDEX IF NOT EXISTS idx_exports_file ON exports(file_id);
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            ",
            )
            .context("failed to initialize database schema")?;

        Ok(())
    }

    // ---- File operations ----

    pub fn upsert_file(&self, file: &FileRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO files (id, path, mtime, language)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET path=?2, mtime=?3, language=?4",
                params![
                    file.id.0,
                    file.path.to_string_lossy().to_string(),
                    file.mtime,
                    file.language.as_str(),
                ],
            )
            .context("failed to upsert file")?;
        Ok(())
    }

    pub fn get_file(&self, id: FileId) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT id, path, mtime, language FROM files WHERE id = ?1",
                params![id.0],
                |row| {
                    let lang_str: String = row.get(3)?;
                    Ok(FileRecord {
                        id: FileId(row.get(0)?),
                        path: std::path::PathBuf::from(row.get::<_, String>(1)?),
                        mtime: row.get(2)?,
                        language: Language::from_stored_str(&lang_str)
                            .unwrap_or(Language::TypeScript),
                    })
                },
            )
            .optional()
            .context("failed to get file")
    }

    pub fn get_file_by_path(&self, path: &str) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT id, path, mtime, language FROM files WHERE path = ?1",
                params![path],
                |row| {
                    let lang_str: String = row.get(3)?;
                    Ok(FileRecord {
                        id: FileId(row.get(0)?),
                        path: std::path::PathBuf::from(row.get::<_, String>(1)?),
                        mtime: row.get(2)?,
                        language: Language::from_stored_str(&lang_str)
                            .unwrap_or(Language::TypeScript),
                    })
                },
            )
            .optional()
            .context("failed to get file by path")
    }

    pub fn all_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, mtime, language FROM files")?;
        let files = stmt
            .query_map([], |row| {
                let lang_str: String = row.get(3)?;
                Ok(FileRecord {
                    id: FileId(row.get(0)?),
                    path: std::path::PathBuf::from(row.get::<_, String>(1)?),
                    mtime: row.get(2)?,
                    language: Language::from_stored_str(&lang_str)
                        .unwrap_or(Language::TypeScript),
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list files")?;
        Ok(files)
    }

    pub fn delete_file(&self, id: FileId) -> Result<()> {
        // CASCADE will clean up symbols, refs, imports, exports
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![id.0])
            .context("failed to delete file")?;
        Ok(())
    }

    // ---- Symbol operations ----

    pub fn insert_symbol(&self, symbol: &Symbol) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO symbols (id, file_id, name, qualified_name, kind,
                 span_start, span_end, line_start, col_start, line_end, col_end,
                 parent_id, visibility, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    symbol.id.0,
                    symbol.file.0,
                    symbol.name,
                    symbol.qualified_name,
                    symbol.kind.as_str(),
                    symbol.span.start,
                    symbol.span.end,
                    symbol.line_span.start.line,
                    symbol.line_span.start.column,
                    symbol.line_span.end.line,
                    symbol.line_span.end.column,
                    symbol.parent.map(|p| p.0),
                    symbol.visibility.as_str(),
                    symbol.signature,
                ],
            )
            .context("failed to insert symbol")?;
        Ok(())
    }

    pub fn get_symbols_by_file(&self, file_id: FileId) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, name, qualified_name, kind,
                    span_start, span_end, line_start, col_start, line_end, col_end,
                    parent_id, visibility, signature
             FROM symbols WHERE file_id = ?1",
        )?;

        let symbols = stmt
            .query_map(params![file_id.0], row_to_symbol)?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get symbols by file")?;
        Ok(symbols)
    }

    pub fn find_symbols_by_name(&self, name: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, name, qualified_name, kind,
                    span_start, span_end, line_start, col_start, line_end, col_end,
                    parent_id, visibility, signature
             FROM symbols WHERE name = ?1",
        )?;

        let symbols = stmt
            .query_map(params![name], row_to_symbol)?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to find symbols by name")?;
        Ok(symbols)
    }

    pub fn find_symbols_by_kind(&self, kind: SymbolKind) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, name, qualified_name, kind,
                    span_start, span_end, line_start, col_start, line_end, col_end,
                    parent_id, visibility, signature
             FROM symbols WHERE kind = ?1",
        )?;

        let symbols = stmt
            .query_map(params![kind.as_str()], row_to_symbol)?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to find symbols by kind")?;
        Ok(symbols)
    }

    pub fn all_symbols(&self) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, name, qualified_name, kind,
                    span_start, span_end, line_start, col_start, line_end, col_end,
                    parent_id, visibility, signature
             FROM symbols",
        )?;

        let symbols = stmt
            .query_map([], row_to_symbol)?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get all symbols")?;
        Ok(symbols)
    }

    // ---- Reference operations ----

    pub fn insert_reference(&self, reference: &Reference) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO refs (id, source_id, target_id, kind, file_id,
                 span_start, span_end, line_start, col_start, line_end, col_end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    reference.id.0,
                    reference.source.0,
                    reference.target.0,
                    reference.kind.as_str(),
                    reference.file.0,
                    reference.span.start,
                    reference.span.end,
                    reference.line_span.start.line,
                    reference.line_span.start.column,
                    reference.line_span.end.line,
                    reference.line_span.end.column,
                ],
            )
            .context("failed to insert reference")?;
        Ok(())
    }

    pub fn all_references(&self) -> Result<Vec<Reference>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_id, target_id, kind, file_id,
                    span_start, span_end, line_start, col_start, line_end, col_end
             FROM refs",
        )?;

        let refs = stmt
            .query_map([], |row| {
                let kind_str: String = row.get(3)?;
                Ok(Reference {
                    id: ReferenceId(row.get(0)?),
                    source: SymbolId(row.get(1)?),
                    target: SymbolId(row.get(2)?),
                    kind: match kind_str.as_str() {
                        "call" => RefKind::Call,
                        "type_usage" => RefKind::TypeUsage,
                        "import" => RefKind::Import,
                        "export" => RefKind::Export,
                        "inheritance" => RefKind::Inheritance,
                        "field_access" => RefKind::FieldAccess,
                        "assignment" => RefKind::Assignment,
                        _ => RefKind::Call,
                    },
                    file: FileId(row.get(4)?),
                    span: Span {
                        start: row.get(5)?,
                        end: row.get(6)?,
                    },
                    line_span: LineSpan {
                        start: Position {
                            line: row.get(7)?,
                            column: row.get(8)?,
                        },
                        end: Position {
                            line: row.get(9)?,
                            column: row.get(10)?,
                        },
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get all references")?;
        Ok(refs)
    }

    // ---- Import operations ----

    pub fn insert_import(&self, import: &ImportRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO imports (file_id, source_path, imported_name, local_name,
                 span_start, span_end, line_start, col_start, line_end, col_end,
                 is_default, is_namespace, is_type_only, is_side_effect, is_dynamic)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    import.file.0,
                    import.source_path,
                    import.imported_name,
                    import.local_name,
                    import.span.start,
                    import.span.end,
                    import.line_span.start.line,
                    import.line_span.start.column,
                    import.line_span.end.line,
                    import.line_span.end.column,
                    import.is_default as i32,
                    import.is_namespace as i32,
                    import.is_type_only as i32,
                    import.is_side_effect as i32,
                    import.is_dynamic as i32,
                ],
            )
            .context("failed to insert import")?;
        Ok(())
    }

    pub fn get_imports_by_file(&self, file_id: FileId) -> Result<Vec<ImportRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, source_path, imported_name, local_name,
                    span_start, span_end, line_start, col_start, line_end, col_end,
                    is_default, is_namespace, is_type_only, is_side_effect, is_dynamic
             FROM imports WHERE file_id = ?1",
        )?;

        let imports = stmt
            .query_map(params![file_id.0], |row| {
                Ok(ImportRecord {
                    file: FileId(row.get(0)?),
                    source_path: row.get(1)?,
                    imported_name: row.get(2)?,
                    local_name: row.get(3)?,
                    span: Span {
                        start: row.get(4)?,
                        end: row.get(5)?,
                    },
                    line_span: LineSpan {
                        start: Position {
                            line: row.get(6)?,
                            column: row.get(7)?,
                        },
                        end: Position {
                            line: row.get(8)?,
                            column: row.get(9)?,
                        },
                    },
                    is_default: row.get::<_, i32>(10)? != 0,
                    is_namespace: row.get::<_, i32>(11)? != 0,
                    is_type_only: row.get::<_, i32>(12)? != 0,
                    is_side_effect: row.get::<_, i32>(13)? != 0,
                    is_dynamic: row.get::<_, i32>(14)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get imports by file")?;
        Ok(imports)
    }

    // ---- Export operations ----

    pub fn insert_export(&self, export: &ExportRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO exports (file_id, symbol_id, exported_name,
                 is_default, is_reexport, is_type_only, source_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    export.file.0,
                    export.symbol.0,
                    export.exported_name,
                    export.is_default as i32,
                    export.is_reexport as i32,
                    export.is_type_only as i32,
                    export.source_path,
                ],
            )
            .context("failed to insert export")?;
        Ok(())
    }

    pub fn get_exports_by_file(&self, file_id: FileId) -> Result<Vec<ExportRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, symbol_id, exported_name, is_default, is_reexport, is_type_only, source_path
             FROM exports WHERE file_id = ?1",
        )?;

        let exports = stmt
            .query_map(params![file_id.0], |row| {
                Ok(ExportRecord {
                    file: FileId(row.get(0)?),
                    symbol: SymbolId(row.get(1)?),
                    exported_name: row.get(2)?,
                    is_default: row.get::<_, i32>(3)? != 0,
                    is_reexport: row.get::<_, i32>(4)? != 0,
                    is_type_only: row.get::<_, i32>(5)? != 0,
                    source_path: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get exports by file")?;
        Ok(exports)
    }

    // ---- Batch operations for indexing ----

    pub fn begin_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN TRANSACTION")
            .context("failed to begin transaction")?;
        Ok(())
    }

    pub fn commit_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("COMMIT")
            .context("failed to commit transaction")?;
        Ok(())
    }

    pub fn rollback_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("ROLLBACK")
            .context("failed to rollback transaction")?;
        Ok(())
    }

    /// Delete all data for a file (symbols, refs, imports, exports via CASCADE).
    pub fn clear_file_data(&self, file_id: FileId) -> Result<()> {
        // Due to CASCADE, deleting from files would remove everything.
        // But we want to keep the file record and just clear its symbols.
        // So we delete symbols (which cascades refs), imports, and exports directly.
        self.conn
            .execute("DELETE FROM exports WHERE file_id = ?1", params![file_id.0])?;
        self.conn
            .execute("DELETE FROM imports WHERE file_id = ?1", params![file_id.0])?;
        self.conn
            .execute("DELETE FROM refs WHERE file_id = ?1", params![file_id.0])?;
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id.0])?;
        Ok(())
    }
}

fn row_to_symbol(row: &rusqlite::Row) -> rusqlite::Result<Symbol> {
    let kind_str: String = row.get(4)?;
    let vis_str: String = row.get(12)?;
    Ok(Symbol {
        id: SymbolId(row.get(0)?),
        file: FileId(row.get(1)?),
        name: row.get(2)?,
        qualified_name: row.get(3)?,
        kind: kind_str.parse().unwrap_or(SymbolKind::Variable),
        span: Span {
            start: row.get(5)?,
            end: row.get(6)?,
        },
        line_span: LineSpan {
            start: Position {
                line: row.get(7)?,
                column: row.get(8)?,
            },
            end: Position {
                line: row.get(9)?,
                column: row.get(10)?,
            },
        },
        parent: row.get::<_, Option<u64>>(11)?.map(SymbolId),
        visibility: match vis_str.as_str() {
            "public" => Visibility::Public,
            "private" => Visibility::Private,
            "protected" => Visibility::Protected,
            _ => Visibility::Private,
        },
        signature: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_db() -> Database {
        Database::in_memory().unwrap()
    }

    fn sample_file() -> FileRecord {
        FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/index.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        }
    }

    fn sample_symbol(id: u64, name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
            file: FileId(1),
            span: Span { start: 0, end: 50 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position { line: 3, column: 1 },
            },
            parent: None,
            visibility: Visibility::Public,
            signature: Some(format!("function {}()", name)),
        }
    }

    #[test]
    fn test_file_crud() {
        let db = test_db();
        let file = sample_file();

        db.upsert_file(&file).unwrap();

        let retrieved = db.get_file(FileId(1)).unwrap().unwrap();
        assert_eq!(retrieved.path, PathBuf::from("src/index.ts"));
        assert_eq!(retrieved.mtime, 1000);

        // Update mtime
        let updated = FileRecord {
            mtime: 2000,
            ..file
        };
        db.upsert_file(&updated).unwrap();
        let retrieved = db.get_file(FileId(1)).unwrap().unwrap();
        assert_eq!(retrieved.mtime, 2000);

        // List all files
        let all = db.all_files().unwrap();
        assert_eq!(all.len(), 1);

        // Delete
        db.delete_file(FileId(1)).unwrap();
        assert!(db.get_file(FileId(1)).unwrap().is_none());
    }

    #[test]
    fn test_symbol_insert_and_query() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();

        let by_file = db.get_symbols_by_file(FileId(1)).unwrap();
        assert_eq!(by_file.len(), 1);
        assert_eq!(by_file[0].name, "greet");
        assert_eq!(by_file[0].kind, SymbolKind::Function);
        assert_eq!(by_file[0].signature.as_deref(), Some("function greet()"));

        let by_name = db.find_symbols_by_name("greet").unwrap();
        assert_eq!(by_name.len(), 1);

        let by_kind = db.find_symbols_by_kind(SymbolKind::Function).unwrap();
        assert_eq!(by_kind.len(), 1);
    }

    #[test]
    fn test_reference_insert_and_query() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym1 = sample_symbol(1, "main", SymbolKind::Function);
        let sym2 = sample_symbol(2, "helper", SymbolKind::Function);
        db.insert_symbol(&sym1).unwrap();
        db.insert_symbol(&sym2).unwrap();

        let reference = Reference {
            id: ReferenceId(1),
            source: SymbolId(1),
            target: SymbolId(2),
            kind: RefKind::Call,
            file: FileId(1),
            span: Span { start: 20, end: 30 },
            line_span: LineSpan {
                start: Position { line: 2, column: 4 },
                end: Position {
                    line: 2,
                    column: 14,
                },
            },
        };
        db.insert_reference(&reference).unwrap();

        let all_refs = db.all_references().unwrap();
        assert_eq!(all_refs.len(), 1);
        assert_eq!(all_refs[0].source, SymbolId(1));
        assert_eq!(all_refs[0].target, SymbolId(2));
        assert_eq!(all_refs[0].kind, RefKind::Call);
    }

    #[test]
    fn test_import_insert_and_query() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let import = ImportRecord {
            file: FileId(1),
            source_path: "./utils".to_string(),
            imported_name: "helper".to_string(),
            local_name: "helper".to_string(),
            span: Span { start: 0, end: 30 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position {
                    line: 1,
                    column: 30,
                },
            },
            is_default: false,
            is_namespace: false,
            is_type_only: false,
            is_side_effect: false,
            is_dynamic: false,
        };
        db.insert_import(&import).unwrap();

        let imports = db.get_imports_by_file(FileId(1)).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].source_path, "./utils");
        assert_eq!(imports[0].imported_name, "helper");
    }

    #[test]
    fn test_export_insert_and_query() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();

        let export = ExportRecord {
            file: FileId(1),
            symbol: SymbolId(1),
            exported_name: "greet".to_string(),
            is_default: false,
            is_reexport: false,
            is_type_only: false,
            source_path: None,
        };
        db.insert_export(&export).unwrap();

        let exports = db.get_exports_by_file(FileId(1)).unwrap();
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].exported_name, "greet");
    }

    #[test]
    fn test_cascade_delete_on_file_removal() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();

        let import = ImportRecord {
            file: FileId(1),
            source_path: "./utils".to_string(),
            imported_name: "helper".to_string(),
            local_name: "helper".to_string(),
            span: Span { start: 0, end: 30 },
            line_span: LineSpan {
                start: Position { line: 1, column: 0 },
                end: Position {
                    line: 1,
                    column: 30,
                },
            },
            is_default: false,
            is_namespace: false,
            is_type_only: false,
            is_side_effect: false,
            is_dynamic: false,
        };
        db.insert_import(&import).unwrap();

        // Delete file - should cascade to symbols and imports
        db.delete_file(FileId(1)).unwrap();

        assert!(db.get_symbols_by_file(FileId(1)).unwrap().is_empty());
        assert!(db.get_imports_by_file(FileId(1)).unwrap().is_empty());
    }

    #[test]
    fn test_clear_file_data() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();

        // Clear data but keep file record
        db.clear_file_data(FileId(1)).unwrap();

        assert!(db.get_symbols_by_file(FileId(1)).unwrap().is_empty());
        // File record should still exist
        assert!(db.get_file(FileId(1)).unwrap().is_some());
    }

    #[test]
    fn test_transaction_commit() {
        let db = test_db();
        db.begin_transaction().unwrap();
        db.upsert_file(&sample_file()).unwrap();
        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();
        db.commit_transaction().unwrap();

        assert_eq!(db.all_symbols().unwrap().len(), 1);
    }

    #[test]
    fn test_get_file_by_path() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let found = db.get_file_by_path("src/index.ts").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FileId(1));

        let not_found = db.get_file_by_path("src/nonexistent.ts").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_symbol_with_parent() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let class = sample_symbol(1, "MyClass", SymbolKind::Class);
        db.insert_symbol(&class).unwrap();

        let method = Symbol {
            id: SymbolId(2),
            name: "doStuff".to_string(),
            qualified_name: "MyClass::doStuff".to_string(),
            kind: SymbolKind::Method,
            file: FileId(1),
            span: Span { start: 20, end: 40 },
            line_span: LineSpan {
                start: Position { line: 2, column: 2 },
                end: Position { line: 4, column: 3 },
            },
            parent: Some(SymbolId(1)),
            visibility: Visibility::Public,
            signature: None,
        };
        db.insert_symbol(&method).unwrap();

        let symbols = db.get_symbols_by_file(FileId(1)).unwrap();
        let method_sym = symbols.iter().find(|s| s.name == "doStuff").unwrap();
        assert_eq!(method_sym.parent, Some(SymbolId(1)));
        assert_eq!(method_sym.qualified_name, "MyClass::doStuff");
    }

    // --- Additional edge case tests added by test-reviewer ---

    #[test]
    fn test_transaction_rollback() {
        let db = test_db();
        db.begin_transaction().unwrap();
        db.upsert_file(&sample_file()).unwrap();
        let sym = sample_symbol(1, "greet", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();
        db.rollback_transaction().unwrap();

        // After rollback, nothing should be persisted
        assert!(db.all_files().unwrap().is_empty());
        assert!(db.all_symbols().unwrap().is_empty());
    }

    #[test]
    fn test_multiple_files_queries_dont_leak() {
        let db = test_db();

        let file1 = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        let file2 = FileRecord {
            id: FileId(2),
            path: PathBuf::from("src/b.ts"),
            mtime: 1000,
            language: Language::JavaScript,
        };
        db.upsert_file(&file1).unwrap();
        db.upsert_file(&file2).unwrap();

        let sym1 = Symbol {
            file: FileId(1),
            ..sample_symbol(1, "foo", SymbolKind::Function)
        };
        let sym2 = Symbol {
            id: SymbolId(2),
            file: FileId(2),
            ..sample_symbol(2, "bar", SymbolKind::Class)
        };
        db.insert_symbol(&sym1).unwrap();
        db.insert_symbol(&sym2).unwrap();

        let file1_syms = db.get_symbols_by_file(FileId(1)).unwrap();
        assert_eq!(file1_syms.len(), 1);
        assert_eq!(file1_syms[0].name, "foo");

        let file2_syms = db.get_symbols_by_file(FileId(2)).unwrap();
        assert_eq!(file2_syms.len(), 1);
        assert_eq!(file2_syms[0].name, "bar");

        // all_symbols returns both
        assert_eq!(db.all_symbols().unwrap().len(), 2);
    }

    #[test]
    fn test_language_roundtrip_all_variants() {
        let db = test_db();

        let languages = vec![
            (FileId(1), "src/a.ts", Language::TypeScript),
            (FileId(2), "src/b.js", Language::JavaScript),
            (FileId(3), "src/c.py", Language::Python),
            (FileId(4), "src/d.rs", Language::Rust),
        ];

        for (id, path, lang) in &languages {
            db.upsert_file(&FileRecord {
                id: *id,
                path: PathBuf::from(path),
                mtime: 1000,
                language: *lang,
            })
            .unwrap();
        }

        for (id, _, expected_lang) in &languages {
            let file = db.get_file(*id).unwrap().unwrap();
            assert_eq!(
                file.language, *expected_lang,
                "language roundtrip failed for {:?}",
                expected_lang
            );
        }
    }

    #[test]
    fn test_symbol_replace_on_duplicate_id() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "original", SymbolKind::Function);
        db.insert_symbol(&sym).unwrap();

        // Insert with same ID but different name
        let updated = sample_symbol(1, "updated", SymbolKind::Class);
        db.insert_symbol(&updated).unwrap();

        let all = db.all_symbols().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "updated");
        assert_eq!(all[0].kind, SymbolKind::Class);
    }

    #[test]
    fn test_reexport_roundtrip() {
        let db = test_db();
        db.upsert_file(&sample_file()).unwrap();

        let sym = sample_symbol(1, "foo", SymbolKind::Export);
        db.insert_symbol(&sym).unwrap();

        let export = ExportRecord {
            file: FileId(1),
            symbol: SymbolId(1),
            exported_name: "bar".to_string(),
            is_default: false,
            is_reexport: true,
            is_type_only: false,
            source_path: Some("./other".to_string()),
        };
        db.insert_export(&export).unwrap();

        let exports = db.get_exports_by_file(FileId(1)).unwrap();
        assert_eq!(exports.len(), 1);
        assert!(exports[0].is_reexport);
        assert_eq!(exports[0].source_path.as_deref(), Some("./other"));
        assert_eq!(exports[0].exported_name, "bar");
    }

    #[test]
    fn test_empty_database_queries() {
        let db = test_db();

        assert!(db.all_files().unwrap().is_empty());
        assert!(db.all_symbols().unwrap().is_empty());
        assert!(db.all_references().unwrap().is_empty());
        assert!(db.get_file(FileId(999)).unwrap().is_none());
        assert!(db.get_file_by_path("nonexistent").unwrap().is_none());
        assert!(db.find_symbols_by_name("anything").unwrap().is_empty());
        assert!(db
            .find_symbols_by_kind(SymbolKind::Function)
            .unwrap()
            .is_empty());
        assert!(db.get_symbols_by_file(FileId(1)).unwrap().is_empty());
        assert!(db.get_imports_by_file(FileId(1)).unwrap().is_empty());
        assert!(db.get_exports_by_file(FileId(1)).unwrap().is_empty());
    }

    #[test]
    fn test_clear_file_data_does_not_affect_other_files() {
        let db = test_db();

        let file1 = FileRecord {
            id: FileId(1),
            path: PathBuf::from("src/a.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        let file2 = FileRecord {
            id: FileId(2),
            path: PathBuf::from("src/b.ts"),
            mtime: 1000,
            language: Language::TypeScript,
        };
        db.upsert_file(&file1).unwrap();
        db.upsert_file(&file2).unwrap();

        let sym1 = Symbol {
            file: FileId(1),
            ..sample_symbol(1, "foo", SymbolKind::Function)
        };
        let sym2 = Symbol {
            id: SymbolId(2),
            file: FileId(2),
            ..sample_symbol(2, "bar", SymbolKind::Function)
        };
        db.insert_symbol(&sym1).unwrap();
        db.insert_symbol(&sym2).unwrap();

        db.clear_file_data(FileId(1)).unwrap();

        // File 1 data cleared, file 2 untouched
        assert!(db.get_symbols_by_file(FileId(1)).unwrap().is_empty());
        assert_eq!(db.get_symbols_by_file(FileId(2)).unwrap().len(), 1);
        // File 1 record still exists
        assert!(db.get_file(FileId(1)).unwrap().is_some());
    }
}
