use anyhow::Result;
use std::path::Path;

use crate::model::{FileId, Language, ParseResult};

pub mod java;
pub mod rust;
pub mod typescript;

/// Trait for language-specific symbol extractors.
///
/// Each language implements this trait to walk a tree-sitter CST and produce
/// a unified set of symbols, references, imports, and exports.
pub trait LanguageParser: Send + Sync {
    /// Parse a source file and extract symbols, references, imports, exports.
    fn parse(&self, file_id: FileId, source: &str, path: &Path) -> Result<ParseResult>;

    /// Which languages does this parser handle?
    fn supported_languages(&self) -> &[Language];
}

/// Registry of language parsers.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn LanguageParser>>,
}

impl ParserRegistry {
    pub fn new() -> Self {
        Self {
            parsers: Vec::new(),
        }
    }

    /// Create a registry with all built-in parsers.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(typescript::TypeScriptParser::new()));
        registry.register(Box::new(java::JavaParser::new()));
        registry.register(Box::new(rust::RustParser::new()));
        registry
    }

    pub fn register(&mut self, parser: Box<dyn LanguageParser>) {
        self.parsers.push(parser);
    }

    /// Find a parser that supports the given language.
    pub fn parser_for(&self, language: Language) -> Option<&dyn LanguageParser> {
        self.parsers
            .iter()
            .find(|p| p.supported_languages().contains(&language))
            .map(|p| p.as_ref())
    }

    /// Parse a source file using the appropriate language parser.
    pub fn parse(
        &self,
        file_id: FileId,
        source: &str,
        path: &Path,
        language: Language,
    ) -> Result<ParseResult> {
        let parser = self
            .parser_for(language)
            .ok_or_else(|| anyhow::anyhow!("no parser for language: {}", language))?;
        parser.parse(file_id, source, path)
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}
