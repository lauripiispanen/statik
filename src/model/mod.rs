use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReferenceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    EnumVariant,
    Interface,
    TypeAlias,
    Variable,
    Constant,
    Module,
    Import,
    Export,
    Annotation,
    Package,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::EnumVariant => "enum_variant",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Module => "module",
            SymbolKind::Import => "import",
            SymbolKind::Export => "export",
            SymbolKind::Annotation => "annotation",
            SymbolKind::Package => "package",
        }
    }
}

impl FromStr for SymbolKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" => Ok(SymbolKind::Function),
            "method" => Ok(SymbolKind::Method),
            "class" => Ok(SymbolKind::Class),
            "struct" => Ok(SymbolKind::Struct),
            "enum" => Ok(SymbolKind::Enum),
            "enum_variant" => Ok(SymbolKind::EnumVariant),
            "interface" => Ok(SymbolKind::Interface),
            "type_alias" => Ok(SymbolKind::TypeAlias),
            "variable" => Ok(SymbolKind::Variable),
            "constant" => Ok(SymbolKind::Constant),
            "module" => Ok(SymbolKind::Module),
            "import" => Ok(SymbolKind::Import),
            "export" => Ok(SymbolKind::Export),
            "annotation" => Ok(SymbolKind::Annotation),
            "package" => Ok(SymbolKind::Package),
            _ => Err(format!("unknown symbol kind: {}", s)),
        }
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKind {
    Call,
    TypeUsage,
    Import,
    Export,
    Inheritance,
    FieldAccess,
    Assignment,
}

impl RefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefKind::Call => "call",
            RefKind::TypeUsage => "type_usage",
            RefKind::Import => "import",
            RefKind::Export => "export",
            RefKind::Inheritance => "inheritance",
            RefKind::FieldAccess => "field_access",
            RefKind::Assignment => "assignment",
        }
    }
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Protected,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "protected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineSpan {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub file: FileId,
    pub span: Span,
    pub line_span: LineSpan,
    pub parent: Option<SymbolId>,
    pub visibility: Visibility,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: ReferenceId,
    pub source: SymbolId,
    pub target: SymbolId,
    pub kind: RefKind,
    pub file: FileId,
    pub span: Span,
    pub line_span: LineSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: FileId,
    pub path: PathBuf,
    pub mtime: u64,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
    Java,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Java => "java",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" | "pyi" => Some(Language::Python),
            "rs" => Some(Language::Rust),
            "java" => Some(Language::Java),
            _ => None,
        }
    }

    /// Parse a language from its stored string representation (as returned by `as_str()`).
    pub fn from_stored_str(s: &str) -> Option<Self> {
        match s {
            "typescript" => Some(Language::TypeScript),
            "javascript" => Some(Language::JavaScript),
            "python" => Some(Language::Python),
            "rust" => Some(Language::Rust),
            "java" => Some(Language::Java),
            _ => None,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Represents an unresolved import extracted from a source file.
/// The import path needs to be resolved to an actual file/symbol during graph construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    pub file: FileId,
    pub source_path: String,
    pub imported_name: String,
    pub local_name: String,
    pub span: Span,
    pub line_span: LineSpan,
    pub is_default: bool,
    pub is_namespace: bool,
    pub is_type_only: bool,
    pub is_side_effect: bool,
}

/// Represents an export from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRecord {
    pub file: FileId,
    pub symbol: SymbolId,
    pub exported_name: String,
    pub is_default: bool,
    pub is_reexport: bool,
    pub is_type_only: bool,
    pub source_path: Option<String>,
}

/// Result of parsing a single file.
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub file_id: FileId,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub imports: Vec<ImportRecord>,
    pub exports: Vec<ExportRecord>,
}

pub mod file_graph;
pub mod graph;
