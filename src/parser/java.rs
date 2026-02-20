use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Tree};

use crate::model::{
    ExportRecord, FileId, ImportRecord, Language, LineSpan, ParseResult, Position, RefKind,
    Reference, ReferenceId, Span, Symbol, SymbolId, SymbolKind, Visibility,
};

use super::LanguageParser;

#[derive(Default)]
pub struct JavaParser;

impl JavaParser {
    pub fn new() -> Self {
        Self
    }

    fn create_parser() -> Result<Parser> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .context("failed to set Java parser language")?;
        Ok(parser)
    }
}

impl LanguageParser for JavaParser {
    fn parse(&self, file_id: FileId, source: &str, _path: &Path) -> Result<ParseResult> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(source, None)
            .context("tree-sitter failed to parse Java")?;

        let mut extractor = Extractor::new(file_id, source, &tree);
        extractor.extract();

        Ok(ParseResult {
            file_id,
            symbols: extractor.symbols,
            references: extractor.references,
            imports: extractor.imports,
            exports: extractor.exports,
        })
    }

    fn supported_languages(&self) -> &[Language] {
        &[Language::Java]
    }
}

/// Walks a Java tree-sitter CST and extracts symbols, references, imports, exports.
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
    /// The package name from the `package` declaration, if any.
    package_name: Option<String>,
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
            package_name: None,
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
        // If we have a parent symbol, qualify with parent name
        if let Some(parent_id) = self.current_parent() {
            if let Some(parent) = self.symbols.iter().find(|s| s.id == parent_id) {
                return format!("{}::{}", parent.qualified_name, name);
            }
        }
        // Otherwise, qualify with package name if available
        if let Some(ref pkg) = self.package_name {
            return format!("{}.{}", pkg, name);
        }
        name.to_string()
    }

    fn extract(&mut self) {
        let root = self.tree.root_node();
        // First pass: find the package declaration
        self.extract_package(&root);
        // Second pass: extract everything else
        self.visit_children(root);
    }

    fn extract_package(&mut self, root: &Node) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "package_declaration" {
                // Get the identifier or scoped_identifier child
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    match inner_child.kind() {
                        "scoped_identifier" | "identifier" => {
                            self.package_name =
                                Some(self.node_text(inner_child).to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node(child);
        }
    }

    fn visit_node(&mut self, node: Node) {
        match node.kind() {
            "import_declaration" => {
                self.extract_import(node);
            }
            "class_declaration" => {
                self.extract_class(node);
            }
            "interface_declaration" => {
                self.extract_interface(node);
            }
            "enum_declaration" => {
                self.extract_enum(node);
            }
            "annotation_type_declaration" => {
                self.extract_annotation_type(node);
            }
            "record_declaration" => {
                self.extract_record(node);
            }
            "method_invocation" => {
                self.extract_call_reference(node);
                self.visit_children(node);
            }
            "object_creation_expression" => {
                self.extract_new_reference(node);
                self.visit_children(node);
            }
            _ => {
                self.visit_children(node);
            }
        }
    }

    /// Extract modifiers from a declaration node.
    /// Returns (visibility, is_static, is_final).
    fn extract_modifiers(&self, node: Node) -> (Visibility, bool, bool) {
        let mut visibility = Visibility::Private; // Java default is package-private, mapped to Private
        let mut is_static = false;
        let mut is_final = false;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "modifiers" {
                let text = self.node_text(child);
                if text.contains("public") {
                    visibility = Visibility::Public;
                } else if text.contains("protected") {
                    visibility = Visibility::Protected;
                } else if text.contains("private") {
                    visibility = Visibility::Private;
                }
                if text.contains("static") {
                    is_static = true;
                }
                if text.contains("final") {
                    is_final = true;
                }
            }
        }

        (visibility, is_static, is_final)
    }

    fn extract_import(&mut self, node: Node) {
        let text = self.node_text(node).trim().to_string();
        let is_static = text.contains("import static ");

        // Find the actual import path from the AST
        let mut import_path = String::new();
        let mut is_wildcard = false;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "scoped_identifier" | "identifier" => {
                    import_path = self.node_text(child).to_string();
                }
                "asterisk" => {
                    is_wildcard = true;
                }
                _ => {}
            }
        }

        if import_path.is_empty() {
            return;
        }

        // For wildcard imports: `import com.example.*`
        // import_path = "com.example", is_wildcard = true
        // For regular: `import com.example.Foo`
        // imported_name = "Foo", source_path = "com.example.Foo"
        let (source_path, imported_name) = if is_wildcard {
            (import_path.clone(), "*".to_string())
        } else {
            let name = import_path
                .rsplit('.')
                .next()
                .unwrap_or(&import_path)
                .to_string();
            (import_path.clone(), name)
        };

        self.imports.push(ImportRecord {
            file: self.file_id,
            source_path,
            imported_name,
            local_name: String::new(),
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            is_default: false,
            is_namespace: is_wildcard,
            is_type_only: !is_static,
            is_side_effect: false,
        });
    }

    fn extract_class(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _is_static, _is_final) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Class,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(format!("class {}", name)),
        };
        self.symbols.push(symbol);

        // Extract extends (superclass)
        if let Some(superclass) = node.child_by_field_name("superclass") {
            self.extract_superclass(superclass, id);
        }

        // Extract implements (interfaces)
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.extract_implements(interfaces, id);
        }

        // Public top-level classes are exports
        if visibility == Visibility::Public && self.current_parent().is_none() {
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

        // Visit class body
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_class_body(body);
        }
        self.parent_stack.pop();
    }

    fn extract_interface(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _, _) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Interface,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(format!("interface {}", name)),
        };
        self.symbols.push(symbol);

        // Extract extends (for interfaces)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "extends_interfaces" {
                self.extract_type_list_refs(child, id);
            }
        }

        // Public top-level interfaces are exports
        if visibility == Visibility::Public && self.current_parent().is_none() {
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

        // Visit interface body
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_interface_body(body);
        }
        self.parent_stack.pop();
    }

    fn extract_enum(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _, _) = self.extract_modifiers(node);
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
            visibility,
            signature: Some(format!("enum {}", name)),
        };
        self.symbols.push(symbol);

        // Extract implements (interfaces) for enum
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.extract_implements(interfaces, id);
        }

        // Public top-level enums are exports
        if visibility == Visibility::Public && self.current_parent().is_none() {
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

        // Visit enum body for constants
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_enum_body(body);
        }
        self.parent_stack.pop();
    }

    fn extract_annotation_type(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _, _) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Annotation,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(format!("@interface {}", name)),
        };
        self.symbols.push(symbol);

        // Public top-level annotation types are exports
        if visibility == Visibility::Public && self.current_parent().is_none() {
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

    fn extract_record(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _, _) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name: name.clone(),
            kind: SymbolKind::Class,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(format!("record {}", name)),
        };
        self.symbols.push(symbol);

        // Public top-level records are exports
        if visibility == Visibility::Public && self.current_parent().is_none() {
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

        // Visit record body
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
                "method_declaration" => {
                    self.extract_method(child);
                }
                "constructor_declaration" => {
                    self.extract_constructor(child);
                }
                "field_declaration" => {
                    self.extract_field(child);
                }
                "class_declaration" => {
                    self.extract_class(child);
                }
                "interface_declaration" => {
                    self.extract_interface(child);
                }
                "enum_declaration" => {
                    self.extract_enum(child);
                }
                "annotation_type_declaration" => {
                    self.extract_annotation_type(child);
                }
                "record_declaration" => {
                    self.extract_record(child);
                }
                _ => {}
            }
        }
    }

    fn visit_interface_body(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "method_declaration" => {
                    self.extract_method(child);
                }
                "constant_declaration" => {
                    self.extract_constant_declaration(child);
                }
                "class_declaration" => {
                    self.extract_class(child);
                }
                "interface_declaration" => {
                    self.extract_interface(child);
                }
                "enum_declaration" => {
                    self.extract_enum(child);
                }
                _ => {}
            }
        }
    }

    fn visit_enum_body(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "enum_constant" => {
                    self.extract_enum_constant(child);
                }
                "enum_body_declarations" => {
                    // The enum body declarations section can contain methods, fields, etc.
                    self.visit_class_body(child);
                }
                _ => {}
            }
        }
    }

    fn extract_method(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _is_static, _is_final) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let return_type = node
            .child_by_field_name("type")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_default();
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_else(|| "()".to_string());

        let signature = if return_type.is_empty() {
            format!("{}{}", name, params)
        } else {
            format!("{} {}{}", return_type, name, params)
        };

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Method,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(signature),
        };
        self.symbols.push(symbol);

        // Visit method body for references
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.parent_stack.pop();
    }

    fn extract_constructor(&mut self, node: Node) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(n).to_string(),
            None => return,
        };

        let (visibility, _, _) = self.extract_modifiers(node);
        let id = self.alloc_symbol_id();

        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_else(|| "()".to_string());

        let symbol = Symbol {
            id,
            qualified_name: self.qualified_name(&name),
            name,
            kind: SymbolKind::Method,
            file: self.file_id,
            span: self.node_span(node),
            line_span: self.node_line_span(node),
            parent: self.current_parent(),
            visibility,
            signature: Some(params),
        };
        self.symbols.push(symbol);

        // Visit constructor body for references
        self.parent_stack.push(id);
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.parent_stack.pop();
    }

    fn extract_field(&mut self, node: Node) {
        let (visibility, is_static, is_final) = self.extract_modifiers(node);
        let kind = if is_static && is_final {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };

        // Fields can have multiple declarators: `int x, y;`
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = self.node_text(name_node).to_string();
                    let id = self.alloc_symbol_id();
                    self.symbols.push(Symbol {
                        id,
                        qualified_name: self.qualified_name(&name),
                        name,
                        kind,
                        file: self.file_id,
                        span: self.node_span(child),
                        line_span: self.node_line_span(child),
                        parent: self.current_parent(),
                        visibility,
                        signature: None,
                    });
                }
            }
        }
    }

    fn extract_constant_declaration(&mut self, node: Node) {
        // Interface constants (implicitly public static final)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = self.node_text(name_node).to_string();
                    let id = self.alloc_symbol_id();
                    self.symbols.push(Symbol {
                        id,
                        qualified_name: self.qualified_name(&name),
                        name,
                        kind: SymbolKind::Constant,
                        file: self.file_id,
                        span: self.node_span(child),
                        line_span: self.node_line_span(child),
                        parent: self.current_parent(),
                        visibility: Visibility::Public,
                        signature: None,
                    });
                }
            }
        }
    }

    fn extract_enum_constant(&mut self, node: Node) {
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

    fn extract_superclass(&mut self, superclass_node: Node, class_id: SymbolId) {
        // superclass field contains a _type node
        let mut cursor = superclass_node.walk();
        for child in superclass_node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "identifier" => {
                    self.add_inheritance_ref(class_id, child);
                }
                "generic_type" => {
                    // Generic superclass like `Foo<T>` - get the base type
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "type_identifier"
                            || inner_child.kind() == "identifier"
                        {
                            self.add_inheritance_ref(class_id, inner_child);
                            break;
                        }
                    }
                }
                "scoped_type_identifier" => {
                    // Qualified type like `com.example.Base`
                    self.add_inheritance_ref(class_id, child);
                }
                _ => {}
            }
        }
    }

    fn extract_implements(&mut self, implements_node: Node, symbol_id: SymbolId) {
        // super_interfaces / interfaces field contains a type_list
        self.extract_type_list_refs(implements_node, symbol_id);
    }

    fn extract_type_list_refs(&mut self, node: Node, symbol_id: SymbolId) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_list" => {
                    self.extract_type_list_refs(child, symbol_id);
                }
                "type_identifier" | "identifier" => {
                    self.add_inheritance_ref(symbol_id, child);
                }
                "generic_type" => {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "type_identifier"
                            || inner_child.kind() == "identifier"
                        {
                            self.add_inheritance_ref(symbol_id, inner_child);
                            break;
                        }
                    }
                }
                "scoped_type_identifier" => {
                    self.add_inheritance_ref(symbol_id, child);
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

    fn extract_call_reference(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        if name_node.is_none() {
            return;
        }

        if let Some(source_id) = self.current_parent() {
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
        }
    }

    fn extract_new_reference(&mut self, node: Node) {
        // `new ClassName(...)` - the type child is the class being constructed
        if let Some(source_id) = self.current_parent() {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_java(source: &str) -> ParseResult {
        let parser = JavaParser::new();
        parser
            .parse(FileId(1), source, Path::new("Test.java"))
            .unwrap()
    }

    #[test]
    fn test_empty_source() {
        let result = parse_java("");
        assert!(result.symbols.is_empty());
        assert!(result.references.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_package_declaration() {
        let result = parse_java(
            r#"
package com.example;
public class App {}
"#,
        );
        let app = result.symbols.iter().find(|s| s.name == "App").unwrap();
        assert_eq!(app.qualified_name, "com.example.App");
    }

    #[test]
    fn test_simple_class() {
        let result = parse_java("public class HelloWorld {}");
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "HelloWorld");
        assert_eq!(result.symbols[0].kind, SymbolKind::Class);
        assert_eq!(result.symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_public_class_is_exported() {
        let result = parse_java("public class App {}");
        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].exported_name, "App");
    }

    #[test]
    fn test_private_class_not_exported() {
        let result = parse_java("class PackagePrivate {}");
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_class_with_methods() {
        let result = parse_java(
            r#"
public class Animal {
    private String name;

    public void speak() {}
    private void run() {}
}
"#,
        );

        let animal = result.symbols.iter().find(|s| s.name == "Animal").unwrap();
        assert_eq!(animal.kind, SymbolKind::Class);

        let speak = result.symbols.iter().find(|s| s.name == "speak").unwrap();
        assert_eq!(speak.kind, SymbolKind::Method);
        assert_eq!(speak.parent, Some(animal.id));
        assert_eq!(speak.visibility, Visibility::Public);

        let run = result.symbols.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.kind, SymbolKind::Method);
        assert_eq!(run.visibility, Visibility::Private);

        let name = result.symbols.iter().find(|s| s.name == "name").unwrap();
        assert_eq!(name.kind, SymbolKind::Variable);
        assert_eq!(name.visibility, Visibility::Private);
    }

    #[test]
    fn test_interface() {
        let result = parse_java(
            r#"
public interface Serializable {
    byte[] serialize();
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

        // Public interface should be exported
        assert!(result.exports.iter().any(|e| e.exported_name == "Serializable"));
    }

    #[test]
    fn test_enum_with_constants() {
        let result = parse_java(
            r#"
public enum Color {
    RED,
    GREEN,
    BLUE;
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
        assert!(names.contains(&"RED"));
        assert!(names.contains(&"GREEN"));
        assert!(names.contains(&"BLUE"));
    }

    #[test]
    fn test_annotation_type() {
        let result = parse_java(
            r#"
public @interface MyAnnotation {
}
"#,
        );
        let ann = result
            .symbols
            .iter()
            .find(|s| s.name == "MyAnnotation")
            .unwrap();
        assert_eq!(ann.kind, SymbolKind::Annotation);
        assert_eq!(ann.visibility, Visibility::Public);
        assert!(result.exports.iter().any(|e| e.exported_name == "MyAnnotation"));
    }

    #[test]
    fn test_import_regular() {
        let result = parse_java("import com.example.Foo;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "com.example.Foo");
        assert_eq!(result.imports[0].imported_name, "Foo");
        assert!(!result.imports[0].is_namespace);
    }

    #[test]
    fn test_import_wildcard() {
        let result = parse_java("import com.example.*;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "com.example");
        assert_eq!(result.imports[0].imported_name, "*");
        assert!(result.imports[0].is_namespace);
    }

    #[test]
    fn test_import_static() {
        let result = parse_java("import static com.example.Foo.bar;");
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source_path, "com.example.Foo.bar");
        assert_eq!(result.imports[0].imported_name, "bar");
        assert!(!result.imports[0].is_type_only);
    }

    #[test]
    fn test_class_extends() {
        let result = parse_java(
            r#"
public class Dog extends Animal {
    public void bark() {}
}
"#,
        );
        let dog = result.symbols.iter().find(|s| s.name == "Dog").unwrap();
        let inheritance_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == dog.id)
            .collect();
        assert_eq!(inheritance_refs.len(), 1);
    }

    #[test]
    fn test_class_implements() {
        let result = parse_java(
            r#"
public class Dog implements Animal, Serializable {
}
"#,
        );
        let dog = result.symbols.iter().find(|s| s.name == "Dog").unwrap();
        let impl_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == dog.id)
            .collect();
        assert_eq!(impl_refs.len(), 2);
    }

    #[test]
    fn test_interface_extends() {
        let result = parse_java(
            r#"
public interface ClickHandler extends EventListener, Serializable {
}
"#,
        );
        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "ClickHandler")
            .unwrap();
        let ext_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == iface.id)
            .collect();
        assert_eq!(ext_refs.len(), 2);
    }

    #[test]
    fn test_constructor() {
        let result = parse_java(
            r#"
public class App {
    public App(String name) {}
}
"#,
        );
        let constructor = result
            .symbols
            .iter()
            .find(|s| s.name == "App" && s.kind == SymbolKind::Method)
            .unwrap();
        assert_eq!(constructor.kind, SymbolKind::Method);
        assert_eq!(constructor.visibility, Visibility::Public);

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "App" && s.kind == SymbolKind::Class)
            .unwrap();
        assert_eq!(constructor.parent, Some(class.id));
    }

    #[test]
    fn test_static_final_field_is_constant() {
        let result = parse_java(
            r#"
public class Config {
    public static final int MAX_SIZE = 100;
    private int count;
}
"#,
        );
        let max_size = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_SIZE")
            .unwrap();
        assert_eq!(max_size.kind, SymbolKind::Constant);

        let count = result.symbols.iter().find(|s| s.name == "count").unwrap();
        assert_eq!(count.kind, SymbolKind::Variable);
    }

    #[test]
    fn test_nested_class() {
        let result = parse_java(
            r#"
public class Outer {
    public class Inner {
        public void innerMethod() {}
    }
}
"#,
        );
        let outer = result.symbols.iter().find(|s| s.name == "Outer").unwrap();
        let inner = result.symbols.iter().find(|s| s.name == "Inner").unwrap();
        assert_eq!(inner.parent, Some(outer.id));

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "innerMethod")
            .unwrap();
        assert_eq!(method.parent, Some(inner.id));
    }

    #[test]
    fn test_qualified_names() {
        let result = parse_java(
            r#"
package com.example;
public class Foo {
    public void bar() {}
}
"#,
        );
        let foo = result.symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.qualified_name, "com.example.Foo");

        let bar = result.symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.qualified_name, "com.example.Foo::bar");
    }

    #[test]
    fn test_method_call_reference() {
        let result = parse_java(
            r#"
public class App {
    public void main() {
        helper();
        doStuff();
    }
}
"#,
        );
        let main_method = result.symbols.iter().find(|s| s.name == "main").unwrap();
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.source == main_method.id && r.kind == RefKind::Call)
            .collect();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_new_expression_reference() {
        let result = parse_java(
            r#"
public class App {
    public void create() {
        Object obj = new MyClass("arg");
    }
}
"#,
        );
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .collect();
        assert!(!calls.is_empty(), "new MyClass() should generate a Call reference");
    }

    #[test]
    fn test_multiple_imports() {
        let result = parse_java(
            r#"
import java.util.List;
import java.util.Map;
import static java.lang.Math.PI;
"#,
        );
        assert_eq!(result.imports.len(), 3);
        assert!(result
            .imports
            .iter()
            .any(|i| i.imported_name == "List"));
        assert!(result
            .imports
            .iter()
            .any(|i| i.imported_name == "Map"));
        assert!(result
            .imports
            .iter()
            .any(|i| i.imported_name == "PI" && !i.is_type_only));
    }

    #[test]
    fn test_enum_with_methods() {
        let result = parse_java(
            r#"
public enum Status {
    ACTIVE,
    INACTIVE;

    public boolean isActive() {
        return this == ACTIVE;
    }
}
"#,
        );
        let status = result.symbols.iter().find(|s| s.name == "Status").unwrap();
        assert_eq!(status.kind, SymbolKind::Enum);

        let is_active = result
            .symbols
            .iter()
            .find(|s| s.name == "isActive")
            .unwrap();
        assert_eq!(is_active.kind, SymbolKind::Method);
        assert_eq!(is_active.parent, Some(status.id));
    }

    #[test]
    fn test_parser_does_not_crash_on_syntax_errors() {
        let result = parse_java("public class { broken syntax }}}");
        let _ = result;
    }

    #[test]
    fn test_complex_real_world_file() {
        let result = parse_java(
            r#"
package com.example.service;

import java.util.List;
import java.util.Optional;
import com.example.model.User;

public interface UserService {
    Optional<User> getUser(String id);
    List<User> getAllUsers();
}
"#,
        );

        // Verify imports
        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.imported_name == "List"));
        assert!(result.imports.iter().any(|i| i.imported_name == "Optional"));
        assert!(result.imports.iter().any(|i| i.imported_name == "User"));

        // Verify interface
        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.qualified_name, "com.example.service.UserService");

        // Verify methods
        let get_user = result.symbols.iter().find(|s| s.name == "getUser").unwrap();
        assert_eq!(get_user.kind, SymbolKind::Method);
        assert_eq!(get_user.parent, Some(iface.id));

        // Verify export
        assert!(result.exports.iter().any(|e| e.exported_name == "UserService"));
    }

    #[test]
    fn test_class_extends_and_implements() {
        let result = parse_java(
            r#"
public class UserServiceImpl extends BaseService implements UserService, Loggable {
    public void getUser() {}
}
"#,
        );
        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserServiceImpl")
            .unwrap();
        let refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == class.id)
            .collect();
        // 1 extends + 2 implements = 3
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn test_protected_method() {
        let result = parse_java(
            r#"
public class Base {
    protected void helper() {}
}
"#,
        );
        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .unwrap();
        assert_eq!(helper.visibility, Visibility::Protected);
    }

    #[test]
    fn test_method_signature() {
        let result = parse_java(
            r#"
public class App {
    public String greet(String name) { return "hello " + name; }
}
"#,
        );
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert!(greet.signature.is_some());
        let sig = greet.signature.as_ref().unwrap();
        assert!(sig.contains("greet"), "signature should contain method name: {}", sig);
        assert!(sig.contains("String"), "signature should contain return type: {}", sig);
    }
}
