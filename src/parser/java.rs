use std::collections::HashSet;
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
        extractor.resolve_intra_file_refs();

        // Build set of explicitly imported names to avoid duplicating them as type-refs
        let explicit_imports: HashSet<String> = extractor
            .imports
            .iter()
            .filter(|i| !i.source_path.starts_with('@'))
            .map(|i| i.imported_name.clone())
            .collect();

        // Filter type_refs: remove explicitly imported names
        let type_references: Vec<String> = extractor
            .type_refs
            .iter()
            .filter(|name| !explicit_imports.contains(*name))
            .cloned()
            .collect();

        // Emit synthetic @type-ref: imports for same-package resolution
        let span = Span { start: 0, end: 0 };
        let line_span = LineSpan {
            start: Position { line: 0, column: 0 },
            end: Position { line: 0, column: 0 },
        };
        for name in &type_references {
            extractor.imports.push(ImportRecord {
                file: file_id,
                source_path: format!("@type-ref:{}", name),
                imported_name: name.clone(),
                local_name: String::new(),
                span,
                line_span,
                is_default: false,
                is_namespace: false,
                is_type_only: true,
                is_side_effect: false,
                is_dynamic: false,
            });
        }

        // Emit synthetic @annotation: imports for entry point detection
        for name in &extractor.annotations {
            extractor.imports.push(ImportRecord {
                file: file_id,
                source_path: format!("@annotation:{}", name),
                imported_name: name.clone(),
                local_name: String::new(),
                span,
                line_span,
                is_default: false,
                is_namespace: false,
                is_type_only: false,
                is_side_effect: false,
                is_dynamic: false,
            });
        }

        let annotations = extractor.annotations.clone();

        Ok(ParseResult {
            file_id,
            symbols: extractor.symbols,
            references: extractor.references,
            imports: extractor.imports,
            exports: extractor.exports,
            type_references,
            annotations,
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
    /// Collected type_identifier names for same-package resolution.
    type_refs: HashSet<String>,
    /// Generic type parameter names to exclude from type_refs.
    type_params: HashSet<String>,
    /// Annotation names on top-level declarations/methods for entry point detection.
    annotations: Vec<String>,
    /// Target names parallel to `references`, used for intra-file resolution post-pass.
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
            package_name: None,
            type_refs: HashSet::new(),
            type_params: HashSet::new(),
            annotations: Vec::new(),
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

    fn all_parents_public(&self) -> bool {
        self.parent_stack.iter().all(|&pid| {
            self.symbols
                .iter()
                .find(|s| s.id == pid)
                .is_some_and(|s| s.visibility == Visibility::Public)
        })
    }

    fn qualified_name(&self, name: &str) -> String {
        // If we have a parent symbol, qualify with parent name
        if let Some(parent_id) = self.current_parent() {
            if let Some(parent) = self.symbols.iter().find(|s| s.id == parent_id) {
                return format!("{}.{}", parent.qualified_name, name);
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
        // Second pass: extract symbols, references, imports, exports, type refs
        self.visit_children(root);
    }

    fn extract_package(&mut self, root: &Node) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "package_declaration" {
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    match inner_child.kind() {
                        "scoped_identifier" | "identifier" => {
                            self.package_name = Some(self.node_text(inner_child).to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn note_type_identifier(&mut self, node: Node) {
        let text = self.node_text(node);
        if !text.is_empty()
            && text != "var"
            && !self.type_params.contains(text)
            && text.chars().next().is_some_and(|c| c.is_uppercase())
        {
            self.type_refs.insert(text.to_string());
        }
    }

    fn collect_type_param_names(&mut self, type_params_node: Node) {
        let mut cursor = type_params_node.walk();
        for child in type_params_node.children(&mut cursor) {
            if child.kind() == "type_parameter" {
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "type_identifier" || inner_child.kind() == "identifier"
                    {
                        let name = self.node_text(inner_child).to_string();
                        if !name.is_empty() {
                            self.type_params.insert(name);
                        }
                        break;
                    }
                }
            }
        }
    }

    fn collect_annotation_names_on(&mut self, decl_node: Node) {
        if self.parent_stack.len() > 1 {
            return;
        }
        let mut cursor = decl_node.walk();
        for child in decl_node.children(&mut cursor) {
            match child.kind() {
                "modifiers" => {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "marker_annotation"
                            || inner_child.kind() == "annotation"
                        {
                            if let Some(name_n) = inner_child.child_by_field_name("name") {
                                self.annotations.push(self.node_text(name_n).to_string());
                            }
                        }
                    }
                }
                "marker_annotation" | "annotation" => {
                    if let Some(name_n) = child.child_by_field_name("name") {
                        self.annotations.push(self.node_text(name_n).to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// Walk all type_identifier nodes under a subtree, recording them as type refs.
    fn scan_type_identifiers(&mut self, node: Node) {
        if node.kind() == "type_identifier" {
            self.note_type_identifier(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_type_identifiers(child);
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
            "type_identifier" => {
                self.note_type_identifier(node);
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

    /// Extract annotation usages from a declaration node's modifiers and
    /// direct annotation children, emitting RefKind::TypeUsage references
    /// with the given symbol as the source.
    fn extract_annotations_on(&mut self, decl_node: Node, symbol_id: SymbolId) {
        let mut cursor = decl_node.walk();
        for child in decl_node.children(&mut cursor) {
            match child.kind() {
                "modifiers" => {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "marker_annotation"
                            || inner_child.kind() == "annotation"
                        {
                            self.emit_annotation_ref(inner_child, symbol_id);
                        }
                    }
                }
                "marker_annotation" | "annotation" => {
                    self.emit_annotation_ref(child, symbol_id);
                }
                _ => {}
            }
        }
    }

    fn emit_annotation_ref(&mut self, annotation_node: Node, source: SymbolId) {
        if let Some(name_n) = annotation_node.child_by_field_name("name") {
            let target_name = self.node_text(name_n).to_string();
            let ref_id = self.alloc_ref_id();
            let placeholder_target = SymbolId(u64::MAX - self.references.len() as u64);
            self.references.push(Reference {
                id: ref_id,
                source,
                target: placeholder_target,
                kind: RefKind::TypeUsage,
                file: self.file_id,
                span: self.node_span(name_n),
                line_span: self.node_line_span(name_n),
            });
            self.ref_target_names.push(target_name);
        }
    }

    fn extract_import(&mut self, node: Node) {
        let text = self.node_text(node).trim().to_string();
        let _is_static = text.contains("import static ");

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
            is_type_only: false, // Java has no type-only import distinction
            is_side_effect: false,
            is_dynamic: false,
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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if let Some(tp) = node.child_by_field_name("type_parameters") {
            self.collect_type_param_names(tp);
        }

        if let Some(superclass) = node.child_by_field_name("superclass") {
            self.extract_superclass(superclass, id);
            self.scan_type_identifiers(superclass);
        }

        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.extract_implements(interfaces, id);
            self.scan_type_identifiers(interfaces);
        }

        if visibility == Visibility::Public && self.all_parents_public() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
            });
        }

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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if let Some(tp) = node.child_by_field_name("type_parameters") {
            self.collect_type_param_names(tp);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "extends_interfaces" {
                self.extract_type_list_refs(child, id);
                self.scan_type_identifiers(child);
            }
        }

        if visibility == Visibility::Public && self.all_parents_public() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
            });
        }

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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.extract_implements(interfaces, id);
            self.scan_type_identifiers(interfaces);
        }

        if visibility == Visibility::Public && self.all_parents_public() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if visibility == Visibility::Public && self.all_parents_public() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if let Some(tp) = node.child_by_field_name("type_parameters") {
            self.collect_type_param_names(tp);
        }

        if visibility == Visibility::Public && self.all_parents_public() {
            self.exports.push(ExportRecord {
                file: self.file_id,
                symbol: id,
                exported_name: name,
                is_default: false,
                is_reexport: false,
                is_type_only: false,
                source_path: None,
                line: 0,
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
        self.extract_annotations_on(node, id);
        self.collect_annotation_names_on(node);

        if let Some(tp) = node.child_by_field_name("type_parameters") {
            self.collect_type_param_names(tp);
        }
        if let Some(ret) = node.child_by_field_name("type") {
            self.scan_type_identifiers(ret);
        }
        if let Some(params) = node.child_by_field_name("parameters") {
            self.scan_type_identifiers(params);
        }
        // throws clause
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "throws" {
                self.scan_type_identifiers(child);
            }
        }

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
        self.extract_annotations_on(node, id);

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
        let mut first_id = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = self.node_text(name_node).to_string();
                    let id = self.alloc_symbol_id();
                    if first_id.is_none() {
                        first_id = Some(id);
                    }
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

        // Scan field type for type references
        self.scan_type_identifiers(node);

        // Extract annotation references (e.g., @Autowired, @Inject)
        if let Some(id) = first_id {
            self.extract_annotations_on(node, id);
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
        let target_name = self.node_text(target_node).to_string();
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
        self.ref_target_names.push(target_name);
    }

    fn extract_call_reference(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        if name_node.is_none() {
            return;
        }
        let target_name = self.node_text(name_node.unwrap()).to_string();

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
            self.ref_target_names.push(target_name);
        }
    }

    fn extract_new_reference(&mut self, node: Node) {
        // `new ClassName(...)` - the type child is the class being constructed
        let target_name = node
            .child_by_field_name("type")
            .map(|n| self.node_text(n).to_string())
            .unwrap_or_else(|| self.node_text(node).to_string());

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
            self.ref_target_names.push(target_name);
        }
    }

    /// Post-pass: resolve placeholder reference targets to actual symbols defined in this file.
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
        assert!(result
            .exports
            .iter()
            .any(|e| e.exported_name == "Serializable"));
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
        assert!(result
            .exports
            .iter()
            .any(|e| e.exported_name == "MyAnnotation"));
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
        assert_eq!(bar.qualified_name, "com.example.Foo.bar");
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
        assert!(
            !calls.is_empty(),
            "new MyClass() should generate a Call reference"
        );
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
        assert!(result.imports.iter().any(|i| i.imported_name == "List"));
        assert!(result.imports.iter().any(|i| i.imported_name == "Map"));
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

        // Verify explicit imports (non-synthetic)
        let explicit: Vec<_> = result
            .imports
            .iter()
            .filter(|i| !i.source_path.starts_with('@'))
            .collect();
        assert_eq!(explicit.len(), 3);
        assert!(explicit.iter().any(|i| i.imported_name == "List"));
        assert!(explicit.iter().any(|i| i.imported_name == "Optional"));
        assert!(explicit.iter().any(|i| i.imported_name == "User"));

        // String appears as a type-ref (from method parameter)
        let type_refs: Vec<_> = result
            .imports
            .iter()
            .filter(|i| i.source_path.starts_with("@type-ref:"))
            .collect();
        assert!(
            type_refs.iter().any(|i| i.imported_name == "String"),
            "String should be extracted as type-ref"
        );

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
        assert!(result
            .exports
            .iter()
            .any(|e| e.exported_name == "UserService"));
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
        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
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
        assert!(
            sig.contains("greet"),
            "signature should contain method name: {}",
            sig
        );
        assert!(
            sig.contains("String"),
            "signature should contain return type: {}",
            sig
        );
    }

    #[test]
    fn test_package_info_file() {
        // package-info.java files contain just a package declaration and annotations.
        // The parser should handle them gracefully without crashing.
        let result = parse_java(
            r#"
/**
 * This package contains utility classes.
 */
@javax.annotation.ParametersAreNonnullByDefault
package com.example.util;
"#,
        );
        // Should produce no symbols or exports, just set the package name
        assert!(
            result.symbols.is_empty(),
            "package-info.java should produce no symbols"
        );
        assert!(
            result.exports.is_empty(),
            "package-info.java should produce no exports"
        );
    }

    #[test]
    fn test_annotation_usage_generates_type_reference() {
        let result = parse_java(
            r#"
public class AppTest {
    @Test
    public void testSomething() {}

    @Override
    public String toString() { return ""; }
}
"#,
        );
        let test_method = result
            .symbols
            .iter()
            .find(|s| s.name == "testSomething")
            .unwrap();
        let type_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::TypeUsage && r.source == test_method.id)
            .collect();
        assert!(
            !type_refs.is_empty(),
            "@Test annotation should generate a TypeUsage reference"
        );
    }

    #[test]
    fn test_spring_annotations_generate_references() {
        let result = parse_java(
            r#"
@SpringBootApplication
public class Application {
    @Autowired
    private UserService userService;

    @RequestMapping("/api")
    public void handle() {}
}
"#,
        );
        let app = result
            .symbols
            .iter()
            .find(|s| s.name == "Application")
            .unwrap();
        // @SpringBootApplication on the class should generate a TypeUsage ref
        let class_type_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::TypeUsage && r.source == app.id)
            .collect();
        assert!(
            !class_type_refs.is_empty(),
            "@SpringBootApplication should generate a TypeUsage reference on the class"
        );
    }

    #[test]
    fn test_record_declaration() {
        let result = parse_java(
            r#"
package com.example;

public record Point(int x, int y) {
    public double distance() {
        return Math.sqrt(x * x + y * y);
    }
}
"#,
        );

        // Record should be extracted as a Class symbol
        let point = result.symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);
        assert_eq!(point.visibility, Visibility::Public);
        assert_eq!(point.qualified_name, "com.example.Point");
        assert!(point.signature.as_ref().unwrap().contains("record"));

        // Method inside the record should be extracted
        let distance = result
            .symbols
            .iter()
            .find(|s| s.name == "distance")
            .unwrap();
        assert_eq!(distance.kind, SymbolKind::Method);
        assert_eq!(distance.parent, Some(point.id));

        // Public top-level record should be exported
        assert!(result.exports.iter().any(|e| e.exported_name == "Point"));
    }

    #[test]
    fn test_generic_superclass() {
        let result = parse_java(
            r#"
public class StringList extends ArrayList<String> {
}
"#,
        );
        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "StringList")
            .unwrap();
        let inheritance_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == RefKind::Inheritance && r.source == class.id)
            .collect();
        // Should resolve the base type `ArrayList` despite `<String>` generic args
        assert_eq!(
            inheritance_refs.len(),
            1,
            "generic superclass should produce exactly one inheritance reference"
        );
    }

    #[test]
    fn test_multiple_field_declarators() {
        let result = parse_java(
            r#"
public class Multi {
    private int x, y, z;
}
"#,
        );
        let fields: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert_eq!(
            fields.len(),
            3,
            "should extract all three declarators from `int x, y, z;`"
        );
        let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
        assert!(names.contains(&"z"));

        // All should be children of Multi
        let multi = result.symbols.iter().find(|s| s.name == "Multi").unwrap();
        for field in &fields {
            assert_eq!(field.parent, Some(multi.id));
        }
    }

    fn type_ref_names(result: &ParseResult) -> Vec<String> {
        result
            .imports
            .iter()
            .filter(|i| i.source_path.starts_with("@type-ref:"))
            .map(|i| i.imported_name.clone())
            .collect()
    }

    fn annotation_names(result: &ParseResult) -> Vec<String> {
        result
            .imports
            .iter()
            .filter(|i| i.source_path.starts_with("@annotation:"))
            .map(|i| i.imported_name.clone())
            .collect()
    }

    // =========================================================================
    // Type reference extraction
    // =========================================================================

    #[test]
    fn test_type_ref_field_types() {
        let result = parse_java(
            r#"
public class Foo {
    private Bar bar;
    private Baz baz;
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Bar".to_string()), "refs: {:?}", refs);
        assert!(refs.contains(&"Baz".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_return_types() {
        let result = parse_java(
            r#"
public class Foo {
    public Widget getWidget() { return null; }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_parameter_types() {
        let result = parse_java(
            r#"
public class Foo {
    public void process(Request req, Response res) {}
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Request".to_string()), "refs: {:?}", refs);
        assert!(refs.contains(&"Response".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_local_variables() {
        let result = parse_java(
            r#"
public class Foo {
    public void method() {
        Config cfg = null;
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Config".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_generics() {
        let result = parse_java(
            r#"
import java.util.List;
public class Foo {
    private List<Widget> widgets;
}
"#,
        );
        let refs = type_ref_names(&result);
        // Widget is a type_identifier inside generic_type
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
        // List is an explicit import, should not appear as type-ref
        assert!(
            !refs.contains(&"List".to_string()),
            "List should be filtered (explicit import): {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_cast_expression() {
        let result = parse_java(
            r#"
public class Foo {
    public void method(Object obj) {
        Widget w = (Widget) obj;
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_instanceof() {
        let result = parse_java(
            r#"
public class Foo {
    public boolean check(Object obj) {
        return obj instanceof Widget;
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_throws_clause() {
        let result = parse_java(
            r#"
public class Foo {
    public void risky() throws CustomException {}
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"CustomException".to_string()),
            "refs: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_filters_generic_type_params() {
        let result = parse_java(
            r#"
public class Container<T, E> {
    private T value;
    private E error;
    private Widget widget;
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            !refs.contains(&"T".to_string()),
            "generic type param T should be filtered: {:?}",
            refs
        );
        assert!(
            !refs.contains(&"E".to_string()),
            "generic type param E should be filtered: {:?}",
            refs
        );
        assert!(
            refs.contains(&"Widget".to_string()),
            "concrete type Widget should be extracted: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_filters_method_type_params() {
        let result = parse_java(
            r#"
public class Util {
    public <T> T convert(T input) { return input; }
    public Widget build() { return null; }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            !refs.contains(&"T".to_string()),
            "method type param T should be filtered: {:?}",
            refs
        );
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_no_primitives() {
        let result = parse_java(
            r#"
public class Primitives {
    private int x;
    private boolean flag;
    private double value;
    private void process() {}
    private Widget widget;
}
"#,
        );
        let refs = type_ref_names(&result);
        // Primitives use integral_type, boolean_type, etc., not type_identifier
        assert!(
            !refs
                .iter()
                .any(|r| r == "int" || r == "boolean" || r == "double" || r == "void"),
            "primitives should not appear as type refs: {:?}",
            refs
        );
        assert!(refs.contains(&"Widget".to_string()), "refs: {:?}", refs);
    }

    #[test]
    fn test_type_ref_skips_lowercase_identifiers() {
        // type_identifier nodes that are lowercase are likely not class names
        let result = parse_java(
            r#"
public class Foo {
    private Widget widget;
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(refs.contains(&"Widget".to_string()));
        // "widget" is an identifier, not a type_identifier
    }

    #[test]
    fn test_type_ref_deduplication() {
        let result = parse_java(
            r#"
public class Foo {
    private Widget a;
    private Widget b;
    public Widget getWidget() { return null; }
}
"#,
        );
        let refs = type_ref_names(&result);
        let widget_count = refs.iter().filter(|r| r.as_str() == "Widget").count();
        assert_eq!(
            widget_count, 1,
            "Widget should appear once (deduplicated): {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_skips_explicitly_imported() {
        let result = parse_java(
            r#"
import com.example.Widget;
public class Foo {
    private Widget widget;
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            !refs.contains(&"Widget".to_string()),
            "Widget is explicitly imported, should not be in type-refs: {:?}",
            refs
        );
    }

    // =========================================================================
    // Inner class exports
    // =========================================================================

    #[test]
    fn test_inner_class_public_exported() {
        let result = parse_java(
            r#"
public class Outer {
    public static class Inner {}
}
"#,
        );
        let names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(names.contains(&"Outer"), "exports: {:?}", names);
        assert!(
            names.contains(&"Inner"),
            "public static inner class should be exported: {:?}",
            names
        );
    }

    #[test]
    fn test_inner_class_private_not_exported() {
        let result = parse_java(
            r#"
public class Outer {
    private static class Secret {}
}
"#,
        );
        let names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(names.contains(&"Outer"));
        assert!(
            !names.contains(&"Secret"),
            "private inner class should not be exported: {:?}",
            names
        );
    }

    #[test]
    fn test_inner_class_in_private_parent_not_exported() {
        let result = parse_java(
            r#"
class PackagePrivate {
    public static class Inner {}
}
"#,
        );
        let names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(
            !names.contains(&"Inner"),
            "public inner in package-private parent should not be exported: {:?}",
            names
        );
    }

    #[test]
    fn test_inner_interface_exported() {
        let result = parse_java(
            r#"
public class Outer {
    public interface Callback {}
}
"#,
        );
        let names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(
            names.contains(&"Callback"),
            "public inner interface should be exported: {:?}",
            names
        );
    }

    #[test]
    fn test_inner_enum_exported() {
        let result = parse_java(
            r#"
public class Outer {
    public enum Status { ACTIVE, INACTIVE }
}
"#,
        );
        let names: Vec<_> = result
            .exports
            .iter()
            .map(|e| e.exported_name.as_str())
            .collect();
        assert!(
            names.contains(&"Status"),
            "public inner enum should be exported: {:?}",
            names
        );
    }

    // =========================================================================
    // Annotation collection
    // =========================================================================

    #[test]
    fn test_annotation_imports_top_level() {
        let result = parse_java(
            r#"
@SpringBootApplication
public class App {}
"#,
        );
        let anns = annotation_names(&result);
        assert!(
            anns.contains(&"SpringBootApplication".to_string()),
            "annotations: {:?}",
            anns
        );
    }

    #[test]
    fn test_annotation_imports_on_method_inside_class() {
        let result = parse_java(
            r#"
public class TestClass {
    @Test
    public void testMethod() {}
}
"#,
        );
        let anns = annotation_names(&result);
        assert!(
            anns.contains(&"Test".to_string()),
            "@Test on method should produce @annotation: import: {:?}",
            anns
        );
    }

    #[test]
    fn test_annotation_imports_not_for_deeply_nested() {
        let result = parse_java(
            r#"
public class Outer {
    public static class Inner {
        @Deprecated
        public void method() {}
    }
}
"#,
        );
        let anns = annotation_names(&result);
        // Annotations at depth 2+ should not be emitted
        assert!(
            !anns.contains(&"Deprecated".to_string()),
            "deeply nested annotation should not produce @annotation: import: {:?}",
            anns
        );
    }

    #[test]
    fn test_annotation_multiple() {
        let result = parse_java(
            r#"
@Configuration
@EnableAutoConfiguration
public class Config {}
"#,
        );
        let anns = annotation_names(&result);
        assert!(
            anns.contains(&"Configuration".to_string()),
            "anns: {:?}",
            anns
        );
        assert!(
            anns.contains(&"EnableAutoConfiguration".to_string()),
            "anns: {:?}",
            anns
        );
    }

    // =========================================================================
    // Qualified name separator
    // =========================================================================

    #[test]
    fn test_qualified_name_dot_separator() {
        let result = parse_java(
            r#"
package com.example;
public class Outer {
    public class Inner {
        public void method() {}
    }
}
"#,
        );
        let inner = result.symbols.iter().find(|s| s.name == "Inner").unwrap();
        assert_eq!(
            inner.qualified_name, "com.example.Outer.Inner",
            "inner class should use dot separator"
        );
        let method = result.symbols.iter().find(|s| s.name == "method").unwrap();
        assert_eq!(
            method.qualified_name, "com.example.Outer.Inner.method",
            "method in inner class should use dot separator"
        );
    }

    #[test]
    fn test_type_ref_is_type_only() {
        let result = parse_java(
            r#"
public class Foo {
    private Widget widget;
}
"#,
        );
        let type_ref_imports: Vec<_> = result
            .imports
            .iter()
            .filter(|i| i.source_path.starts_with("@type-ref:"))
            .collect();
        assert!(!type_ref_imports.is_empty());
        for import in &type_ref_imports {
            assert!(import.is_type_only, "type-ref imports should be type_only");
        }
    }

    // =========================================================================
    // Edge case tests for type reference extraction
    // =========================================================================

    #[test]
    fn test_type_ref_enhanced_for_loop() {
        let result = parse_java(
            r#"
import java.util.List;
public class Foo {
    public void process(List<User> users) {
        for (User u : users) {
            System.out.println(u);
        }
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"User".to_string()),
            "enhanced for loop variable type should be extracted: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_multi_catch() {
        let result = parse_java(
            r#"
public class Foo {
    public void risky() {
        try {
            doStuff();
        } catch (IOException | SQLException e) {
            e.printStackTrace();
        }
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"IOException".to_string()),
            "multi-catch first type should be extracted: {:?}",
            refs
        );
        assert!(
            refs.contains(&"SQLException".to_string()),
            "multi-catch second type should be extracted: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_try_with_resources() {
        let result = parse_java(
            r#"
public class Foo {
    public void read() {
        try (BufferedReader br = new BufferedReader(null)) {
            br.readLine();
        }
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"BufferedReader".to_string()),
            "try-with-resources type should be extracted: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_array_type() {
        let result = parse_java(
            r#"
public class Foo {
    private User[] users;
    public Widget[] getWidgets() { return null; }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"User".to_string()),
            "array element type should be extracted from field: {:?}",
            refs
        );
        assert!(
            refs.contains(&"Widget".to_string()),
            "array element type should be extracted from return type: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_no_self_reference() {
        // A class referencing its own name (factory pattern) should still
        // collect the type_ref, but when resolved at the graph level the
        // resolver should skip self-edges. At the parser level we just
        // verify the name is collected (it will be the class's own name).
        let result = parse_java(
            r#"
public class Widget {
    public static Widget create() {
        return new Widget();
    }
}
"#,
        );
        // Widget appears as both a declared class and a type ref (return type).
        // The parser collects it; self-edge filtering is the resolver's job.
        let refs = type_ref_names(&result);
        assert!(
            refs.contains(&"Widget".to_string()),
            "self-referencing type should still be collected as type-ref: {:?}",
            refs
        );
    }

    #[test]
    fn test_type_ref_var_not_extracted() {
        let result = parse_java(
            r#"
public class Foo {
    public void method() {
        var x = new User();
        var y = "hello";
    }
}
"#,
        );
        let refs = type_ref_names(&result);
        assert!(
            !refs.contains(&"var".to_string()),
            "var keyword should NOT be extracted as type-ref: {:?}",
            refs
        );
        // User still appears from the new expression's type_identifier
        assert!(
            refs.contains(&"User".to_string()),
            "User from new expression should be extracted: {:?}",
            refs
        );
    }

    #[test]
    fn test_intra_file_inheritance_resolved() {
        let result = parse_java(
            r#"
            package com.example;
            public class Base {}
            public class Derived extends Base {}
            "#,
        );

        let base_sym = result.symbols.iter().find(|s| s.name == "Base").unwrap();

        let inherit_ref = result
            .references
            .iter()
            .find(|r| r.kind == RefKind::Inheritance && r.target == base_sym.id);
        assert!(
            inherit_ref.is_some(),
            "Derived should have a resolved inheritance ref to Base, refs: {:?}",
            result
                .references
                .iter()
                .map(|r| (r.source, r.target, r.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_intra_file_call_reference_resolved() {
        let result = parse_java(
            r#"
            package com.example;
            public class MyClass {
                void helper() {}
                void main() { helper(); }
            }
            "#,
        );

        let helper_sym = result.symbols.iter().find(|s| s.name == "helper").unwrap();

        let call_ref = result
            .references
            .iter()
            .find(|r| r.kind == RefKind::Call && r.target == helper_sym.id);
        assert!(
            call_ref.is_some(),
            "main should have a resolved call ref to helper"
        );
    }
}
