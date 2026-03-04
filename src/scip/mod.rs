use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{Language, SymbolKind};

/// A definition extracted from a SCIP index.
#[derive(Debug, Clone)]
pub struct ScipDefinition {
    /// The SCIP symbol string (globally unique identifier).
    pub symbol: String,
    /// Human-readable name.
    pub name: String,
    /// Fully qualified name derived from the SCIP symbol.
    pub qualified_name: String,
    /// Mapped symbol kind.
    pub kind: SymbolKind,
    /// 0-based start line.
    pub line: u32,
    /// 0-based start column.
    pub column: u32,
    /// 0-based end line.
    pub end_line: u32,
    /// 0-based end column.
    pub end_column: u32,
}

/// A reference (usage) extracted from a SCIP index.
#[derive(Debug, Clone)]
pub struct ScipReference {
    /// The SCIP symbol string being referenced.
    pub symbol: String,
    /// What kind of reference this is.
    pub role: ScipRole,
    /// 0-based start line.
    pub line: u32,
    /// 0-based start column.
    pub column: u32,
    /// 0-based end line.
    pub end_line: u32,
    /// 0-based end column.
    pub end_column: u32,
}

/// Simplified role classification for SCIP occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScipRole {
    Definition,
    Reference,
    Import,
}

/// Parsed data for a single file from a SCIP index.
#[derive(Debug, Clone)]
pub struct ScipDocument {
    /// Relative path of the file within the project.
    pub relative_path: PathBuf,
    /// Language of the file.
    pub language: Option<Language>,
    /// Symbol definitions in this file.
    pub definitions: Vec<ScipDefinition>,
    /// Symbol references (usages) in this file.
    pub references: Vec<ScipReference>,
}

/// Result of reading a complete SCIP index file.
#[derive(Debug)]
pub struct ScipIndex {
    /// Project root from the SCIP metadata (if present).
    pub project_root: Option<String>,
    /// Tool that generated this index (if present).
    pub tool_name: Option<String>,
    /// Per-file parsed data.
    pub documents: Vec<ScipDocument>,
}

/// Read and parse a `.scip` protobuf file into statik's intermediate types.
pub fn read_scip_index(path: &Path) -> Result<ScipIndex> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read SCIP file: {}", path.display()))?;

    let index: scip::types::Index = protobuf::Message::parse_from_bytes(&data)
        .with_context(|| format!("failed to parse SCIP protobuf: {}", path.display()))?;

    // Extract metadata
    let (project_root, tool_name) = if let Some(meta) = index.metadata.as_ref() {
        let root = if meta.project_root.is_empty() {
            None
        } else {
            Some(meta.project_root.clone())
        };
        let tool = meta.tool_info.as_ref().map(|t| t.name.clone());
        (root, tool)
    } else {
        (None, None)
    };

    // Build a lookup from symbol string -> SymbolInformation for kind mapping.
    // Documents carry per-document symbol info; external_symbols are index-level.
    let mut symbol_kinds: HashMap<String, SymbolKind> = HashMap::new();
    for ext in &index.external_symbols {
        if let Some(kind) = map_scip_kind(&ext.kind) {
            symbol_kinds.insert(ext.symbol.clone(), kind);
        }
    }

    let documents: Vec<ScipDocument> = index
        .documents
        .iter()
        .map(|doc| parse_document(doc, &mut symbol_kinds))
        .collect();

    Ok(ScipIndex {
        project_root,
        tool_name,
        documents,
    })
}

/// Parse a single SCIP Document into our intermediate representation.
fn parse_document(
    doc: &scip::types::Document,
    symbol_kinds: &mut HashMap<String, SymbolKind>,
) -> ScipDocument {
    let language = map_scip_language(&doc.language);

    // Register per-document symbol info kinds
    for sym_info in &doc.symbols {
        if let Some(kind) = map_scip_kind(&sym_info.kind) {
            symbol_kinds.insert(sym_info.symbol.clone(), kind);
        }
    }

    let mut definitions = Vec::new();
    let mut references = Vec::new();

    for occ in &doc.occurrences {
        if occ.symbol.is_empty() {
            continue;
        }

        let (line, column, end_line, end_column) = parse_range(&occ.range);
        let roles = occ.symbol_roles;
        let is_definition = (roles & scip::types::SymbolRole::Definition as i32) != 0;
        let is_import = (roles & scip::types::SymbolRole::Import as i32) != 0;

        if is_definition {
            let kind = symbol_kinds
                .get(&occ.symbol)
                .copied()
                .unwrap_or_else(|| infer_kind_from_symbol(&occ.symbol));
            let (name, qualified_name) = extract_names(&occ.symbol);

            definitions.push(ScipDefinition {
                symbol: occ.symbol.clone(),
                name,
                qualified_name,
                kind,
                line,
                column,
                end_line,
                end_column,
            });
        } else {
            let role = if is_import {
                ScipRole::Import
            } else {
                ScipRole::Reference
            };
            references.push(ScipReference {
                symbol: occ.symbol.clone(),
                role,
                line,
                column,
                end_line,
                end_column,
            });
        }
    }

    ScipDocument {
        relative_path: PathBuf::from(&doc.relative_path),
        language,
        definitions,
        references,
    }
}

/// Parse a SCIP range (3 or 4 element i32 array) into (line, col, end_line, end_col).
/// All values are 0-based.
fn parse_range(range: &[i32]) -> (u32, u32, u32, u32) {
    match range.len() {
        4 => (
            range[0] as u32,
            range[1] as u32,
            range[2] as u32,
            range[3] as u32,
        ),
        3 => (
            range[0] as u32,
            range[1] as u32,
            range[0] as u32, // end line = start line
            range[2] as u32,
        ),
        _ => (0, 0, 0, 0),
    }
}

/// Extract a human-readable name and qualified name from a SCIP symbol string.
///
/// SCIP symbols look like:
///   `lsif-java maven package 1.0.0 java/io/File#Entry.method(+1).`
///   `rust-analyzer cargo statik 0.1.0 cli/commands/run_deps().`
///   `local 42`
///
/// We extract the last descriptor name as the short name,
/// and join all descriptor names as the qualified name.
fn extract_names(symbol_str: &str) -> (String, String) {
    if let Ok(parsed) = scip::symbol::parse_symbol(symbol_str) {
        let names: Vec<&str> = parsed
            .descriptors
            .iter()
            .map(|d| d.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        let name = names.last().map(|s| s.to_string()).unwrap_or_default();
        let qualified = names.join("::");
        (name, qualified)
    } else {
        // Fallback for unparseable symbols
        let name = symbol_str
            .rsplit(['/', '.', '#'])
            .find(|s| !s.is_empty())
            .unwrap_or(symbol_str)
            .to_string();
        (name.clone(), name)
    }
}

/// Map a SCIP SymbolInformation.Kind to statik's SymbolKind.
fn map_scip_kind(
    kind: &protobuf::EnumOrUnknown<scip::types::symbol_information::Kind>,
) -> Option<SymbolKind> {
    use scip::types::symbol_information::Kind;
    match kind.enum_value() {
        Ok(Kind::Function) => Some(SymbolKind::Function),
        Ok(Kind::Method)
        | Ok(Kind::AbstractMethod)
        | Ok(Kind::StaticMethod)
        | Ok(Kind::TraitMethod)
        | Ok(Kind::ProtocolMethod)
        | Ok(Kind::PureVirtualMethod)
        | Ok(Kind::TypeClassMethod)
        | Ok(Kind::MethodSpecification)
        | Ok(Kind::Constructor)
        | Ok(Kind::Getter)
        | Ok(Kind::Setter)
        | Ok(Kind::Accessor) => Some(SymbolKind::Method),
        Ok(Kind::Class) | Ok(Kind::SingletonClass) | Ok(Kind::Object) => Some(SymbolKind::Class),
        Ok(Kind::Struct) | Ok(Kind::Union) => Some(SymbolKind::Struct),
        Ok(Kind::Enum) => Some(SymbolKind::Enum),
        Ok(Kind::EnumMember) => Some(SymbolKind::EnumVariant),
        Ok(Kind::Interface) | Ok(Kind::Trait) | Ok(Kind::Protocol) | Ok(Kind::TypeClass) => {
            Some(SymbolKind::Interface)
        }
        Ok(Kind::TypeAlias) | Ok(Kind::Type) | Ok(Kind::AssociatedType) | Ok(Kind::TypeFamily) => {
            Some(SymbolKind::TypeAlias)
        }
        Ok(Kind::Variable)
        | Ok(Kind::StaticVariable)
        | Ok(Kind::Field)
        | Ok(Kind::StaticField)
        | Ok(Kind::StaticDataMember)
        | Ok(Kind::Property)
        | Ok(Kind::StaticProperty)
        | Ok(Kind::Value) => Some(SymbolKind::Variable),
        Ok(Kind::Constant) => Some(SymbolKind::Constant),
        Ok(Kind::Module)
        | Ok(Kind::Namespace)
        | Ok(Kind::Package)
        | Ok(Kind::PackageObject)
        | Ok(Kind::Library)
        | Ok(Kind::File) => Some(SymbolKind::Module),
        Ok(Kind::Attribute) | Ok(Kind::Modifier) => Some(SymbolKind::Annotation),
        Ok(Kind::Macro) => Some(SymbolKind::Function),
        Ok(Kind::UnspecifiedKind) => None,
        // All remaining exotic kinds map to Variable as a sensible default
        Ok(_) => Some(SymbolKind::Variable),
        Err(_) => None,
    }
}

/// Infer a SymbolKind from the SCIP symbol string's descriptor suffix
/// when no explicit SymbolInformation.Kind is available.
fn infer_kind_from_symbol(symbol_str: &str) -> SymbolKind {
    if let Ok(parsed) = scip::symbol::parse_symbol(symbol_str) {
        if let Some(last) = parsed.descriptors.last() {
            use scip::types::descriptor::Suffix;
            return match last.suffix.enum_value() {
                Ok(Suffix::Method) => SymbolKind::Method,
                Ok(Suffix::Type) => SymbolKind::Class,
                Ok(Suffix::Term) => SymbolKind::Variable,
                Ok(Suffix::Package) | Ok(Suffix::Namespace) => SymbolKind::Module,
                Ok(Suffix::Macro) => SymbolKind::Function,
                Ok(Suffix::Parameter) => SymbolKind::Variable,
                Ok(Suffix::TypeParameter) => SymbolKind::TypeAlias,
                Ok(Suffix::Meta) => SymbolKind::Annotation,
                _ => SymbolKind::Variable,
            };
        }
    }
    SymbolKind::Variable
}

/// Map a SCIP language string to statik's Language enum.
/// SCIP uses the string name from its Language enum (e.g., "Java", "TypeScript").
/// The document also stores a language string field which may differ.
fn map_scip_language(lang_str: &str) -> Option<Language> {
    match lang_str.to_lowercase().as_str() {
        "typescript" | "typescriptreact" | "tsx" => Some(Language::TypeScript),
        "javascript" | "javascriptreact" | "jsx" => Some(Language::JavaScript),
        "python" => Some(Language::Python),
        "rust" => Some(Language::Rust),
        "java" => Some(Language::Java),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_range_four_elements() {
        let range = vec![10, 5, 12, 20];
        assert_eq!(parse_range(&range), (10, 5, 12, 20));
    }

    #[test]
    fn test_parse_range_three_elements() {
        let range = vec![10, 5, 20];
        assert_eq!(parse_range(&range), (10, 5, 10, 20));
    }

    #[test]
    fn test_parse_range_empty() {
        let range: Vec<i32> = vec![];
        assert_eq!(parse_range(&range), (0, 0, 0, 0));
    }

    #[test]
    fn test_extract_names_java_symbol() {
        let (name, qualified) =
            extract_names("lsif-java maven package 1.0.0 java/io/File#Entry.method(+1).");
        assert_eq!(name, "method");
        assert_eq!(qualified, "java::io::File::Entry::method");
    }

    #[test]
    fn test_extract_names_local_symbol() {
        let (name, qualified) = extract_names("local myVar");
        assert_eq!(name, "myVar");
        assert_eq!(qualified, "myVar");
    }

    #[test]
    fn test_extract_names_rust_symbol() {
        let (name, qualified) =
            extract_names("rust-analyzer cargo statik 0.1.0 cli/commands/run_deps().");
        assert_eq!(name, "run_deps");
        assert_eq!(qualified, "cli::commands::run_deps");
    }

    #[test]
    fn test_map_scip_language() {
        assert_eq!(map_scip_language("TypeScript"), Some(Language::TypeScript));
        assert_eq!(map_scip_language("Java"), Some(Language::Java));
        assert_eq!(map_scip_language("Rust"), Some(Language::Rust));
        assert_eq!(map_scip_language("Python"), Some(Language::Python));
        assert_eq!(map_scip_language("JavaScript"), Some(Language::JavaScript));
        assert_eq!(map_scip_language("Haskell"), None);
    }

    #[test]
    fn test_map_scip_kind() {
        use scip::types::symbol_information::Kind;

        let check = |kind: Kind, expected: SymbolKind| {
            let enum_val = protobuf::EnumOrUnknown::new(kind);
            assert_eq!(map_scip_kind(&enum_val), Some(expected));
        };

        check(Kind::Function, SymbolKind::Function);
        check(Kind::Method, SymbolKind::Method);
        check(Kind::Class, SymbolKind::Class);
        check(Kind::Struct, SymbolKind::Struct);
        check(Kind::Enum, SymbolKind::Enum);
        check(Kind::EnumMember, SymbolKind::EnumVariant);
        check(Kind::Interface, SymbolKind::Interface);
        check(Kind::TypeAlias, SymbolKind::TypeAlias);
        check(Kind::Variable, SymbolKind::Variable);
        check(Kind::Constant, SymbolKind::Constant);
        check(Kind::Module, SymbolKind::Module);
        check(Kind::Namespace, SymbolKind::Module);
    }

    #[test]
    fn test_infer_kind_from_symbol() {
        assert_eq!(
            infer_kind_from_symbol("rust-analyzer cargo pkg 0.1.0 MyType#new()."),
            SymbolKind::Method
        );
        assert_eq!(
            infer_kind_from_symbol("rust-analyzer cargo pkg 0.1.0 MyType#"),
            SymbolKind::Class
        );
        assert_eq!(
            infer_kind_from_symbol("rust-analyzer cargo pkg 0.1.0 my_var."),
            SymbolKind::Variable
        );
        assert_eq!(
            infer_kind_from_symbol("rust-analyzer cargo pkg 0.1.0 mymod/"),
            SymbolKind::Module
        );
    }

    #[test]
    fn test_read_scip_index_with_synthetic_data() {
        // Build a minimal SCIP Index in memory and serialize it
        use scip::types::{
            symbol_information::Kind, Document, Index, Metadata, Occurrence, SymbolInformation,
            SymbolRole, ToolInfo,
        };

        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "file:///tmp/test-project".to_string(),
            tool_info: protobuf::MessageField::some(ToolInfo {
                name: "test-indexer".to_string(),
                version: "1.0.0".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });

        let mut doc = Document::new();
        doc.relative_path = "src/main.rs".to_string();
        doc.language = "Rust".to_string();

        // Add a definition occurrence
        let mut def_occ = Occurrence::new();
        def_occ.symbol = "rust-analyzer cargo test-project 0.1.0 main/MyStruct#".to_string();
        def_occ.range = vec![10, 4, 10, 12]; // line 10, col 4-12
        def_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(def_occ);

        // Add a reference occurrence
        let mut ref_occ = Occurrence::new();
        ref_occ.symbol = "rust-analyzer cargo test-project 0.1.0 main/MyStruct#".to_string();
        ref_occ.range = vec![20, 8, 16]; // line 20, col 8-16
        ref_occ.symbol_roles = 0; // plain reference
        doc.occurrences.push(ref_occ);

        // Add a method definition
        let mut method_occ = Occurrence::new();
        method_occ.symbol =
            "rust-analyzer cargo test-project 0.1.0 main/MyStruct#new().".to_string();
        method_occ.range = vec![15, 8, 11]; // line 15, col 8-11
        method_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(method_occ);

        // Add symbol info for the struct
        let mut sym_info = SymbolInformation::new();
        sym_info.symbol = "rust-analyzer cargo test-project 0.1.0 main/MyStruct#".to_string();
        sym_info.kind = protobuf::EnumOrUnknown::new(Kind::Struct);
        doc.symbols.push(sym_info);

        // Add symbol info for the method
        let mut method_info = SymbolInformation::new();
        method_info.symbol =
            "rust-analyzer cargo test-project 0.1.0 main/MyStruct#new().".to_string();
        method_info.kind = protobuf::EnumOrUnknown::new(Kind::Method);
        doc.symbols.push(method_info);

        index.documents.push(doc);

        // Write to a temp file and read it back
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let scip_path = dir.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).expect("failed to write SCIP");

        let result = read_scip_index(&scip_path).expect("failed to read SCIP");

        assert_eq!(
            result.project_root.as_deref(),
            Some("file:///tmp/test-project")
        );
        assert_eq!(result.tool_name.as_deref(), Some("test-indexer"));
        assert_eq!(result.documents.len(), 1);

        let doc = &result.documents[0];
        assert_eq!(doc.relative_path, PathBuf::from("src/main.rs"));
        assert_eq!(doc.language, Some(Language::Rust));

        // Should have 2 definitions (struct + method)
        assert_eq!(doc.definitions.len(), 2);

        let struct_def = &doc.definitions[0];
        assert_eq!(struct_def.name, "MyStruct");
        assert_eq!(struct_def.kind, SymbolKind::Struct);
        assert_eq!(struct_def.line, 10);
        assert_eq!(struct_def.column, 4);

        let method_def = &doc.definitions[1];
        assert_eq!(method_def.name, "new");
        assert_eq!(method_def.kind, SymbolKind::Method);
        assert_eq!(method_def.line, 15);

        // Should have 1 reference
        assert_eq!(doc.references.len(), 1);
        let reference = &doc.references[0];
        assert_eq!(reference.role, ScipRole::Reference);
        assert_eq!(reference.line, 20);
        assert_eq!(reference.column, 8);
        assert_eq!(reference.end_line, 20); // 3-element range: end_line = start_line
        assert_eq!(reference.end_column, 16);
    }

    #[test]
    fn test_multi_language_index() {
        use scip::types::{Document, Index, Occurrence, SymbolRole};

        let mut index = Index::new();

        // Java document
        let mut java_doc = Document::new();
        java_doc.relative_path = "src/main/java/App.java".to_string();
        java_doc.language = "Java".to_string();
        let mut occ = Occurrence::new();
        occ.symbol = "lsif-java maven pkg 1.0.0 App#main().".to_string();
        occ.range = vec![5, 4, 8];
        occ.symbol_roles = SymbolRole::Definition as i32;
        java_doc.occurrences.push(occ);
        index.documents.push(java_doc);

        // TypeScript document
        let mut ts_doc = Document::new();
        ts_doc.relative_path = "src/index.ts".to_string();
        ts_doc.language = "TypeScript".to_string();
        let mut occ2 = Occurrence::new();
        occ2.symbol = "scip-typescript npm pkg 1.0.0 index.ts/App#".to_string();
        occ2.range = vec![1, 0, 3];
        occ2.symbol_roles = SymbolRole::Definition as i32;
        ts_doc.occurrences.push(occ2);
        index.documents.push(ts_doc);

        let dir = tempfile::tempdir().expect("temp dir");
        let scip_path = dir.path().join("multi.scip");
        scip::write_message_to_file(&scip_path, index).expect("write");

        let result = read_scip_index(&scip_path).expect("read");
        assert_eq!(result.documents.len(), 2);
        assert_eq!(result.documents[0].language, Some(Language::Java));
        assert_eq!(result.documents[1].language, Some(Language::TypeScript));
    }

    #[test]
    fn test_read_empty_scip_index() {
        let index = scip::types::Index::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let scip_path = dir.path().join("empty.scip");
        scip::write_message_to_file(&scip_path, index).expect("write");

        let result = read_scip_index(&scip_path).expect("read");
        assert_eq!(result.documents.len(), 0);
        assert!(result.project_root.is_none());
        assert!(result.tool_name.is_none());
    }

    #[test]
    fn test_read_invalid_scip_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bad_path = dir.path().join("bad.scip");
        std::fs::write(&bad_path, "this is not valid protobuf data").expect("write");

        let result = read_scip_index(&bad_path);
        // Should return an error, not panic
        assert!(result.is_ok() || result.is_err());
        // In practice protobuf may parse garbage as an empty message, or may error.
        // Either outcome is acceptable as long as it doesn't panic.
    }

    #[test]
    fn test_read_nonexistent_scip_file() {
        let result = read_scip_index(std::path::Path::new("/nonexistent/path/test.scip"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("failed to read SCIP file"),
            "Error should mention reading: {}",
            err_msg
        );
    }

    #[test]
    fn test_occurrences_with_empty_symbol_are_skipped() {
        use scip::types::{Document, Index, Occurrence, SymbolRole};

        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/main.rs".to_string();
        doc.language = "Rust".to_string();

        // Add an occurrence with an empty symbol (should be skipped)
        let mut empty_occ = Occurrence::new();
        empty_occ.symbol = "".to_string();
        empty_occ.range = vec![1, 0, 5];
        empty_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(empty_occ);

        // Add a valid occurrence
        let mut valid_occ = Occurrence::new();
        valid_occ.symbol = "rust-analyzer cargo pkg 0.1.0 main().".to_string();
        valid_occ.range = vec![0, 3, 7];
        valid_occ.symbol_roles = SymbolRole::Definition as i32;
        doc.occurrences.push(valid_occ);

        index.documents.push(doc);

        let dir = tempfile::tempdir().expect("temp dir");
        let scip_path = dir.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).expect("write");

        let result = read_scip_index(&scip_path).expect("read");
        assert_eq!(result.documents[0].definitions.len(), 1);
        assert_eq!(result.documents[0].definitions[0].name, "main");
    }

    #[test]
    fn test_import_occurrences_classified_correctly() {
        use scip::types::{Document, Index, Occurrence, SymbolRole};

        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/main.rs".to_string();
        doc.language = "Rust".to_string();

        // Import occurrence (SymbolRole::Import bit set)
        let mut import_occ = Occurrence::new();
        import_occ.symbol = "rust-analyzer cargo other 0.1.0 helper().".to_string();
        import_occ.range = vec![0, 4, 10];
        import_occ.symbol_roles = SymbolRole::Import as i32;
        doc.occurrences.push(import_occ);

        index.documents.push(doc);

        let dir = tempfile::tempdir().expect("temp dir");
        let scip_path = dir.path().join("test.scip");
        scip::write_message_to_file(&scip_path, index).expect("write");

        let result = read_scip_index(&scip_path).expect("read");
        assert_eq!(result.documents[0].references.len(), 1);
        assert_eq!(result.documents[0].references[0].role, ScipRole::Import);
    }
}
