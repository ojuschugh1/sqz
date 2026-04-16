//! AST-level Delta Encoding — computes structural diffs between two versions
//! of source code using tree-sitter parse trees.
//!
//! Instead of line-level diffs, this module compares AST node structures to
//! produce a compact delta containing only the changed nodes and their tree
//! paths. This is more semantically meaningful than text diffs for code.
//!
//! # Example
//! ```
//! use sqz_engine::ast_delta::{ast_diff, encode_delta};
//!
//! let old = "fn foo() -> i32 { 1 }";
//! let new = "fn foo() -> i32 { 2 }";
//! let delta = ast_diff(old, new, "rust").unwrap();
//! let encoded = encode_delta(&delta);
//! assert!(!encoded.is_empty());
//! ```

use tree_sitter::{Language, Parser, Node};
use crate::error::{Result, SqzError};

/// A single change in the AST delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstChange {
    /// Path from root to the changed node (e.g., `["function_item", "block", "integer_literal"]`).
    pub path: Vec<String>,
    /// The kind of change.
    pub kind: ChangeKind,
    /// The old node text (empty for additions).
    pub old_text: String,
    /// The new node text (empty for deletions).
    pub new_text: String,
}

/// The kind of AST change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// A node was modified (text changed but kind is the same).
    Modified,
    /// A node was added in the new version.
    Added,
    /// A node was removed from the old version.
    Removed,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeKind::Modified => write!(f, "modified"),
            ChangeKind::Added => write!(f, "added"),
            ChangeKind::Removed => write!(f, "removed"),
        }
    }
}

/// A complete AST delta between two versions of code.
#[derive(Debug, Clone)]
pub struct AstDelta {
    /// The language used for parsing.
    pub language: String,
    /// The list of changes detected.
    pub changes: Vec<AstChange>,
    /// Number of nodes in the old tree.
    pub old_node_count: usize,
    /// Number of nodes in the new tree.
    pub new_node_count: usize,
}

impl AstDelta {
    /// Returns true if no changes were detected.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns the number of changes.
    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

/// Compute an AST-level diff between `old` and `new` source code.
///
/// Parses both versions with tree-sitter and compares the tree structures
/// node-by-node. Returns a compact delta containing only changed nodes.
///
/// # Arguments
/// * `old` — the original source code
/// * `new` — the modified source code
/// * `language` — one of `"rust"`, `"python"`, `"javascript"`
pub fn ast_diff(old: &str, new: &str, language: &str) -> Result<AstDelta> {
    let ts_lang = get_language(language)?;

    let old_tree = parse_source(old, &ts_lang)?;
    let new_tree = parse_source(new, &ts_lang)?;

    let old_root = old_tree.root_node();
    let new_root = new_tree.root_node();

    let old_node_count = count_nodes(&old_root);
    let new_node_count = count_nodes(&new_root);

    let mut changes = Vec::new();
    let path = Vec::new();
    diff_nodes(&old_root, old, &new_root, new, &path, &mut changes);

    Ok(AstDelta {
        language: language.to_string(),
        changes,
        old_node_count,
        new_node_count,
    })
}

/// Encode an `AstDelta` as a compact human-readable string.
///
/// Format:
/// ```text
/// [language] delta: N changes (old: M nodes, new: K nodes)
/// [modified] path/to/node: "old" -> "new"
/// [added] path/to/node: "new text"
/// [removed] path/to/node: "old text"
/// ```
pub fn encode_delta(delta: &AstDelta) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "[{}] delta: {} changes (old: {} nodes, new: {} nodes)\n",
        delta.language,
        delta.changes.len(),
        delta.old_node_count,
        delta.new_node_count,
    ));

    for change in &delta.changes {
        let path_str = change.path.join("/");
        match change.kind {
            ChangeKind::Modified => {
                let old_preview = truncate_preview(&change.old_text, 60);
                let new_preview = truncate_preview(&change.new_text, 60);
                out.push_str(&format!(
                    "  [modified] {path_str}: \"{old_preview}\" -> \"{new_preview}\"\n"
                ));
            }
            ChangeKind::Added => {
                let preview = truncate_preview(&change.new_text, 80);
                out.push_str(&format!("  [added] {path_str}: \"{preview}\"\n"));
            }
            ChangeKind::Removed => {
                let preview = truncate_preview(&change.old_text, 80);
                out.push_str(&format!("  [removed] {path_str}: \"{preview}\"\n"));
            }
        }
    }

    out
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Parse source code into a tree-sitter tree.
fn parse_source(source: &str, language: &Language) -> Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;
    parser
        .parse(source, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))
}

/// Resolve a language name to a tree-sitter `Language`.
fn get_language(language: &str) -> Result<Language> {
    match language {
        "rust" => Ok(tree_sitter_rust::language()),
        "python" => Ok(tree_sitter_python::language()),
        "javascript" | "js" => Ok(tree_sitter_javascript::language()),
        other => Err(SqzError::UnsupportedLanguage(other.to_string())),
    }
}

/// Count total named nodes in a tree.
fn count_nodes(node: &Node) -> usize {
    let mut count = 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            count += count_nodes(&child);
        }
    }
    count
}

/// Recursively diff two AST nodes and collect changes.
fn diff_nodes(
    old_node: &Node,
    old_src: &str,
    new_node: &Node,
    new_src: &str,
    path: &[String],
    changes: &mut Vec<AstChange>,
) {
    let old_text = &old_src[old_node.byte_range()];
    let new_text = &new_src[new_node.byte_range()];

    // If the text is identical, no changes in this subtree
    if old_text == new_text {
        return;
    }

    // If the node kinds differ, record as modified at this level
    if old_node.kind() != new_node.kind() {
        changes.push(AstChange {
            path: path.to_vec(),
            kind: ChangeKind::Modified,
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        });
        return;
    }

    // Same kind but different text — compare children
    let old_children: Vec<Node> = {
        let mut cursor = old_node.walk();
        old_node
            .children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    };
    let new_children: Vec<Node> = {
        let mut cursor = new_node.walk();
        new_node
            .children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    };

    let max_len = old_children.len().max(new_children.len());
    if max_len == 0 {
        // Leaf node with different text
        changes.push(AstChange {
            path: path.to_vec(),
            kind: ChangeKind::Modified,
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        });
        return;
    }

    for i in 0..max_len {
        match (old_children.get(i), new_children.get(i)) {
            (Some(old_child), Some(new_child)) => {
                let mut child_path = path.to_vec();
                child_path.push(format!("{}[{}]", old_child.kind(), i));
                diff_nodes(old_child, old_src, new_child, new_src, &child_path, changes);
            }
            (None, Some(new_child)) => {
                let mut child_path = path.to_vec();
                child_path.push(format!("{}[{}]", new_child.kind(), i));
                changes.push(AstChange {
                    path: child_path,
                    kind: ChangeKind::Added,
                    old_text: String::new(),
                    new_text: new_src[new_child.byte_range()].to_string(),
                });
            }
            (Some(old_child), None) => {
                let mut child_path = path.to_vec();
                child_path.push(format!("{}[{}]", old_child.kind(), i));
                changes.push(AstChange {
                    path: child_path,
                    kind: ChangeKind::Removed,
                    old_text: old_src[old_child.byte_range()].to_string(),
                    new_text: String::new(),
                });
            }
            (None, None) => unreachable!(),
        }
    }
}

/// Truncate a string for display, adding "…" if truncated.
fn truncate_preview(s: &str, max_len: usize) -> String {
    let single_line = s.replace('\n', "\\n");
    if single_line.len() <= max_len {
        single_line
    } else {
        format!("{}…", &single_line[..max_len])
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_code_no_changes() {
        let code = "fn foo() -> i32 { 42 }";
        let delta = ast_diff(code, code, "rust").unwrap();
        assert!(delta.is_empty(), "identical code should produce no changes");
    }

    #[test]
    fn test_modified_function_body() {
        let old = "fn foo() -> i32 { 1 }";
        let new = "fn foo() -> i32 { 2 }";
        let delta = ast_diff(old, new, "rust").unwrap();
        assert!(
            !delta.is_empty(),
            "changed function body should produce changes"
        );
        // Should detect the integer literal change
        let has_modified = delta
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Modified);
        assert!(has_modified, "should have a Modified change");
    }

    #[test]
    fn test_added_function() {
        let old = "fn foo() { }";
        let new = "fn foo() { }\nfn bar() { }";
        let delta = ast_diff(old, new, "rust").unwrap();
        assert!(!delta.is_empty(), "added function should produce changes");
    }

    #[test]
    fn test_encode_delta_format() {
        let old = "fn foo() -> i32 { 1 }";
        let new = "fn foo() -> i32 { 2 }";
        let delta = ast_diff(old, new, "rust").unwrap();
        let encoded = encode_delta(&delta);
        assert!(encoded.contains("[rust] delta:"));
        assert!(encoded.contains("changes"));
    }

    #[test]
    fn test_unsupported_language() {
        let result = ast_diff("code", "code", "cobol");
        assert!(matches!(result, Err(SqzError::UnsupportedLanguage(_))));
    }

    #[test]
    fn test_python_diff() {
        let old = "def greet(name):\n    return 'hello ' + name\n";
        let new = "def greet(name):\n    return 'hi ' + name\n";
        let delta = ast_diff(old, new, "python").unwrap();
        assert!(!delta.is_empty());
    }

    #[test]
    fn test_javascript_diff() {
        let old = "function add(a, b) { return a + b; }";
        let new = "function multiply(a, b) { return a * b; }";
        let delta = ast_diff(old, new, "javascript").unwrap();
        assert!(!delta.is_empty());
    }

    #[test]
    fn test_node_count() {
        let code = "fn foo() { let x = 1; let y = 2; }";
        let delta = ast_diff(code, code, "rust").unwrap();
        assert!(delta.old_node_count > 0);
        assert_eq!(delta.old_node_count, delta.new_node_count);
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Diffing identical code always produces zero changes.
        #[test]
        fn prop_identical_code_no_changes(
            n_fns in 1usize..=3,
            body in 1usize..=5,
        ) {
            let mut code = String::new();
            for i in 0..n_fns {
                code.push_str(&format!("fn func_{i}() -> i32 {{\n"));
                for j in 0..body {
                    code.push_str(&format!("    let _v{j} = {j};\n"));
                }
                code.push_str("    0\n}\n\n");
            }
            let delta = ast_diff(&code, &code, "rust").unwrap();
            prop_assert!(
                delta.is_empty(),
                "identical code should produce 0 changes, got {}",
                delta.len()
            );
        }

        /// Encoded delta is never empty for non-identical code.
        #[test]
        fn prop_encoded_delta_non_empty_for_changes(val_a in 1i32..100, val_b in 100i32..200) {
            let old = format!("fn foo() -> i32 {{ {val_a} }}");
            let new = format!("fn foo() -> i32 {{ {val_b} }}");
            let delta = ast_diff(&old, &new, "rust").unwrap();
            if !delta.is_empty() {
                let encoded = encode_delta(&delta);
                prop_assert!(!encoded.is_empty());
                prop_assert!(encoded.contains("[rust] delta:"));
            }
        }

        /// The number of changes is bounded by the total node count.
        #[test]
        fn prop_changes_bounded_by_nodes(val in 1i32..1000) {
            let old = format!("fn foo() -> i32 {{ {} }}", val);
            let new = format!("fn foo() -> i32 {{ {} }}", val + 1);
            let delta = ast_diff(&old, &new, "rust").unwrap();
            let max_nodes = delta.old_node_count.max(delta.new_node_count);
            prop_assert!(
                delta.len() <= max_nodes,
                "changes ({}) should be <= max nodes ({})",
                delta.len(),
                max_nodes
            );
        }
    }
}
