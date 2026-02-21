use std::path::{Path, PathBuf};

pub mod java;
pub mod rust;
pub mod tsconfig;
pub mod typescript;

/// Result of resolving an import path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Successfully resolved to an absolute file path.
    Resolved(PathBuf),
    /// Resolved but with a caveat about precision.
    ResolvedWithCaveat(PathBuf, ResolutionCaveat),
    /// The import refers to an external package (e.g. node_modules).
    External(String),
    /// Could not resolve the import.
    Unresolved(UnresolvedReason),
}

/// Caveats that reduce confidence in a resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionCaveat {
    /// Resolved through an `export *` barrel file; the specific symbol may not exist.
    BarrelFileWildcard,
    /// Multiple index files could match; we picked the first one.
    AmbiguousIndex,
    /// Resolved via tsconfig path alias; the mapping may be ambiguous.
    PathAlias,
    /// Both `foo.rs` and `foo/mod.rs` exist (Rust E0761); picked `foo.rs`.
    AmbiguousModule,
}

/// Reasons why an import could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// The import path uses a computed/dynamic expression.
    DynamicPath,
    /// The import refers to a third-party package in node_modules.
    NodeModules,
    /// The target file was not found on disk.
    FileNotFound(String),
    /// The import syntax is not supported by this resolver.
    UnsupportedSyntax(String),
}

/// Context about the project needed for resolution.
#[derive(Debug)]
pub struct ProjectContext {
    /// The root directory of the project.
    pub root: PathBuf,
    /// Known file paths in the project (for fast existence checks).
    pub known_files: Vec<PathBuf>,
}

/// Trait for language-specific import resolution.
///
/// The resolver takes an import path string (e.g. `"./utils"`, `"@/components/Button"`)
/// and the file containing the import, and resolves it to an actual file path.
pub trait Resolver: Send + Sync {
    /// Resolve an import path to a file.
    ///
    /// - `import_source`: The string literal from the import statement (e.g. `"./utils"`)
    /// - `from_file`: The absolute path of the file containing the import
    fn resolve(&self, import_source: &str, from_file: &Path) -> Resolution;
}
