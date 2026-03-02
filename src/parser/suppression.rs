use std::collections::HashMap;

/// Extract `statik-ignore` suppression comments from source text.
///
/// Supported formats:
/// - `// statik-ignore[rule-id]` — suppress a specific rule on the next line
/// - `// statik-ignore` — suppress all rules on the next line
/// - `/* statik-ignore[rule-id] */` — block comment variant
///
/// Returns a map from 1-based line number to list of suppressed rule IDs.
/// An empty vec means "suppress all rules for that line".
pub fn extract_suppressions(source: &str) -> HashMap<usize, Vec<String>> {
    let mut suppressions: HashMap<usize, Vec<String>> = HashMap::new();

    for (idx, line) in source.lines().enumerate() {
        let line_number = idx + 1; // 1-based
        let trimmed = line.trim();

        // Try to find statik-ignore in line comments or block comments
        let ignore_pos = match trimmed.find("statik-ignore") {
            Some(pos) => pos,
            None => continue,
        };

        // Verify the prefix is a comment marker
        let prefix = trimmed[..ignore_pos].trim_end();
        if !is_comment_prefix(prefix) {
            continue;
        }

        // Extract rule IDs from the remainder after "statik-ignore"
        let after_ignore = &trimmed[ignore_pos + "statik-ignore".len()..];
        let rule_ids = parse_rule_ids(after_ignore);

        // Suppression applies to the next line
        let target_line = line_number + 1;
        let entry = suppressions.entry(target_line).or_default();
        if rule_ids.is_empty() {
            // Suppress all — only set empty if we don't already have specific rules
            // (empty vec means "suppress all")
            if entry.is_empty() {
                // Already empty = suppress all, no-op
            }
        } else {
            entry.extend(rule_ids);
        }
    }

    suppressions
}

/// Check whether a string prefix is a valid comment marker.
fn is_comment_prefix(prefix: &str) -> bool {
    // Line comments: //, #
    // Block comments: /*, or something ending with /*
    prefix == "//"
        || prefix == "#"
        || prefix == "/*"
        || prefix.ends_with("//")
        || prefix.ends_with("/*")
}

/// Parse rule IDs from the text after "statik-ignore".
///
/// Examples:
/// - `[rule-id]` -> `["rule-id"]`
/// - `[rule-a, rule-b]` -> `["rule-a", "rule-b"]`
/// - `` (empty) -> `[]` (suppress all)
/// - ` */` -> `[]` (suppress all, block comment ending)
fn parse_rule_ids(s: &str) -> Vec<String> {
    let s = s.trim();
    if !s.starts_with('[') {
        return Vec::new();
    }

    let bracket_end = match s.find(']') {
        Some(pos) => pos,
        None => return Vec::new(),
    };

    let inner = s[1..bracket_end].trim();
    if inner.is_empty() {
        return Vec::new();
    }

    inner
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_rule_line_comment() {
        let source = "// statik-ignore[no-ui-to-db]\nimport { db } from './db';\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec!["no-ui-to-db".to_string()]));
    }

    #[test]
    fn test_suppress_all_line_comment() {
        let source = "// statik-ignore\nimport { db } from './db';\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec![]));
    }

    #[test]
    fn test_single_rule_block_comment() {
        let source = "/* statik-ignore[no-ui-to-db] */\nimport { db } from './db';\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec!["no-ui-to-db".to_string()]));
    }

    #[test]
    fn test_suppress_all_block_comment() {
        let source = "/* statik-ignore */\nimport { db } from './db';\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec![]));
    }

    #[test]
    fn test_multiple_rules() {
        let source = "// statik-ignore[rule-a, rule-b]\nimport { x } from './x';\n";
        let result = extract_suppressions(source);
        assert_eq!(
            result.get(&2),
            Some(&vec!["rule-a".to_string(), "rule-b".to_string()])
        );
    }

    #[test]
    fn test_no_suppression_comments() {
        let source = "import { db } from './db';\nconst x = 1;\n";
        let result = extract_suppressions(source);
        assert!(result.is_empty());
    }

    #[test]
    fn test_not_a_comment() {
        // statik-ignore in a string literal should not be treated as a suppression
        let source = "const s = 'statik-ignore[rule]';\nconst x = 1;\n";
        let result = extract_suppressions(source);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rust_line_comment() {
        let source = "// statik-ignore[model-is-leaf]\nuse crate::resolver::TypeScriptResolver;\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec!["model-is-leaf".to_string()]));
    }

    #[test]
    fn test_java_line_comment() {
        let source = "// statik-ignore[no-cross-module]\nimport com.example.Foo;\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec!["no-cross-module".to_string()]));
    }

    #[test]
    fn test_indented_comment() {
        let source = "    // statik-ignore[rule-id]\n    import { x } from './x';\n";
        let result = extract_suppressions(source);
        assert_eq!(result.get(&2), Some(&vec!["rule-id".to_string()]));
    }

    #[test]
    fn test_applies_to_next_line() {
        let source = "line1\n// statik-ignore[rule-id]\nline3\nline4\n";
        let result = extract_suppressions(source);
        // Should apply to line 3 (the line after the comment on line 2)
        assert_eq!(result.get(&3), Some(&vec!["rule-id".to_string()]));
        assert!(result.get(&2).is_none());
        assert!(result.get(&4).is_none());
    }

    #[test]
    fn test_empty_brackets() {
        let source = "// statik-ignore[]\nimport { x } from './x';\n";
        let result = extract_suppressions(source);
        // Empty brackets = suppress all
        assert_eq!(result.get(&2), Some(&vec![]));
    }
}
