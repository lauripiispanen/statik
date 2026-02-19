fn main() {
    // Test 1: enum members
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();

    let code = r#"enum Color { Red, Green, Blue }"#;
    let tree = parser.parse(code, None).unwrap();
    println!("=== ENUM ===");
    print_tree(tree.root_node(), code, 0);

    // Test 2: class extends
    let code2 = r#"class Dog extends Animal { bark() {} }"#;
    let tree2 = parser.parse(code2, None).unwrap();
    println!("\n=== CLASS EXTENDS ===");
    print_tree(tree2.root_node(), code2, 0);

    // Test 3: interface extends
    let code3 = r#"interface ClickableProps extends ComponentProps { onClick: () => void; }"#;
    let tree3 = parser.parse(code3, None).unwrap();
    println!("\n=== INTERFACE EXTENDS ===");
    print_tree(tree3.root_node(), code3, 0);

    // Test 4: implements
    let code4 = r#"class Dog implements Animal, Serializable { name: string = "Rex"; }"#;
    let tree4 = parser.parse(code4, None).unwrap();
    println!("\n=== CLASS IMPLEMENTS ===");
    print_tree(tree4.root_node(), code4, 0);

    // Test 5: namespace import
    let code5 = r#"import * as utils from './utils';"#;
    let tree5 = parser.parse(code5, None).unwrap();
    println!("\n=== NAMESPACE IMPORT ===");
    print_tree(tree5.root_node(), code5, 0);

    // Test 6: new expression
    let code6 = r#"function main() { const obj = new MyClass("arg"); }"#;
    let tree6 = parser.parse(code6, None).unwrap();
    println!("\n=== NEW EXPRESSION ===");
    print_tree(tree6.root_node(), code6, 0);
}

fn print_tree(node: tree_sitter::Node, source: &str, indent: usize) {
    let prefix = "  ".repeat(indent);
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");
    let short_text = if text.len() > 60 { &text[..60] } else { text };
    let field_name = node.parent().and_then(|p| {
        for i in 0..p.child_count() {
            if let Some(c) = p.child(i) {
                if c.id() == node.id() {
                    return p.field_name_for_child(i as u32);
                }
            }
        }
        None
    });
    let field = field_name
        .map(|n| format!(" [field: {}]", n))
        .unwrap_or_default();
    println!(
        "{}{}{}: {:?}",
        prefix,
        node.kind(),
        field,
        short_text.replace('\n', "\\n")
    );
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(child, source, indent + 1);
    }
}
