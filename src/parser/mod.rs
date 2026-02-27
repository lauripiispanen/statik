use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::model::{FileId, Language, LanguageSemantics, ParseResult};

pub mod java;
pub mod resolve;
pub mod rust;
pub mod typescript;

/// Trait for language-specific symbol extractors.
///
/// Each language implements this trait to walk a tree-sitter CST and produce
/// a unified set of symbols, references, imports, and exports.
/// Also provides language-specific semantic rules via the `LanguageSemantics` supertrait.
pub trait LanguageParser: LanguageSemantics {
    /// Parse a source file and extract symbols, references, imports, exports.
    fn parse(&self, file_id: FileId, source: &str, path: &Path) -> Result<ParseResult>;

    /// Which languages does this parser handle?
    fn supported_languages(&self) -> &[Language];

    /// Upcast to `&dyn LanguageSemantics` without unstable trait upcasting.
    fn as_semantics(&self) -> &dyn LanguageSemantics;
}

/// Registry of language parsers.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn LanguageParser>>,
}

impl ParserRegistry {
    /// Create a registry with all built-in parsers.
    pub fn with_defaults() -> Self {
        let mut registry = Self {
            parsers: Vec::new(),
        };
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

    /// Build a lookup table of language semantics for passing to analysis functions.
    pub fn semantics_map(&self) -> HashMap<Language, &dyn LanguageSemantics> {
        let mut map = HashMap::new();
        for parser in &self.parsers {
            for &lang in parser.languages() {
                map.insert(lang, parser.as_ref().as_semantics());
            }
        }
        map
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
