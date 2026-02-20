use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Tree};

use crate::model::{
    ExportRecord, FileId, ImportRecord, Language, LineSpan, ParseResult, Position, RefKind,
    Reference, ReferenceId, Span, Symbol, SymbolId, SymbolKind, Visibility,
};

use super::LanguageParser;

#[derive(Default)]
pub struct TypeScriptParser {
    // We create parsers per-call since tree_sitter::Parser is not Sync
}

impl TypeScriptParser {
    pub fn new() -> Self {
        Self::default()
    }

    fn create_parser(language: Language) -> Result<Parser> {
        let mut parser = Parser::new();
        let ts_language = match language {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            _ => anyhow::bail!("unsupported language: {}", language),
        };
        parser
            .set_language(&ts_language)
            .context("failed to set parser language")?;
        Ok(parser)
    }

    fn detect_language(path: &Path) -> Language {
        match path.extension().and_then(|e| e.to_str()) {
            Some("ts" | "tsx") => Language::TypeScript,
            _ => Language::JavaScript,
        }
    }
}

impl LanguageParser for TypeScriptParser {
    fn parse(&self, file_id: FileId, source: &str, path: &Path) -> Result<ParseResult> {
        let language = Self::detect_language(path);
        let mut parser = Self::create_parser(language)?;
        let tree = parser
            .parse(source, None)
            .context("tree-sitter failed to parse")?;

        let mut extractor = Extractor::new(file_id, source, &tree);
        extractor.extract();

        Ok(ParseResult {
            file_id,
            symbols: extractor.symbols,
            references: extractor.references,
            imports: extractor.imports,
            exports: extractor.exports,
            type_references: Vec::new(),
            annotations: Vec::new(),
        })
    }

    fn supported_languages(&self) -> &[Language] {
        &[Language::TypeScript, Language::JavaScript]
    }
}

/// Walks a tree-sitter CST and extracts symbols, references, imports, exports.
struct Extractor<'a> {
    file_id: FileId,
    source: &'a str,
    tree: &'a Tree,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    imports: Vec<ImportRecord>,
    exports: Vec<ExportRecord>,
    next_symbol_id: u64,
    next_ref_id: u64,
    /// Stack of parent symbol IDs for tracking nesting.
    parent_stack: Vec<SymbolId>,
}

impl<'a> Extractor<'a> {
    fn new(file_id: FileId, source: &'a str, tree: &'a Tree) -> Self {
        Self {
            file_id,
            source,
            tree,
            symbols: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            next_symbol_id: file_id.0 * 100_000 + 1,
            next_ref_id: file_id.0 * 100_000 + 1,
            parent_stack: Vec::new(),
        }
    }

    fn alloc_symbol_id(&mut self) -> SymbolId {
        let id = SymbolId(self.next_symbol_id);
        self.next_symbol_id += 1;
        id
    }

    fn alloc_ref_id(&mut self) -> ReferenceId {
        let id = ReferenceId(self.next_ref_id);
        self.next_ref_id += 1;
        id
    }

    fn node_text(&self, node: Node) -> &str {
        node.utf8_text(self.source.as_bytes()).unwrap_or("")
    }

    /// Find the first identifier child of a node and return its text.
    fn find_identifier_in(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return Some(self.node_text(child).to_string());
            }
        }
        None
    }

    fn node_span(&self, node: Node) -> Span {
        Span {
            start: node.start_byte(),
            end: node.end_byte(),
        }
    }

    fn node_line_span(&self, node: Node) -> LineSpan {
        let start = node.start_position();
        let end = node.end_position();
        LineSpan {
            start: Position {
                line: start.row + 1,
                column: start.column,
            },
            end: Position {
                line: end.row + 1,
                column: end.column,
            },
        }
    }

    fn current_parent(&self) -> Option<SymbolId> {
        self.parent_stack.last().copied()
    }

    fn qualified_name(&self, name: &str) -> String {
        if self.parent_stack.is_empty() {
            return name.to_string();
        }
        // Find parent symbol name
        if let Some(parent_id) = self.current_parent() {
            if let Some(parent) = self.symbols.iter().find(|s| s.id == parent_id) {
                return format!("{}::{}", parent.qualified_name, name);
            }
        }
        name.to_string()
    }

    fn extract(&mut self) {
        let root = self.tree.root_node();
        self.visit_children(root);
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node(child);
        }
    }

    fn visit_node(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                self.extract_function(node, false);
            }
            "class_declaration" => {
                self.extract_class(node);
            }
            "interface_declaration" => {
                self.extract_interface(node);
            }
            "type_alias_declaration" => {
                self.extract_type_alias(node);
            }
            "enum_declaration" => {
                self.extract_enum(node);
            }
            "lexical_declaration" => {
                self.extract_variable_declaration(node);
            }
            "variable_declaration" => {
                self.extract_variable_declaration(node);
            }
            "import_statement" => {
                self.extract_import(node);
            }
            "export_statement" => {
                self.extract_export(node);
            }
            "call_expression" => {
                if !self.try_extract_dynamic_import(node) {
                    self.extract_call_reference(node);
                }
                // Still visit children for nested calls
                self.visit_children(node);
            }
            "new_expression" => {
                self.extract_new_reference(node);
                self.visit_children(node);
            }
            _ => {
                self.visit_children(node);
            }
        }
    }

    fn is_exported(&self, node: Node) -> bool {
        if let Some(parent) = node.parent() {
            parent.kind() == "export_statement"
        } else {
            false
        }
    }

    fn extract_function(&mut self, node: Node, is_method: bool) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let exported = self.is_exported(node);
        let kind = if is_method {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        let signature = self.extract_function_signature(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature,
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }

        // Visit function body for nested symbols and references
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.parent_stack.pop();
    }

    fn extract_function_signature(&self, node: Node) -> Option<String> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.node_text(n))
            .unwrap_or("anonymous");
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(n))
            .unwrap_or("()");
        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(n))
            .unwrap_or("");

        Some(format!("{}{}{}", name, params, return_type))
    }

    fn extract_class(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let exported = self.is_exported(node);
        let id = self.alloc_symbol_id();

        // Check for heritage (extends/implements)
        self.extract_heritage(node, id);

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Class,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature: Some(format!("class {}", name)),
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }

        // Visit class body for methods and properties
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_class_body(body);
        }
        self.parent_stack.pop();
    }

    fn visit_class_body(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "method_definition" => {
                    self.extract_method(child);
                }
                "public_field_definition" | "property_definition" => {
                    self.extract_property(child);
                }
                _ => {}
            }
        }
    }

    fn extract_method(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let id = self.alloc_symbol_id();
        let signature = self.extract_function_signature(node);

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Method,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: self.extract_member_visibility(node),
            signature,
        };
        self.symbols.push(symbol);

        // Visit method body
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.parent_stack.pop();
    }

    fn extract_member_visibility(&self, node: Node) -> Visibility {
        // Check for accessibility modifier
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "accessibility_modifier" {
                match self.node_text(child) {
                    "private" => return Visibility::Private,
                    "protected" => return Visibility::Protected,
                    _ => return Visibility::Public,
                }
            }
        }
        Visibility::Public
    }

    fn extract_property(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let id = self.alloc_symbol_id();
        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Variable,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: self.extract_member_visibility(node),
            signature: None,
        };
        self.symbols.push(symbol);
    }

    fn extract_interface(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let exported = self.is_exported(node);
        let id = self.alloc_symbol_id();

        // Check for heritage (extends)
        self.extract_heritage(node, id);

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Interface,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature: Some(format!("interface {}", name)),
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }
    }

    fn extract_type_alias(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let exported = self.is_exported(node);
        let id = self.alloc_symbol_id();
        let type_text = node
            .child_by_field_name("value")
            .map(|n| self.node_text(n).to_string());

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::TypeAlias,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature: type_text.map(|t| format!("type {} = {}", name, t)),
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }
    }

    fn extract_enum(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let name = match name_node {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let exported = self.is_exported(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Enum,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature: Some(format!("enum {}", name)),
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }

        // Extract enum members
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "enum_member" => {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let member_name = self.node_text(name_node).to_string();
                            let member_id = self.alloc_symbol_id();
                            self.symbols.push(Symbol {
                                id: member_id,
                                qualified_name: self.qualified_name(&member_name),
                                name: member_name,
                                kind: SymbolKind::EnumVariant,
                                file: self.file_id,
                                span: self.node_span(child),
                                line_span: self.node_line_span(child),
                                parent: self.current_parent(),
                                visibility: Visibility::Public,
                                signature: None,
                            });
                        }
                    }
                    "property_identifier" => {
                        let member_name = self.node_text(child).to_string();
                        if !member_name.is_empty() {
                            let member_id = self.alloc_symbol_id();
                            self.symbols.push(Symbol {
                                id: member_id,
                                qualified_name: self.qualified_name(&member_name),
                                name: member_name,
                                kind: SymbolKind::EnumVariant,
                                file: self.file_id,
                                span: self.node_span(child),
                                line_span: self.node_line_span(child),
                                parent: self.current_parent(),
                                visibility: Visibility::Public,
                                signature: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        self.parent_stack.pop();
    }

    fn extract_variable_declaration(&mut self, node: Node) {
        let exported = self.is_exported(node);
        let is_const = self.node_text(node).starts_with("const");

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                self.extract_variable_declarator(child, exported, is_const);
            }
        }
    }

    fn extract_variable_declarator(&mut self, node: Node, exported: bool, is_const: bool) {
        let name_node = node.child_by_field_name("name");
        let name_node = match name_node {
            Some(n) => n,
            None => return,
        };

        // Handle destructuring patterns
        if name_node.kind() == "object_pattern" || name_node.kind() == "array_pattern" {
            self.extract_destructured_names(name_node, exported, is_const);
            return;
        }

        let name = self.node_text(name_node).to_string();
        if name.is_empty() {
            return;
        }

        let kind = if is_const {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };

        let id = self.alloc_symbol_id();
        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: if exported {
                Visibility::Public
            } else {
                Visibility::Private
            },
            signature: None,
        };
        self.symbols.push(symbol);

        if exported {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }

        // Check if the value is a function expression or arrow function
        if let Some(value) = node.child_by_field_name("value") {
            match value.kind() {
                "arrow_function" | "function_expression" | "generator_function" => {
                    // Visit the function body for references
                    self.parent_stack.push(id);
                    if let Some(body) = value.child_by_field_name("body") {
                        self.visit_children(body);
                    }
                    self.parent_stack.pop();
                }
                _ => {
                    // Visit value for references (e.g., new expressions, call expressions).
                    // Don't push the variable as parent - attribute references to the
                    // enclosing function/method scope instead.
                    self.visit_node(value);
                }
            }
        }
    }

    fn extract_destructured_names(&mut self, node: Node, exported: bool, is_const: bool) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "shorthand_property_identifier_pattern" | "shorthand_property_identifier" => {
                    let name = self.node_text(child).to_string();
                    if !name.is_empty() {
                        let kind = if is_const {
                            SymbolKind::Constant
                        } else {
                            SymbolKind::Variable
                        };
                        let id = self.alloc_symbol_id();
                        self.symbols.push(Symbol {
                            id,
                            qualified_name: self.qualified_name(&name),
                            name: name.clone(),
                            kind,
                            file: self.file_id,
                            span: self.node_span(child),
                            line_span: self.node_line_span(child),
                            parent: self.current_parent(),
                            visibility: if exported {
                                Visibility::Public
                            } else {
                                Visibility::Private
                            },
                            signature: None,
                        });
                        if exported {
                            self.exports.push(ExportRecord {
                                file: self.file_id,
                                symbol: id,
                                exported_name: name,
                                is_default: false,
                                is_reexport: false,
                                is_type_only: false,
                                source_path: None,
                            });
                        }
                    }
                }
                "pair_pattern" => {
                    // { key: value } destructuring - value is the local name
                    if let Some(value) = child.child_by_field_name("value") {
                        self.extract_destructured_names(value, exported, is_const);
                    }
                }
                "object_pattern" | "array_pattern" => {
                    self.extract_destructured_names(child, exported, is_const);
                }
                "identifier" => {
                    let name = self.node_text(child).to_string();
                    if !name.is_empty() {
                        let kind = if is_const {
                            SymbolKind::Constant
                        } else {
                            SymbolKind::Variable
                        };
                        let id = self.alloc_symbol_id();
                        self.symbols.push(Symbol {
                            id,
                            qualified_name: self.qualified_name(&name),
                            name: name.clone(),
                            kind,
                            file: self.file_id,
                            span: self.node_span(child),
                            line_span: self.node_line_span(child),
                            parent: self.current_parent(),
                            visibility: if exported {
                                Visibility::Public
                            } else {
                                Visibility::Private
                            },
                            signature: None,
                        });
                        if exported {
                            self.exports.push(ExportRecord {
                                file: self.file_id,
                                symbol: id,
                                exported_name: name,
                                is_default: false,
                                is_reexport: false,
                                is_type_only: false,
                                source_path: None,
                            });
                        }
                    }
                }
                _ => {
                    // Recurse for nested patterns
                    self.extract_destructured_names(child, exported, is_const);
                }
            }
        }
    }

    fn extract_import(&mut self, node: Node) {
        // Find the source (module path)
        let source_node = node.child_by_field_name("source");
        let source_path = match source_node {
            Some(n) => {
                let text = self.node_text(n);
                // Strip quotes
                text.trim_matches(|c| c == '\'' || c == '"').to_string()
            }
            None => return,
        };

        // Check for `import type` syntax
        let is_type_only = self.node_text(node).starts_with("import type");

        // Track import count before processing to detect side-effect imports
        let imports_before = self.imports.len();

        // Look for import clause
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_clause" => {
                    self.extract_import_clause(child, &source_path, node, is_type_only);
                }
                "namespace_import" => {
                    // import * as name from '...'
                    let local = self.find_identifier_in(child);
                    if let Some(local) = local {
                        self.imports.push(ImportRecord {
                            file: self.file_id,
                            source_path: source_path.clone(),
                            imported_name: "*".to_string(),
                            local_name: local,
                            span: self.node_span(node),
                            line_span: self.node_line_span(node),
                            is_default: false,
                            is_namespace: true,
                            is_type_only,
                            is_side_effect: false,
                            is_dynamic: false,
                        });
                    }
                }
                "named_imports" => {
                    self.extract_named_imports(child, &source_path, node, is_type_only);
                }
                _ => {}
            }
        }

        // If no named/default/namespace imports were added, this is a side-effect import
        // e.g. `import './polyfill'`
        if self.imports.len() == imports_before {
            self.imports.push(ImportRecord {
                file: self.file_id,
                source_path,
                imported_name: String::new(),
                local_name: String::new(),
                span: self.node_span(node),
                line_span: self.node_line_span(node),
                is_default: false,
                is_namespace: false,
                is_type_only: false,
                is_side_effect: true,
                is_dynamic: false,
            });
        }
    }

    fn extract_import_clause(
        &mut self,
        node: Node,
        source_path: &str,
        import_node: Node,
        is_type_only: bool,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    // Default import: import Foo from '...'
                    let local = self.node_text(child).to_string();
                    self.imports.push(ImportRecord {
                        file: self.file_id,
                        source_path: source_path.to_string(),
                        imported_name: "default".to_string(),
                        local_name: local,
                        span: self.node_span(import_node),
                        line_span: self.node_line_span(import_node),
                        is_default: true,
                        is_namespace: false,
                        is_type_only,
                        is_side_effect: false,
                        is_dynamic: false,
                    });
                }
                "named_imports" => {
                    self.extract_named_imports(child, source_path, import_node, is_type_only);
                }
                "namespace_import" => {
                    let local = self.find_identifier_in(child);
                    if let Some(local) = local {
                        self.imports.push(ImportRecord {
                            file: self.file_id,
                            source_path: source_path.to_string(),
                            imported_name: "*".to_string(),
                            local_name: local,
                            span: self.node_span(import_node),
                            line_span: self.node_line_span(import_node),
                            is_default: false,
                            is_namespace: true,
                            is_type_only,
                            is_side_effect: false,
                            is_dynamic: false,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn extract_named_imports(
        &mut self,
        node: Node,
        source_path: &str,
        import_node: Node,
        is_type_only: bool,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import_specifier" {
                let name_node = child.child_by_field_name("name");
                let alias_node = child.child_by_field_name("alias");

                if let Some(name_n) = name_node {
                    let imported_name = self.node_text(name_n).to_string();
                    let local_name = alias_node
                        .map(|n| self.node_text(n).to_string())
                        .unwrap_or_else(|| imported_name.clone());

                    self.imports.push(ImportRecord {
                        file: self.file_id,
                        source_path: source_path.to_string(),
                        imported_name,
                        local_name,
                        span: self.node_span(import_node),
                        line_span: self.node_line_span(import_node),
                        is_default: false,
                        is_namespace: false,
                        is_type_only,
                        is_side_effect: false,
                        is_dynamic: false,
                    });
                }
            }
        }
    }

    fn extract_export(&mut self, node: Node) {
        // Check for re-exports: export { ... } from '...'
        let source_node = node.child_by_field_name("source");

        let mut cursor = node.walk();
        let mut has_declaration = false;
        let mut is_default = false;

        for child in node.children(&mut cursor) {
            match child.kind() {
                "export_clause" => {
                    // export { a, b } or export { a, b } from '...'
                    self.extract_export_clause(
                        child,
                        source_node.map(|n| {
                            self.node_text(n)
                                .trim_matches(|c| c == '\'' || c == '"')
                                .to_string()
                        }),
                    );
                    return;
                }
                "*" => {
                    // export * from '...' (wildcard re-export)
                    if let Some(src) = source_node {
                        let source_path = self
                            .node_text(src)
                            .trim_matches(|c| c == '\'' || c == '"')
                            .to_string();
                        let id = self.alloc_symbol_id();
                        self.symbols.push(Symbol {
                            id,
                            qualified_name: "*".to_string(),
                            name: "*".to_string(),
                            kind: SymbolKind::Export,
                            file: self.file_id,
                            span: self.node_span(node),
                            line_span: self.node_line_span(node),
                            parent: None,
                            visibility: Visibility::Public,
                            signature: None,
                        });
                        self.exports.push(ExportRecord {
                            file: self.file_id,
                            symbol: id,
                            exported_name: "*".to_string(),
                            is_default: false,
                            is_reexport: true,
                            is_type_only: false,
                            source_path: Some(source_path.clone()),
                        });
                        // Also emit an ImportRecord so the re-export creates a
                        // file-level dependency edge in build_file_graph()
                        self.imports.push(ImportRecord {
                            file: self.file_id,
                            source_path,
                            imported_name: "*".to_string(),
                            local_name: String::new(),
                            span: self.node_span(node),
                            line_span: self.node_line_span(node),
                            is_default: false,
                            is_namespace: true,
                            is_type_only: false,
                            is_side_effect: false,
                            is_dynamic: false,
                        });
                    }
                    return;
                }
                "default" => {
                    is_default = true;
                }
                // Declaration exports: export function foo, export class Bar, etc.
                "function_declaration" | "generator_function_declaration" => {
                    has_declaration = true;
                    self.extract_function(child, false);
                }
                "class_declaration" => {
                    has_declaration = true;
                    self.extract_class(child);
                }
                "interface_declaration" => {
                    has_declaration = true;
                    self.extract_interface(child);
                }
                "type_alias_declaration" => {
                    has_declaration = true;
                    self.extract_type_alias(child);
                }
                "enum_declaration" => {
                    has_declaration = true;
                    self.extract_enum(child);
                }
                "lexical_declaration" | "variable_declaration" => {
                    has_declaration = true;
                    self.extract_variable_declaration(child);
                }
                _ => {}
            }
        }

        if is_default && !has_declaration {
            // export default <expression>
            // We create a synthetic symbol for the default export
            let id = self.alloc_symbol_id();
            let symbol = Symbol {
                id,
                qualified_name: "default".to_string(),
                name: "default".to_string(),
                kind: SymbolKind::Export,
                file: self.file_id,
                span: self.node_span(node),
                line_span: self.node_line_span(node),
                parent: None,
                visibility: Visibility::Public,
                signature: None,
            };
            self.symbols.push(symbol);
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: "default".to_string(),
                is_default: true,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
            });
        }
    }

    fn extract_export_clause(&mut self, node: Node, source_path: Option<String>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "export_specifier" {
                let name_node = child.child_by_field_name("name");
                let alias_node = child.child_by_field_name("alias");

                if let Some(name_n) = name_node {
                    let local_name = self.node_text(name_n).to_string();
                    let exported_name = alias_node
                        .map(|n| self.node_text(n).to_string())
                        .unwrap_or_else(|| local_name.clone());

                    let id = self.alloc_symbol_id();
                    let symbol = Symbol {
                        id,
                        qualified_name: exported_name.clone(),
                        name: exported_name.clone(),
                        kind: SymbolKind::Export,
                        file: self.file_id,
                        span: self.node_span(child),
                        line_span: self.node_line_span(child),
                        parent: None,
                        visibility: Visibility::Public,
                        signature: None,
                    };
                    self.symbols.push(symbol);

                    let is_reexport = source_path.is_some();
                    self.exports.push(ExportRecord {
                        file: self.file_id,
                        symbol: id,
                        exported_name: exported_name.clone(),
                        is_default: false,
                        is_reexport,
                        is_type_only: false,
                        source_path: source_path.clone(),
                    });

                    // For re-exports (export { foo } from './module'), also emit
                    // an ImportRecord so the re-export creates a file-level
                    // dependency edge in build_file_graph()
                    if let Some(ref src) = source_path {
                        self.imports.push(ImportRecord {
                            file: self.file_id,
                            source_path: src.clone(),
                            imported_name: local_name,
                            local_name: exported_name,
                            span: self.node_span(child),
                            line_span: self.node_line_span(child),
                            is_default: false,
                            is_namespace: false,
                            is_type_only: false,
                            is_side_effect: false,
                            is_dynamic: false,
                        });
                    }
                }
            }
        }
    }

    fn extract_heritage(&mut self, node: Node, symbol_id: SymbolId) {
        // Look for extends/implements clauses in the class/interface declaration.
        // Tree-sitter structures:
        //   class:     class_heritage > extends_clause > identifier[field:value]
        //   class:     class_heritage > implements_clause > type_identifier
        //   interface: extends_type_clause > type_identifier[field:type]
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "class_heritage" => {
                    // Contains extends_clause and/or implements_clause
                    self.extract_heritage_clauses(child, symbol_id);
                }
                "extends_type_clause" => {
                    // Interface extends: direct type_identifier children
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "type_identifier"
                            || inner_child.kind() == "identifier"
                        {
                            self.add_inheritance_ref(symbol_id, inner_child);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn extract_heritage_clauses(&mut self, node: Node, symbol_id: SymbolId) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "extends_clause" => {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "identifier"
                            || inner_child.kind() == "type_identifier"
                        {
                            self.add_inheritance_ref(symbol_id, inner_child);
                        }
                    }
                }
                "implements_clause" => {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "type_identifier"
                            || inner_child.kind() == "identifier"
                        {
                            self.add_inheritance_ref(symbol_id, inner_child);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn add_inheritance_ref(&mut self, source: SymbolId, target_node: Node) {
        let ref_id = self.alloc_ref_id();
        let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
        self.references.push(Reference {
            id: ref_id,
            source,
            target: placeholder_target,
            kind: RefKind::Inheritance,
            file: self.file_id,
            span: self.node_span(target_node),
            line_span: self.node_line_span(target_node),
        });
    }

    /// Try to extract a dynamic import expression: `import('./module')`.
    /// Returns true if this call expression was a dynamic import, false otherwise.
    fn try_extract_dynamic_import(&mut self, node: Node) -> bool {
        let func_node = match node.child_by_field_name("function") {
            Some(n) => n,
            None => return false,
        };

        // Dynamic imports appear as call_expression with function = "import"
        if func_node.kind() != "import" {
            return false;
        }

        // Get the arguments
        let args_node = match node.child_by_field_name("arguments") {
            Some(n) => n,
            None => return false,
        };

        // The first argument should be a string literal
        let mut cursor = args_node.walk();
        let first_arg = args_node.children(&mut cursor).find(|c| {
            c.kind() == "string" || c.kind() == "template_string" || c.kind() != ","
                && c.kind() != "("
                && c.kind() != ")"
        });

        if let Some(arg) = first_arg {
            if arg.kind() == "string" {
                // String literal argument: import('./module')
                let source_path = self
                    .node_text(arg)
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string();
                self.imports.push(ImportRecord {
                    file: self.file_id,
                    source_path,
                    imported_name: "*".to_string(),
                    local_name: String::new(),
                    span: self.node_span(node),
                    line_span: self.node_line_span(node),
                    is_default: false,
                    is_namespace: true,
                    is_type_only: false,
                    is_side_effect: false,
                    is_dynamic: true,
                });
            }
            // Non-literal arguments (template strings, variables) -- we can't
            // resolve these statically, so just skip them. The call_expression
            // will still be visited for call references.
        }

        true
    }

    fn extract_call_reference(&mut self, node: Node) {
        let func_node = node.child_by_field_name("function");
        if func_node.is_none() {
            return;
        }
        let func_node = func_node.unwrap();

        // Only record call references if we have a parent context
        if let Some(source_id) = self.current_parent() {
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source: source_id,
                target: placeholder_target,
                kind: RefKind::Call,
                file: self.file_id,
                span: self.node_span(func_node),
                line_span: self.node_line_span(func_node),
            });
        }
    }

    fn extract_new_reference(&mut self, node: Node) {
        // new ClassName(...)
        let constructor = node.child_by_field_name("constructor");
        if let Some(ctor_node) = constructor {
            if let Some(source_id) = self.current_parent() {
                let ref_id = self.alloc_ref_id();
                let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
                self.references.push(Reference {
                    id: ref_id,
                    source: source_id,
                    target: placeholder_target,
                    kind: RefKind::Call,
                    file: self.file_id,
                    span: self.node_span(ctor_node),
                    line_span: self.node_line_span(ctor_node),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ts(source: &str) -> ParseResult {
        let parser = TypeScriptParser::new();
        parser
            .parse(FileId(1), source, Path::new("test.ts"))
            .unwrap()
    }

    fn parse_js(source: &str) -> ParseResult {
        let parser = TypeScriptParser::new();
        parser
            .parse(FileId(1), source, Path::new("test.js"))
            .unwrap()
    }

    #[test]
    fn test_function_declaration() {
        let result = parse_ts("function greet(name: string): void { }");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "greet");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
        assert!(result.symbols[0].signature.is_some());
    }

    #[test]
    fn test_exported_function() {
        let result = parse_ts("export function doStuff() { }");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "doStuff");
        assert_eq!(result.symbols[0].visibility, Visibility::Public);
        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].exported_name, "doStuff");
    }

    #[test]
    fn test_class_with_methods() {
        let result = parse_ts(
            r#"
class Animal {
    name: string;
    speak() { }
    private run() { }
}
"#,
        );
        let class = result.symbols.iter().find(|s| s.name == "Animal").unwrap();
        assert_eq!(class.kind, SymbolKind::Class);

        let speak = result.symbols.iter().find(|s| s.name == "speak").unwrap();
        assert_eq!(speak.kind, SymbolKind::Method);
        assert_eq!(speak.parent, Some(class.id));

        let run = result.symbols.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.kind, SymbolKind::Method);
        assert_eq!(run.visibility, Visibility::Private);
    }

    #[test]
    fn test_interface() {
        let result = parse_ts(
            r#"
export interface Serializable {
    serialize(): string;
}
"#,
        );
        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Serializable")
            .unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.visibility, Visibility::Public);
    }

    #[test]
    fn test_type_alias() {
        let result = parse_ts("export type UserId = string | number;");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "UserId");
        assert_eq!(result.symbols[0].kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_enum_with_variants() {
        let result = parse_ts(
            r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#,
        );
        let color = result.symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Enum);

        let variants: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumVariant)
            .collect();
        assert_eq!(variants.len(), 3);
        let variant_names: Vec<_> = variants.iter().map(|v| v.name.as_str()).collect();
        assert!(variant_names.contains(&"Red"));
        assert!(variant_names.contains(&"Green"));
        assert!(variant_names.contains(&"Blue"));
    }

    #[test]
    fn test_const_variable() {
        let result = parse_ts("export const MAX_SIZE = 100;");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "MAX_SIZE");
        assert_eq!(result.symbols[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn test_let_variable() {
        let result = parse_ts("let counter = 0;");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "counter");
        assert_eq!(result.symbols[0].kind, SymbolKind::Variable);
    }

    #[test]
    fn test_arrow_function_variable() {
        let result = parse_ts("const add = (a: number, b: number) => a + b;");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "add");
        assert_eq!(result.symbols[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn test_named_imports() {
        let result = parse_ts("import { foo, bar as baz } from './utils';");
        assert_eq!(result.imports.len(), 2);

        let foo_import = result
            .imports
            .iter()
            .find(|i| i.imported_name == "foo")
            .unwrap();
        assert_eq!(foo_import.local_name, "foo");
        assert_eq!(foo_import.source_path, "./utils");
        assert!(!foo_import.is_default);

        let bar_import = result
            .imports
            .iter()
            .find(|i| i.imported_name == "bar")
            .unwrap();
        assert_eq!(bar_import.local_name, "baz");
    }

    #[test]
    fn test_default_import() {
        let result = parse_ts("import React from 'react';");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].imported_name, "default");
        assert_eq!(result.imports[0].local_name, "React");
        assert!(result.imports[0].is_default);
    }

    #[test]
    fn test_namespace_import() {
        let result = parse_ts("import * as utils from './utils';");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].imported_name, "*");
        assert_eq!(result.imports[0].local_name, "utils");
        assert!(result.imports[0].is_namespace);
    }

    #[test]
    fn test_reexport() {
        let result = parse_ts("export { foo, bar as baz } from './module';");
        assert_eq!(result.exports.len(), 2);
        let foo_export = result
            .exports
            .iter()
            .find(|e| e.exported_name == "foo")
            .unwrap();
        assert!(foo_export.is_reexport);
        assert_eq!(foo_export.source_path.as_deref(), Some("./module"));

        let baz_export = result
            .exports
            .iter()
            .find(|e| e.exported_name == "baz")
            .unwrap();
        assert!(baz_export.is_reexport);
    }

    #[test]
    fn test_export_default_expression() {
        let result = parse_ts("export default 42;");
        let default_sym = result.symbols.iter().find(|s| s.name == "default").unwrap();
        assert_eq!(default_sym.kind, SymbolKind::Export);
        assert_eq!(result.exports.len(), 1);
        assert!(result.exports[0].is_default);
    }

    #[test]
    fn test_destructured_const() {
        let result = parse_ts("const { x, y } = getCoords();");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
        assert!(result
            .symbols
            .iter()
            .all(|s| s.kind == SymbolKind::Constant));
    }

    #[test]
    fn test_class_extends() {
        let result = parse_ts(
            r#"
class Dog extends Animal {
    bark() { }
}
"#,
        );
        let dog = result.symbols.iter().find(|s| s.name == "Dog").unwrap();
        assert_eq!(dog.kind, SymbolKind::Class);

        // Should have an inheritance reference from Dog to "Animal"
        let inheritance_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == dog.id)
            .collect();
        assert_eq!(
            inheritance_refs.len(),
            1,
            "Dog should have exactly one Inheritance reference"
        );
    }

    #[test]
    fn test_call_expression_in_function() {
        let result = parse_ts(
            r#"
function main() {
    helper();
    doStuff();
}
"#,
        );
        let main_fn = result.symbols.iter().find(|s| s.name == "main").unwrap();
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == main_fn.id && r.kind == RefKind::Call)
            .collect();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_javascript_parsing() {
        let result = parse_js(
            r#"
function hello() {
    console.log('hello');
}
const x = 42;
"#,
        );
        assert!(result.symbols.iter().any(|s| s.name == "hello"));
        assert!(result.symbols.iter().any(|s| s.name == "x"));
    }

    #[test]
    fn test_multiple_exports() {
        let result = parse_ts(
            r#"
export function foo() { }
export class Bar { }
export const X = 1;
export interface IBaz { }
export type MyType = string;
export enum Status { Active, Inactive }
"#,
        );
        // All top-level declarations should be exported
        let exported: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.visibility == Visibility::Public && s.parent.is_none())
            .collect();
        assert_eq!(exported.len(), 6, "expected exactly 6 exported top-level symbols: foo, Bar, X, IBaz, MyType, Status; got {:?}",
            exported.iter().map(|s| s.name.as_str()).collect::<Vec<_>>());

        assert_eq!(result.exports.len(), 6, "expected exactly 6 export records");
    }

    #[test]
    fn test_nested_class_method_qualified_name() {
        let result = parse_ts(
            r#"
class Foo {
    bar() { }
}
"#,
        );
        let bar = result.symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.qualified_name, "Foo::bar");
    }

    #[test]
    fn test_symbol_positions() {
        let source = "function hello() { }\nconst x = 1;";
        let result = parse_ts(source);

        let hello = result.symbols.iter().find(|s| s.name == "hello").unwrap();
        assert_eq!(hello.line_span.start.line, 1);

        let x = result.symbols.iter().find(|s| s.name == "x").unwrap();
        assert_eq!(x.line_span.start.line, 2);
    }

    #[test]
    fn test_empty_source() {
        let result = parse_ts("");
        assert!(result.symbols.is_empty());
        assert!(result.references.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_mixed_default_and_named_import() {
        let result = parse_ts("import React, { useState, useEffect } from 'react';");
        assert_eq!(result.imports.len(), 3);
        assert!(result
            .imports
            .iter()
            .any(|i| i.is_default && i.local_name == "React"));
        assert!(result.imports.iter().any(|i| i.imported_name == "useState"));
        assert!(result
            .imports
            .iter()
            .any(|i| i.imported_name == "useEffect"));
    }

    // --- Edge case tests for parser completeness ---

    #[test]
    fn test_anonymous_default_export_function() {
        // Common in React: export default function() { }
        let result = parse_ts("export default function() { }");
        // Should produce a default export even without a function name
        assert!(
            result.exports.iter().any(|e| e.is_default),
            "anonymous default export function should produce a default export record"
        );
    }

    #[test]
    fn test_anonymous_default_export_class() {
        let result = parse_ts("export default class { method() { } }");
        assert!(
            result.exports.iter().any(|e| e.is_default),
            "anonymous default export class should produce a default export record"
        );
    }

    #[test]
    fn test_array_destructuring() {
        let result = parse_ts("const [a, b, c] = getValues();");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"a"),
            "array destructuring should extract 'a': found {:?}",
            names
        );
        assert!(
            names.contains(&"b"),
            "array destructuring should extract 'b': found {:?}",
            names
        );
        assert!(
            names.contains(&"c"),
            "array destructuring should extract 'c': found {:?}",
            names
        );
    }

    #[test]
    fn test_wildcard_reexport() {
        let result = parse_ts("export * from './module';");
        assert!(
            !result.exports.is_empty(),
            "export * should produce at least one export record"
        );
        assert!(result.exports[0].is_reexport);
        assert_eq!(result.exports[0].exported_name, "*");
        assert_eq!(result.exports[0].source_path.as_deref(), Some("./module"));
    }

    #[test]
    fn test_dynamic_import_expression() {
        let result = parse_ts(
            r#"
async function loadModule() {
    const mod = await import("./lazy");
}
"#,
        );
        // Dynamic imports should create an import record with the source path
        assert!(
            result.imports.iter().any(|i| i.source_path == "./lazy"),
            "dynamic import('./lazy') should generate an import record"
        );
        let dynamic_import = result.imports.iter().find(|i| i.source_path == "./lazy").unwrap();
        assert!(dynamic_import.is_dynamic, "should be marked as dynamic");
        assert!(dynamic_import.is_namespace, "dynamic imports are namespace imports");
    }

    // --- Additional edge case tests added by test-reviewer ---

    #[test]
    fn test_side_effect_only_import() {
        let result = parse_ts("import './polyfill';");
        assert!(result.symbols.is_empty());
        // Side-effect imports should create an import record to track the dependency
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "./polyfill");
        assert!(result.imports[0].is_side_effect);
        assert!(result.imports[0].imported_name.is_empty());
    }

    #[test]
    fn test_type_only_import() {
        let result = parse_ts("import type { Config } from './config';");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].imported_name, "Config");
        assert!(result.imports[0].is_type_only);
        assert!(!result.imports[0].is_side_effect);
    }

    #[test]
    fn test_export_default_class_declaration() {
        let result = parse_ts(
            r#"
export default class Greeter {
    greet() { return "hello"; }
}
"#,
        );
        // Should extract the class AND mark it as a default export
        let class = result.symbols.iter().find(|s| s.name == "Greeter");
        assert!(
            class.is_some(),
            "default exported class should be extracted as a named symbol"
        );
        if let Some(cls) = class {
            assert_eq!(cls.kind, SymbolKind::Class);
        }
    }

    #[test]
    fn test_export_default_function_declaration() {
        let result = parse_ts(
            r#"
export default function handler() { }
"#,
        );
        let func = result.symbols.iter().find(|s| s.name == "handler");
        assert!(
            func.is_some(),
            "default exported function should be extracted as a named symbol"
        );
        if let Some(f) = func {
            assert_eq!(f.kind, SymbolKind::Function);
        }
    }

    #[test]
    fn test_async_function() {
        let result = parse_ts("async function fetchData(): Promise<void> { }");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "fetchData");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_generator_function() {
        let result = parse_ts("function* range(start: number, end: number) { yield start; }");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "range");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_new_expression_generates_call_reference() {
        let result = parse_ts(
            r#"
function main() {
    const obj = new MyClass("arg");
}
"#,
        );
        // The `new MyClass()` generates a Call reference. The source is the enclosing
        // function (main), since the variable declaration processing visits the value
        // before pushing the variable as parent context.
        let call_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .collect();
        assert!(
            !call_refs.is_empty(),
            "new MyClass() should generate a Call reference"
        );
    }

    #[test]
    fn test_nested_call_expressions() {
        let result = parse_ts(
            r#"
function process() {
    outer(middle(inner()));
}
"#,
        );
        let process_fn = result.symbols.iter().find(|s| s.name == "process").unwrap();
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == process_fn.id && r.kind == RefKind::Call)
            .collect();
        assert_eq!(
            calls.len(),
            3,
            "outer(middle(inner())) should generate 3 call references"
        );
    }

    #[test]
    fn test_class_getter_and_setter() {
        let result = parse_ts(
            r#"
class Config {
    private _value: number = 0;
    get value(): number { return this._value; }
    set value(v: number) { this._value = v; }
}
"#,
        );
        let config = result.symbols.iter().find(|s| s.name == "Config").unwrap();
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.parent == Some(config.id) && s.kind == SymbolKind::Method)
            .collect();
        // Getters and setters should be extracted as methods
        let method_names: Vec<_> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(
            method_names.contains(&"value"),
            "getter should be extracted: found {:?}",
            method_names
        );
    }

    #[test]
    fn test_const_enum() {
        let result = parse_ts(
            r#"
const enum Direction {
    Up = "UP",
    Down = "DOWN",
}
"#,
        );
        let dir = result.symbols.iter().find(|s| s.name == "Direction");
        assert!(dir.is_some(), "const enum should be extracted");
        if let Some(d) = dir {
            assert_eq!(d.kind, SymbolKind::Enum);
        }
    }

    #[test]
    fn test_interface_extends() {
        let result = parse_ts(
            r#"
interface ClickableProps extends ComponentProps {
    onClick: () => void;
}
"#,
        );
        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "ClickableProps")
            .unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);

        let inheritance_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == iface.id)
            .collect();
        assert!(
            !inheritance_refs.is_empty(),
            "interface extends should generate Inheritance reference"
        );
    }

    #[test]
    fn test_class_implements() {
        let result = parse_ts(
            r#"
class Dog implements Animal, Serializable {
    name: string = "Rex";
}
"#,
        );
        let dog = result.symbols.iter().find(|s| s.name == "Dog").unwrap();
        let impl_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == dog.id)
            .collect();
        assert_eq!(
            impl_refs.len(),
            2,
            "class implementing two interfaces should generate 2 Inheritance references"
        );
    }

    #[test]
    fn test_multiple_variable_declarators() {
        let result = parse_ts("const a = 1, b = 2, c = 3;");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
        assert_eq!(result.symbols.len(), 3);
    }

    #[test]
    fn test_parser_does_not_crash_on_syntax_errors() {
        // Intentionally malformed TypeScript - parser should not panic
        let result = parse_ts("function { broken syntax export class ;;; }}}");
        // We don't care what's extracted, just that it doesn't crash
        let _ = result;
    }

    #[test]
    fn test_complex_real_world_file() {
        // Simulate a realistic file with mixed constructs
        let result = parse_ts(
            r#"
import { Logger } from './logger';
import type { Config } from './config';

export interface UserService {
    getUser(id: string): Promise<User>;
}

export class UserServiceImpl implements UserService {
    private logger: Logger;

    constructor(logger: Logger) {
        this.logger = logger;
    }

    async getUser(id: string): Promise<User> {
        this.logger.info(`Fetching user ${id}`);
        return findUser(id);
    }

    private validate(id: string): boolean {
        return id.length > 0;
    }
}

export function createUserService(logger: Logger): UserService {
    return new UserServiceImpl(logger);
}

export type User = {
    id: string;
    name: string;
};

const CACHE_TTL = 3600;
"#,
        );

        // Verify key symbols are extracted
        let symbol_names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            symbol_names.contains(&"UserService"),
            "interface should be extracted"
        );
        assert!(
            symbol_names.contains(&"UserServiceImpl"),
            "class should be extracted"
        );
        assert!(
            symbol_names.contains(&"getUser"),
            "method should be extracted"
        );
        assert!(
            symbol_names.contains(&"validate"),
            "private method should be extracted"
        );
        assert!(
            symbol_names.contains(&"createUserService"),
            "function should be extracted"
        );
        assert!(
            symbol_names.contains(&"User"),
            "type alias should be extracted"
        );
        assert!(
            symbol_names.contains(&"CACHE_TTL"),
            "const should be extracted"
        );

        // Verify imports
        assert_eq!(result.imports.len(), 2);
        assert!(result.imports.iter().any(|i| i.imported_name == "Logger"));
        assert!(result.imports.iter().any(|i| i.imported_name == "Config"));

        // Verify exports
        let export_names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(export_names.contains(&"UserService"));
        assert!(export_names.contains(&"UserServiceImpl"));
        assert!(export_names.contains(&"createUserService"));
        assert!(export_names.contains(&"User"));

        // CACHE_TTL is not exported
        assert!(!export_names.contains(&"CACHE_TTL"));
    }

    #[test]
    fn test_wildcard_reexport_creates_import_record() {
        let result = parse_ts("export * from './module';");
        // Should create both an ExportRecord and an ImportRecord
        assert_eq!(result.exports.len(), 1);
        assert!(result.exports[0].is_reexport);

        assert!(
            result.imports.iter().any(|i| i.source_path == "./module" && i.imported_name == "*"),
            "wildcard re-export should also create an ImportRecord for the dependency edge"
        );
    }

    #[test]
    fn test_named_reexport_creates_import_records() {
        let result = parse_ts("export { foo, bar as baz } from './module';");
        // Should create ExportRecords AND ImportRecords
        assert_eq!(result.exports.len(), 2);

        let import_names: Vec<&str> = result.imports.iter().map(|i| i.imported_name.as_str()).collect();
        assert!(
            import_names.contains(&"foo"),
            "named re-export should create ImportRecord for 'foo'"
        );
        assert!(
            import_names.contains(&"bar"),
            "named re-export should create ImportRecord for 'bar' (original name)"
        );
    }

    #[test]
    fn test_dynamic_import_non_literal_ignored() {
        // Template literals and variables can't be resolved statically
        let result = parse_ts(
            r#"
function load(name: string) {
    return import(name);
}
"#,
        );
        // No import record should be created for variable arguments
        assert!(
            result.imports.is_empty(),
            "dynamic import with variable argument should not create an import record"
        );
    }

    #[test]
    fn test_dynamic_import_top_level() {
        // Top-level dynamic import (not inside a function)
        let result = parse_ts(r#"const p = import("./module");"#);
        assert!(
            result.imports.iter().any(|i| i.source_path == "./module" && i.is_dynamic),
            "top-level dynamic import should be extracted"
        );
    }
}
