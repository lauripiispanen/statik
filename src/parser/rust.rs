use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Tree};

use crate::model::{
    ExportRecord, FileId, ImportRecord, Language, LineSpan, ParseResult, Position, RefKind,
    Reference, ReferenceId, Span, Symbol, SymbolId, SymbolKind, Visibility,
};

use super::LanguageParser;

#[derive(Default)]
pub struct RustParser;

impl RustParser {
    pub fn new() -> Self {
        Self
    }

    fn create_parser() -> Result<Parser> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .context("failed to set Rust parser language")?;
        Ok(parser)
    }
}

impl LanguageParser for RustParser {
    fn parse(&self, file_id: FileId, source: &str, _path: &Path) -> Result<ParseResult> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(source, None)
            .context("tree-sitter failed to parse Rust")?;

        let mut extractor = Extractor::new(file_id, source, &tree);
        extractor.extract();
        extractor.resolve_intra_file_refs();

        Ok(ParseResult {
            file_id,
            symbols: extractor.symbols,
            references: extractor.references,
            imports: extractor.imports,
            exports: extractor.exports,
            type_references: vec![],
            annotations: vec![],
        })
    }

    fn supported_languages(&self) -> &[Language] {
        &[Language::Rust]
    }
}

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
    parent_stack: Vec<SymbolId>,
    ref_target_names: Vec<String>,
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
            ref_target_names: Vec::new(),
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
            "use_declaration" => self.extract_use(node),
            "function_item" => self.extract_function(node),
            "struct_item" => self.extract_struct(node),
            "enum_item" => self.extract_enum(node),
            "trait_item" => self.extract_trait(node),
            "type_item" => self.extract_type_alias(node),
            "const_item" => self.extract_const(node),
            "static_item" => self.extract_static(node),
            "mod_item" => self.extract_mod(node),
            "impl_item" => self.extract_impl(node),
            "macro_definition" => self.extract_macro_def(node),
            "extern_crate_declaration" => self.extract_extern_crate(node),
            "call_expression" => {
                self.extract_call_reference(node);
                self.visit_children(node);
            }
            "struct_expression" => {
                self.extract_struct_expression_reference(node);
                self.visit_children(node);
            }
            "type_identifier" => {
                self.extract_type_reference(node);
            }
            _ => self.visit_children(node),
        }
    }

    // --- Visibility ---

    fn extract_visibility(&self, node: Node) -> Visibility {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                let text = self.node_text(child);
                if text.contains("crate") || text.contains("super") || text.contains("in ") {
                    return Visibility::Protected;
                }
                return Visibility::Public;
            }
        }
        Visibility::Private
    }

    // --- Use declarations ---

    fn extract_use(&mut self, node: Node) {
        let vis = self.extract_visibility(node);
        let is_pub = vis == Visibility::Public;
        let decl_span = self.node_span(node);
        let decl_line_span = self.node_line_span(node);

        // Find the argument child of the use_declaration
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "use" | "visibility_modifier" | ";" => {}
                _ => {
                    self.walk_use_tree(child, &[], is_pub, decl_span, decl_line_span);
                }
            }
        }
    }

    fn walk_use_tree(
        &mut self,
        node: Node,
        prefix: &[String],
        is_pub: bool,
        decl_span: Span,
        decl_line_span: LineSpan,
    ) {
        match node.kind() {
            "scoped_identifier" => {
                let full_path = self.node_text(node).to_string();
                let name = full_path
                    .rsplit("::")
                    .next()
                    .unwrap_or(&full_path)
                    .to_string();
                self.add_import(&full_path, &name, None, false, decl_span, decl_line_span);
                if is_pub {
                    self.add_reexport(&full_path, &name, decl_line_span);
                }
            }
            "identifier" => {
                let name = self.node_text(node).to_string();
                let full_path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", prefix.join("::"), name)
                };
                self.add_import(&full_path, &name, None, false, decl_span, decl_line_span);
                if is_pub {
                    self.add_reexport(&full_path, &name, decl_line_span);
                }
            }
            "use_as_clause" => {
                let mut path_node = None;
                let mut alias_node = None;
                let mut inner_cursor = node.walk();
                for child in node.children(&mut inner_cursor) {
                    match child.kind() {
                        "scoped_identifier" | "identifier" => {
                            if path_node.is_none() {
                                path_node = Some(child);
                            } else {
                                alias_node = Some(child);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(pn) = path_node {
                    let full_path = self.node_text(pn).to_string();
                    let name = full_path
                        .rsplit("::")
                        .next()
                        .unwrap_or(&full_path)
                        .to_string();
                    let alias = alias_node.map(|n| self.node_text(n).to_string());
                    self.add_import(
                        &full_path,
                        &name,
                        alias.as_deref(),
                        false,
                        decl_span,
                        decl_line_span,
                    );
                    if is_pub {
                        let export_name = alias.as_deref().unwrap_or(&name);
                        self.add_reexport(&full_path, export_name, decl_line_span);
                    }
                }
            }
            "use_wildcard" => {
                let text = self.node_text(node).to_string();
                let path = text.strip_suffix("::*").unwrap_or("");
                let full_path = if prefix.is_empty() {
                    path.to_string()
                } else if path.is_empty() {
                    prefix.join("::")
                } else {
                    format!("{}::{}", prefix.join("::"), path)
                };
                self.add_import(&full_path, "*", None, true, decl_span, decl_line_span);
                if is_pub {
                    self.add_reexport(&full_path, "*", decl_line_span);
                }
            }
            "scoped_use_list" => {
                // Collect path prefix first, then process use_list
                let mut path_parts: Vec<String> = prefix.to_vec();
                let mut use_list_node = None;
                let mut inner_cursor = node.walk();
                for child in node.children(&mut inner_cursor) {
                    match child.kind() {
                        "use_list" => {
                            use_list_node = Some(child);
                        }
                        "identifier" | "scoped_identifier" | "self" | "crate" | "super" => {
                            path_parts = prefix.to_vec();
                            let text = self.node_text(child).to_string();
                            for segment in text.split("::") {
                                path_parts.push(segment.to_string());
                            }
                        }
                        "::" => {}
                        _ => {}
                    }
                }
                if let Some(list_node) = use_list_node {
                    self.walk_use_tree(list_node, &path_parts, is_pub, decl_span, decl_line_span);
                }
            }
            "use_list" => {
                let mut inner_cursor = node.walk();
                for child in node.children(&mut inner_cursor) {
                    match child.kind() {
                        "," | "{" | "}" => {}
                        "self" => {
                            let full_path = prefix.join("::");
                            let name = prefix.last().map(|s| s.as_str()).unwrap_or("self");
                            self.add_import(
                                &full_path,
                                name,
                                None,
                                false,
                                decl_span,
                                decl_line_span,
                            );
                            if is_pub {
                                self.add_reexport(&full_path, name, decl_line_span);
                            }
                        }
                        _ => {
                            self.walk_use_tree(child, prefix, is_pub, decl_span, decl_line_span);
                        }
                    }
                }
            }
            "self" => {
                let full_path = if prefix.is_empty() {
                    "self".to_string()
                } else {
                    format!("{}::self", prefix.join("::"))
                };
                let name = prefix.last().map(|s| s.as_str()).unwrap_or("self");
                self.add_import(&full_path, name, None, false, decl_span, decl_line_span);
                if is_pub {
                    self.add_reexport(&full_path, name, decl_line_span);
                }
            }
            _ => {
                let mut inner_cursor = node.walk();
                for child in node.children(&mut inner_cursor) {
                    self.walk_use_tree(child, prefix, is_pub, decl_span, decl_line_span);
                }
            }
        }
    }

    fn add_import(
        &mut self,
        source_path: &str,
        imported_name: &str,
        local_name: Option<&str>,
        is_namespace: bool,
        span: Span,
        line_span: LineSpan,
    ) {
        self.imports.push(ImportRecord {
            file: self.file_id,
            source_path: source_path.to_string(),
            imported_name: if is_namespace {
                "*".to_string()
            } else {
                imported_name.to_string()
            },
            local_name: local_name.unwrap_or("").to_string(),
            span,
            line_span,
            is_default: false,
            is_namespace,
            is_type_only: false,
            is_side_effect: false,
            is_dynamic: false,
        });
    }

    fn add_reexport(
        &mut self,
        source_path: &str,
        exported_name: &str,
        decl_line_span: LineSpan,
    ) {
        let id = self.alloc_symbol_id();
        let span = Span { start: 0, end: 0 };
        // Create a synthetic symbol so the export FK constraint is satisfied
        self.symbols.push(Symbol {
            id,
            qualified_name: exported_name.to_string(),
            name: exported_name.to_string(),
            kind: SymbolKind::Export,
            file: self.file_id,
            span,
            line_span: decl_line_span,
            parent: None,
            visibility: Visibility::Public,
            signature: None,
        });
        self.exports.push(ExportRecord {
            file: self.file_id,
            symbol: id,
            exported_name: exported_name.to_string(),
            is_default: false,
            is_reexport: true,
            is_type_only: false,
            source_path: Some(source_path.to_string()),
            line: decl_line_span.start.line,
        });
    }

    // --- Extern crate ---

    fn extract_extern_crate(&mut self, node: Node) {
        // `extern crate foo;` or `extern crate foo as bar;`
        // Use the "name" field which is the crate name identifier
        let crate_name = node
            .child_by_field_name("name")
            .map(|n| self.node_text(n).to_string());
        let alias_name = node
            .child_by_field_name("alias")
            .map(|n| self.node_text(n).to_string());
        if let Some(name) = crate_name {
            let source_path = format!("extern::{}", name);
            let span = self.node_span(node);
            let line_span = self.node_line_span(node);
            self.add_import(
                &source_path,
                &name,
                alias_name.as_deref(),
                false,
                span,
                line_span,
            );
        }
    }

    // --- Symbols ---

    fn extract_function(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        // Classify as Method only when parent is a type (Struct/Enum/Trait),
        // not when inside a mod block
        let kind = if self
            .current_parent()
            .and_then(|pid| self.symbols.iter().find(|s| s.id == pid))
            .is_some_and(|s| {
                s.kind == SymbolKind::Struct
                    || s.kind == SymbolKind::Enum
                    || s.kind == SymbolKind::Interface
            }) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        let id = self.alloc_symbol_id();
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_else(|| "()".to_string());
        let ret = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(n).to_string());
        let signature = match ret {
            Some(r) => format!("fn {}{} -> {}", name, params, r),
            None => format!("fn {}{}", name, params),
        };

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(signature),
        });

        // Export if pub at file scope or in pub parent
        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }

        self.parent_stack.push(id);
        // Visit parameter and return types for type references
        if let Some(params) = node.child_by_field_name("parameters") {
            self.scan_for_type_refs(params);
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            self.scan_for_type_refs(ret);
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.parent_stack.pop();
    }

    fn extract_struct(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Struct,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(format!("struct {}", name)),
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_enum(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Enum,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(format!("enum {}", name)),
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }

        // Extract enum variants
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "enum_variant" {
                    self.extract_enum_variant(child);
                }
            }
        }
        self.parent_stack.pop();
    }

    fn extract_enum_variant(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let id = self.alloc_symbol_id();
        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::EnumVariant,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: Visibility::Public,
            signature: None,
        });
    }

    fn extract_trait(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Interface,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(format!("trait {}", name)),
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }

        // Extract trait body methods (both with and without default implementations)
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "function_item" => self.extract_function(child),
                    "function_signature_item" => self.extract_function_signature(child),
                    _ => {}
                }
            }
        }
        self.parent_stack.pop();
    }

    fn extract_function_signature(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_else(|| "()".to_string());
        let ret = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(n).to_string());
        let signature = match ret {
            Some(r) => format!("fn {}{} -> {}", name, params, r),
            None => format!("fn {}{}", name, params),
        };

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Method,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(signature),
        });
    }

    fn extract_type_alias(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::TypeAlias,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(format!("type {}", name)),
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_const(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Constant,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: None,
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_static(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Variable,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: None,
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_mod(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let vis = self.extract_visibility(node);
        let id = self.alloc_symbol_id();

        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Module,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: vis,
            signature: Some(format!("mod {}", name)),
        });

        if vis == Visibility::Public && self.parent_stack.is_empty() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name.clone(),
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: node.start_position().row + 1,
            });
        }

        // Check if this is an external mod declaration (no body, just semicolon)
        // vs an inline mod (has a body block)
        let has_body = node.child_by_field_name("body").is_some();

        if !has_body {
            // External mod declaration: `mod foo;`
            // Emit synthetic import for module file resolution
            let span = Span { start: 0, end: 0 };
            let line_span = self.node_line_span(node);
            self.imports.push(ImportRecord {
                file: self.file_id,
                source_path: format!("@mod:{}", name),
                imported_name: name,
                local_name: String::new(),
                span,
                line_span,
                is_default: false,
                is_namespace: false,
                is_type_only: false,
                is_side_effect: true,
                is_dynamic: false,
            });
        } else {
            // Inline mod: visit its body
            self.parent_stack.push(id);
            if let Some(body) = node.child_by_field_name("body") {
                self.visit_children(body);
            }
            self.parent_stack.pop();
        }
    }

    fn extract_impl(&mut self, node: Node) {
        // Find the type being implemented
        let type_name = node
            .child_by_field_name("type")
            .map(|n| self.node_text(n).to_string());

        // Check for trait implementation
        let trait_node = node.child_by_field_name("trait");

        // Find the impl target symbol to use as parent
        let parent_id = type_name.as_ref().and_then(|tn| {
            // Strip generic params for matching
            let base = tn.split('<').next().unwrap_or(tn).trim();
            self.symbols.iter().find(|s| s.name == base).map(|s| s.id)
        });

        // If it's a trait impl, emit an inheritance reference
        if let (Some(trait_n), Some(pid)) = (trait_node, parent_id) {
            let trait_name = self.node_text(trait_n).to_string();
            let base_trait = trait_name.split('<').next().unwrap_or(&trait_name).trim();
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source: pid,
                target: placeholder_target,
                kind: RefKind::Inheritance,
                file: self.file_id,
                span: self.node_span(trait_n),
                line_span: self.node_line_span(trait_n),
            });
            self.ref_target_names.push(base_trait.to_string());
        }

        // Visit impl body with the type as parent
        if let Some(pid) = parent_id {
            self.parent_stack.push(pid);
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "function_item" {
                    self.extract_function(child);
                } else if child.kind() == "type_item" {
                    self.extract_type_alias(child);
                } else if child.kind() == "const_item" {
                    self.extract_const(child);
                } else {
                    self.visit_node(child);
                }
            }
        }
        if parent_id.is_some() {
            self.parent_stack.pop();
        }
    }

    fn extract_macro_def(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let id = self.alloc_symbol_id();
        self.symbols.push(Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Function,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility: Visibility::Private,
            signature: None,
        });
    }

    // --- References ---

    fn extract_call_reference(&mut self, node: Node) {
        let func = match node.child_by_field_name("function") {
            Some(n) => n,
            None => return,
        };

        let target_name = match func.kind() {
            "identifier" => self.node_text(func).to_string(),
            "field_expression" => {
                // method call: obj.method()
                if let Some(field) = func.child_by_field_name("field") {
                    self.node_text(field).to_string()
                } else {
                    return;
                }
            }
            "scoped_identifier" => {
                // qualified call: Foo::bar()
                let text = self.node_text(func);
                text.rsplit("::").next().unwrap_or(text).to_string()
            }
            _ => return,
        };

        if let Some(source_id) = self.find_enclosing_symbol() {
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source: source_id,
                target: placeholder_target,
                kind: RefKind::Call,
                file: self.file_id,
                span: self.node_span(node),
                line_span: self.node_line_span(node),
            });
            self.ref_target_names.push(target_name);
        }
    }

    fn extract_struct_expression_reference(&mut self, node: Node) {
        let name_node = match node.child_by_field_name("name") {
            Some(n) => n,
            None => return,
        };

        let target_name = {
            let text = self.node_text(name_node);
            text.rsplit("::").next().unwrap_or(text).to_string()
        };

        if let Some(source_id) = self.find_enclosing_symbol() {
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source: source_id,
                target: placeholder_target,
                kind: RefKind::Call,
                file: self.file_id,
                span: self.node_span(node),
                line_span: self.node_line_span(node),
            });
            self.ref_target_names.push(target_name);
        }
    }

    fn extract_type_reference(&mut self, node: Node) {
        if let Some(source_id) = self.find_enclosing_symbol() {
            let target_name = self.node_text(node).to_string();
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source: source_id,
                target: placeholder_target,
                kind: RefKind::TypeUsage,
                file: self.file_id,
                span: self.node_span(node),
                line_span: self.node_line_span(node),
            });
            self.ref_target_names.push(target_name);
        }
    }

    fn find_enclosing_symbol(&self) -> Option<SymbolId> {
        self.parent_stack.last().copied()
    }

    /// Recursively scan a subtree for type_identifier nodes, emitting TypeUsage references.
    fn scan_for_type_refs(&mut self, node: Node) {
        if node.kind() == "type_identifier" {
            self.extract_type_reference(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_for_type_refs(child);
        }
    }

    // --- Intra-file resolution ---

    fn resolve_intra_file_refs(&mut self) {
        use std::collections::HashMap;

        let mut name_to_id: HashMap<&str, Option<SymbolId>> = HashMap::new();
        for symbol in &self.symbols {
            match name_to_id.get(symbol.name.as_str()) {
                None => {
                    name_to_id.insert(&symbol.name, Some(symbol.id));
                }
                Some(Some(_)) => {
                    name_to_id.insert(&symbol.name, None);
                }
                Some(None) => {}
            }
        }

        for (i, reference) in self.references.iter_mut().enumerate() {
            if reference.target.0 >= u64::MAX - 1_000_000 {
                if let Some(target_name) = self.ref_target_names.get(i) {
                    if let Some(Some(resolved_id)) = name_to_id.get(target_name.as_str()) {
                        reference.target = *resolved_id;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> ParseResult {
        let parser = RustParser::new();
        parser
            .parse(FileId(1), source, Path::new("test.rs"))
            .unwrap()
    }

    #[test]
    fn test_empty_source() {
        let result = parse_rust("");
        assert!(result.symbols.is_empty());
        assert!(result.references.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_simple_function() {
        let result = parse_rust("fn hello() {}");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
        assert_eq!(result.symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_pub_function_is_exported() {
        let result = parse_rust("pub fn greet() {}");
        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].exported_name, "greet");
        assert!(!result.exports[0].is_reexport);
    }

    #[test]
    fn test_private_function_not_exported() {
        let result = parse_rust("fn helper() {}");
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_struct_with_methods() {
        let result = parse_rust(
            r#"
pub struct Foo;

impl Foo {
    pub fn new() -> Self { Foo }
    fn private_method(&self) {}
}
"#,
        );

        let foo = result.symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.kind, SymbolKind::Struct);
        assert_eq!(foo.visibility, Visibility::Public);

        let new_method = result.symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_method.kind, SymbolKind::Method);
        assert_eq!(new_method.parent, Some(foo.id));
        assert_eq!(new_method.visibility, Visibility::Public);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "private_method")
            .unwrap();
        assert_eq!(private.kind, SymbolKind::Method);
        assert_eq!(private.visibility, Visibility::Private);
    }

    #[test]
    fn test_enum_with_variants() {
        let result = parse_rust(
            r#"
pub enum Color {
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
        let names: Vec<_> = variants.iter().map(|v| v.name.as_str()).collect();
        assert!(names.contains(&"Red"));
        assert!(names.contains(&"Green"));
        assert!(names.contains(&"Blue"));

        for v in &variants {
            assert_eq!(v.parent, Some(color.id));
            assert_eq!(v.visibility, Visibility::Public);
        }
    }

    #[test]
    fn test_trait_declaration() {
        let result = parse_rust(
            r#"
pub trait Drawable {
    fn draw(&self);
}
"#,
        );

        let drawable = result
            .symbols
            .iter()
            .find(|s| s.name == "Drawable")
            .unwrap();
        assert_eq!(drawable.kind, SymbolKind::Interface);
        assert_eq!(drawable.visibility, Visibility::Public);

        let draw = result.symbols.iter().find(|s| s.name == "draw").unwrap();
        assert_eq!(draw.kind, SymbolKind::Method);
        assert_eq!(draw.parent, Some(drawable.id));

        assert!(result.exports.iter().any(|e| e.exported_name == "Drawable"));
    }

    #[test]
    fn test_impl_block() {
        let result = parse_rust(
            r#"
struct Point { x: f64, y: f64 }

impl Point {
    fn distance(&self) -> f64 { 0.0 }
}
"#,
        );

        let point = result.symbols.iter().find(|s| s.name == "Point").unwrap();
        let distance = result
            .symbols
            .iter()
            .find(|s| s.name == "distance")
            .unwrap();
        assert_eq!(distance.kind, SymbolKind::Method);
        assert_eq!(distance.parent, Some(point.id));
    }

    #[test]
    fn test_use_simple() {
        let result = parse_rust("use crate::foo::Bar;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "crate::foo::Bar");
        assert_eq!(result.imports[0].imported_name, "Bar");
    }

    #[test]
    fn test_use_grouped() {
        let result = parse_rust("use crate::foo::{Bar, Baz};");
        assert_eq!(result.imports.len(), 2);
        let names: Vec<_> = result
            .imports
            .iter()
            .map(|i| i.imported_name.as_str())
            .collect();
        assert!(names.contains(&"Bar"));
        assert!(names.contains(&"Baz"));

        let paths: Vec<_> = result
            .imports
            .iter()
            .map(|i| i.source_path.as_str())
            .collect();
        assert!(paths.contains(&"crate::foo::Bar"));
        assert!(paths.contains(&"crate::foo::Baz"));
    }

    #[test]
    fn test_use_wildcard() {
        let result = parse_rust("use crate::foo::*;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "crate::foo");
        assert_eq!(result.imports[0].imported_name, "*");
        assert!(result.imports[0].is_namespace);
    }

    #[test]
    fn test_use_alias() {
        let result = parse_rust("use crate::foo::Bar as Baz;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "crate::foo::Bar");
        assert_eq!(result.imports[0].imported_name, "Bar");
        assert_eq!(result.imports[0].local_name, "Baz");
    }

    #[test]
    fn test_use_self() {
        let result = parse_rust("use crate::foo::{self, Bar};");
        assert_eq!(result.imports.len(), 2);

        let self_import = result
            .imports
            .iter()
            .find(|i| i.imported_name == "foo")
            .unwrap();
        assert_eq!(self_import.source_path, "crate::foo");

        let bar_import = result
            .imports
            .iter()
            .find(|i| i.imported_name == "Bar")
            .unwrap();
        assert_eq!(bar_import.source_path, "crate::foo::Bar");
    }

    #[test]
    fn test_use_super() {
        let result = parse_rust("use super::Bar;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "super::Bar");
        assert_eq!(result.imports[0].imported_name, "Bar");
    }

    #[test]
    fn test_mod_declaration() {
        let result = parse_rust("mod foo;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "@mod:foo");
        assert_eq!(result.imports[0].imported_name, "foo");
        assert!(result.imports[0].is_side_effect);

        let mod_sym = result.symbols.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(mod_sym.kind, SymbolKind::Module);
    }

    #[test]
    fn test_pub_use_reexport() {
        let result = parse_rust("pub use crate::model::User;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "crate::model::User");

        let reexports: Vec<_> = result.exports.iter().filter(|e| e.is_reexport).collect();
        assert_eq!(reexports.len(), 1);
        assert_eq!(reexports[0].exported_name, "User");
        assert!(reexports[0].is_reexport);
    }

    #[test]
    fn test_extern_crate() {
        let result = parse_rust("extern crate serde;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "extern::serde");
        assert_eq!(result.imports[0].imported_name, "serde");
    }

    #[test]
    fn test_extern_crate_with_alias() {
        let result = parse_rust("extern crate serde as s;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "extern::serde");
        assert_eq!(result.imports[0].imported_name, "serde");
        assert_eq!(result.imports[0].local_name, "s");
    }

    #[test]
    fn test_visibility_pub() {
        let result = parse_rust("pub fn public_fn() {}");
        assert_eq!(result.symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_visibility_pub_crate() {
        let result = parse_rust("pub(crate) fn crate_fn() {}");
        assert_eq!(result.symbols[0].visibility, Visibility::Protected);
    }

    #[test]
    fn test_visibility_private() {
        let result = parse_rust("fn private_fn() {}");
        assert_eq!(result.symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_visibility_pub_super() {
        let result = parse_rust("pub(super) fn parent_visible() {}");
        assert_eq!(result.symbols[0].visibility, Visibility::Protected);
    }

    #[test]
    fn test_qualified_names() {
        let result = parse_rust(
            r#"
struct Foo;

impl Foo {
    fn bar() {}
}
"#,
        );
        let foo = result.symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.qualified_name, "Foo");

        let bar = result.symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.qualified_name, "Foo::bar");
    }

    #[test]
    fn test_nested_mod() {
        let result = parse_rust(
            r#"
mod inner {
    pub fn baz() {}
}
"#,
        );

        let inner = result.symbols.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!(inner.kind, SymbolKind::Module);

        let baz = result.symbols.iter().find(|s| s.name == "baz").unwrap();
        assert_eq!(baz.qualified_name, "inner::baz");
        assert_eq!(baz.parent, Some(inner.id));
    }

    #[test]
    fn test_call_reference() {
        let result = parse_rust(
            r#"
fn helper() {}
fn main() {
    helper();
}
"#,
        );

        let main_fn = result.symbols.iter().find(|s| s.name == "main").unwrap();
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == main_fn.id && r.kind == RefKind::Call)
            .collect();
        assert!(!calls.is_empty(), "main should call helper");
    }

    #[test]
    fn test_struct_expression_reference() {
        let result = parse_rust(
            r#"
struct Point { x: i32, y: i32 }
fn create() {
    let p = Point { x: 1, y: 2 };
}
"#,
        );

        let create = result.symbols.iter().find(|s| s.name == "create").unwrap();
        let refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == create.id && r.kind == RefKind::Call)
            .collect();
        assert!(
            !refs.is_empty(),
            "create should reference Point struct expression"
        );
    }

    #[test]
    fn test_intra_file_call_resolved() {
        let result = parse_rust(
            r#"
fn helper() {}
fn main() {
    helper();
}
"#,
        );

        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
        let call_ref = result
            .references
            .iter()
            .find(|r| r.kind == RefKind::Call && r.target == helper.id);
        assert!(
            call_ref.is_some(),
            "call to helper should resolve intra-file"
        );
    }

    #[test]
    fn test_trait_impl_inheritance_ref() {
        let result = parse_rust(
            r#"
trait Drawable {
    fn draw(&self);
}

struct Circle;

impl Drawable for Circle {
    fn draw(&self) {}
}
"#,
        );

        let circle = result.symbols.iter().find(|s| s.name == "Circle").unwrap();
        let drawable = result
            .symbols
            .iter()
            .find(|s| s.name == "Drawable")
            .unwrap();

        let inheritance_ref = result
            .references
            .iter()
            .find(|r| r.kind == RefKind::Inheritance && r.source == circle.id);
        assert!(
            inheritance_ref.is_some(),
            "Circle should have inheritance ref to Drawable"
        );
        // Should be resolved intra-file
        assert_eq!(
            inheritance_ref.unwrap().target,
            drawable.id,
            "inheritance ref should resolve to Drawable"
        );
    }

    #[test]
    fn test_const_and_static() {
        let result = parse_rust(
            r#"
pub const MAX: u32 = 100;
pub static COUNTER: u32 = 0;
"#,
        );

        let max_const = result.symbols.iter().find(|s| s.name == "MAX").unwrap();
        assert_eq!(max_const.kind, SymbolKind::Constant);
        assert_eq!(max_const.visibility, Visibility::Public);

        let counter = result.symbols.iter().find(|s| s.name == "COUNTER").unwrap();
        assert_eq!(counter.kind, SymbolKind::Variable);
        assert_eq!(counter.visibility, Visibility::Public);

        assert!(result.exports.iter().any(|e| e.exported_name == "MAX"));
        assert!(result.exports.iter().any(|e| e.exported_name == "COUNTER"));
    }

    #[test]
    fn test_type_alias() {
        let result = parse_rust("pub type Result<T> = std::result::Result<T, Error>;");

        let sym = result.symbols.iter().find(|s| s.name == "Result").unwrap();
        assert_eq!(sym.kind, SymbolKind::TypeAlias);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(result.exports.iter().any(|e| e.exported_name == "Result"));
    }

    #[test]
    fn test_syntax_error_does_not_crash() {
        let result = parse_rust("pub fn { broken syntax }}}");
        let _ = result;
    }

    #[test]
    fn test_macro_definition() {
        let result = parse_rust(
            r#"
macro_rules! my_macro {
    () => {};
}
"#,
        );

        let mac = result
            .symbols
            .iter()
            .find(|s| s.name == "my_macro")
            .unwrap();
        assert_eq!(mac.kind, SymbolKind::Function);
        assert_eq!(mac.visibility, Visibility::Private);
    }

    #[test]
    fn test_pub_mod_declaration_exported() {
        let result = parse_rust("pub mod service;");
        assert!(
            result
                .exports
                .iter()
                .any(|e| e.exported_name == "service" && !e.is_reexport),
            "pub mod should be exported"
        );
    }

    #[test]
    fn test_use_nested_groups() {
        let result = parse_rust("use std::collections::{HashMap, HashSet};");
        assert_eq!(result.imports.len(), 2);

        let paths: Vec<_> = result
            .imports
            .iter()
            .map(|i| i.source_path.as_str())
            .collect();
        assert!(paths.contains(&"std::collections::HashMap"));
        assert!(paths.contains(&"std::collections::HashSet"));
    }

    #[test]
    fn test_pub_use_wildcard_reexport() {
        let result = parse_rust("pub use crate::model::*;");
        assert_eq!(result.imports.len(), 1);
        assert!(result.imports[0].is_namespace);

        let reexports: Vec<_> = result.exports.iter().filter(|e| e.is_reexport).collect();
        assert_eq!(reexports.len(), 1);
        assert_eq!(reexports[0].exported_name, "*");
    }

    #[test]
    fn test_function_signature() {
        let result = parse_rust("fn add(a: i32, b: i32) -> i32 { a + b }");
        let sig = result.symbols[0].signature.as_ref().unwrap();
        assert!(sig.contains("fn add"));
        assert!(sig.contains("i32"));
    }

    #[test]
    fn test_multiple_impl_blocks() {
        let result = parse_rust(
            r#"
struct Foo;

impl Foo {
    fn method_a(&self) {}
}

impl Foo {
    fn method_b(&self) {}
}
"#,
        );

        let foo = result.symbols.iter().find(|s| s.name == "Foo").unwrap();
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.parent == Some(foo.id) && s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn test_pub_crate_not_exported() {
        let result = parse_rust("pub(crate) fn internal() {}");
        // pub(crate) items are Protected, not exported at file scope
        assert!(
            result.exports.is_empty(),
            "pub(crate) functions should not be exported"
        );
    }

    #[test]
    fn test_inline_mod_symbols() {
        let result = parse_rust(
            r#"
mod tests {
    fn test_something() {}
}
"#,
        );

        let tests_mod = result.symbols.iter().find(|s| s.name == "tests").unwrap();
        assert_eq!(tests_mod.kind, SymbolKind::Module);

        let test_fn = result
            .symbols
            .iter()
            .find(|s| s.name == "test_something")
            .unwrap();
        assert_eq!(test_fn.parent, Some(tests_mod.id));
        assert_eq!(test_fn.qualified_name, "tests::test_something");

        // Inline mod should NOT generate @mod: import
        assert!(
            result
                .imports
                .iter()
                .all(|i| !i.source_path.starts_with("@mod:")),
            "inline mod should not generate @mod: import"
        );
    }

    #[test]
    fn test_path_attribute_does_not_crash() {
        let result = parse_rust(
            r#"
#[path = "platform/linux.rs"]
mod platform;
"#,
        );

        // Should still produce the mod symbol and @mod: import
        let platform = result
            .symbols
            .iter()
            .find(|s| s.name == "platform")
            .unwrap();
        assert_eq!(platform.kind, SymbolKind::Module);
        assert!(result
            .imports
            .iter()
            .any(|i| i.source_path == "@mod:platform"));
    }

    #[test]
    fn test_generic_function() {
        let result = parse_rust(
            r#"
pub fn convert<T: Clone>(input: T) -> T {
    input.clone()
}
"#,
        );

        let convert = result.symbols.iter().find(|s| s.name == "convert").unwrap();
        assert_eq!(convert.kind, SymbolKind::Function);
        assert!(convert.signature.as_ref().unwrap().contains("fn convert"));
    }

    #[test]
    fn test_trait_with_default_method() {
        let result = parse_rust(
            r#"
pub trait Greeting {
    fn hello(&self) -> String {
        "hello".to_string()
    }
    fn goodbye(&self);
}
"#,
        );

        let greeting = result
            .symbols
            .iter()
            .find(|s| s.name == "Greeting")
            .unwrap();
        assert_eq!(greeting.kind, SymbolKind::Interface);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.parent == Some(greeting.id))
            .collect();
        assert_eq!(
            methods.len(),
            2,
            "should extract both default and signature methods"
        );
    }

    #[test]
    fn test_type_reference_in_function() {
        let result = parse_rust(
            r#"
struct Config;
fn setup() -> Config {
    Config
}
"#,
        );

        let setup = result.symbols.iter().find(|s| s.name == "setup").unwrap();
        let type_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == setup.id && r.kind == RefKind::TypeUsage)
            .collect();
        assert!(
            !type_refs.is_empty(),
            "return type Config should create a TypeUsage reference"
        );
    }

    #[test]
    fn test_deeply_nested_use_groups() {
        let result = parse_rust("use std::{collections::{HashMap, HashSet}, fmt};");
        assert!(result.imports.len() >= 3);

        let paths: Vec<_> = result
            .imports
            .iter()
            .map(|i| i.source_path.as_str())
            .collect();
        assert!(
            paths.contains(&"std::collections::HashMap"),
            "paths: {:?}",
            paths
        );
        assert!(
            paths.contains(&"std::collections::HashSet"),
            "paths: {:?}",
            paths
        );
        assert!(paths.contains(&"std::fmt"), "paths: {:?}", paths);
    }

    #[test]
    fn test_async_function() {
        let result = parse_rust(
            r#"
pub async fn fetch_data() -> Vec<u8> {
    vec![]
}
"#,
        );

        let fetch = result
            .symbols
            .iter()
            .find(|s| s.name == "fetch_data")
            .unwrap();
        assert_eq!(fetch.kind, SymbolKind::Function);
        assert_eq!(fetch.visibility, Visibility::Public);
    }

    #[test]
    fn test_method_call_on_field() {
        let result = parse_rust(
            r#"
struct Foo;
impl Foo {
    fn do_thing(&self) {
        self.helper();
    }
    fn helper(&self) {}
}
"#,
        );

        let do_thing = result
            .symbols
            .iter()
            .find(|s| s.name == "do_thing")
            .unwrap();
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == do_thing.id && r.kind == RefKind::Call)
            .collect();
        assert!(!calls.is_empty(), "method call should be detected");
    }

    #[test]
    fn test_pub_in_path_visibility() {
        let result = parse_rust("pub(in crate::model) fn restricted() {}");
        assert_eq!(result.symbols[0].visibility, Visibility::Protected);
    }

    #[test]
    fn test_use_import_spans_nonzero() {
        let result = parse_rust("use crate::foo::Bar;");
        assert_eq!(result.imports.len(), 1);
        let import = &result.imports[0];
        assert!(
            import.span.start < import.span.end,
            "import span should be non-zero: {:?}",
            import.span
        );
        assert!(
            import.line_span.start.line > 0,
            "import line_span start line should be > 0: {:?}",
            import.line_span
        );
    }

    #[test]
    fn test_use_grouped_import_spans_nonzero() {
        let result = parse_rust("use crate::foo::{Bar, Baz};");
        assert_eq!(result.imports.len(), 2);
        for import in &result.imports {
            assert!(
                import.span.start < import.span.end,
                "grouped import span should be non-zero for {}: {:?}",
                import.imported_name,
                import.span
            );
        }
    }

    #[test]
    fn test_function_in_mod_is_function_not_method() {
        let result = parse_rust(
            r#"
mod inner {
    pub fn helper() {}
}
"#,
        );

        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(
            helper.kind,
            SymbolKind::Function,
            "function inside mod block should be Function, not Method"
        );
    }

    /// Regression test for bug #30: pub use reexport line numbers were 0.
    #[test]
    fn test_pub_use_reexport_has_nonzero_line() {
        let result = parse_rust(
            r#"
pub use crate::model::User;
"#,
        );

        let reexports: Vec<_> = result.exports.iter().filter(|e| e.is_reexport).collect();
        assert_eq!(reexports.len(), 1);
        assert!(
            reexports[0].line > 0,
            "reexport line should be > 0, got: {}",
            reexports[0].line
        );
    }

    /// Regression test for bug #30: all Rust export line numbers should be nonzero.
    #[test]
    fn test_export_line_numbers_nonzero() {
        let result = parse_rust(
            r#"
pub fn greet() {}
pub struct Config;
pub enum Mode { A, B }
pub trait Handler { fn handle(&self); }
pub const MAX: u32 = 100;
pub type Result<T> = std::result::Result<T, Error>;
"#,
        );

        for export in &result.exports {
            assert!(
                export.line > 0,
                "export '{}' should have line > 0, got: {}",
                export.exported_name, export.line
            );
        }
    }
}
