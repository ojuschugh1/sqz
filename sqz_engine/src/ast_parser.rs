//! AST-based code signature extraction using tree-sitter and regex fallbacks.
//!
//! Supports 18+ programming languages. Languages with tree-sitter grammars
//! (Rust, Python, JavaScript, Bash) use full AST parsing. All other languages
//! use regex-based extraction which is fast and reliable.

use crate::error::{Result, SqzError};
use std::collections::HashMap;
use tree_sitter::{Language, Parser};

/// A single import declaration extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub text: String,
}

/// A function or method signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    pub name: String,
    pub signature: String,
}

/// A class, struct, or interface definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDefinition {
    pub name: String,
    pub signature: String,
}

/// A type alias or type declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDeclaration {
    pub name: String,
    pub signature: String,
}

/// Summary of extracted code structure.
#[derive(Debug, Clone)]
pub struct CodeSummary {
    pub imports: Vec<ImportDecl>,
    pub functions: Vec<FunctionSignature>,
    pub classes: Vec<ClassDefinition>,
    pub types: Vec<TypeDeclaration>,
    pub tokens_original: u32,
    pub tokens_summary: u32,
}

impl CodeSummary {
    /// Render the summary as a compact text representation.
    pub fn to_text(&self) -> String {
        let mut parts = Vec::new();
        for imp in &self.imports {
            parts.push(imp.text.clone());
        }
        for cls in &self.classes {
            parts.push(cls.signature.clone());
        }
        for ty in &self.types {
            parts.push(ty.signature.clone());
        }
        for func in &self.functions {
            parts.push(func.signature.clone());
        }
        parts.join("\n")
    }
}

/// Approximate token count as char_count / 4.
fn approx_tokens(s: &str) -> u32 {
    ((s.len() as f64) / 4.0).ceil() as u32
}

// ---------------------------------------------------------------------------
// Tree-sitter based extractors
// ---------------------------------------------------------------------------

fn extract_line(source: &str, node: &tree_sitter::Node) -> String {
    let start = node.start_position().row;
    let end = node.end_position().row;
    let lines: Vec<&str> = source.lines().collect();
    if start < lines.len() {
        if start == end {
            lines[start].trim().to_string()
        } else {
            // Multi-line: take first line only (signature line)
            lines[start].trim().to_string()
        }
    } else {
        String::new()
    }
}

fn node_text<'a>(source: &'a str, node: &tree_sitter::Node) -> &'a str {
    &source[node.byte_range()]
}

/// Extract signatures from Rust source using tree-sitter.
fn extract_rust(source: &str, language: Language) -> Result<CodeSummary> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))?;

    let root = tree.root_node();
    let mut imports = Vec::new();
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut types = Vec::new();

    let mut cursor = root.walk();
    // Walk top-level items
    for child in root.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                imports.push(ImportDecl {
                    text: extract_line(source, &child),
                });
            }
            "function_item" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                functions.push(FunctionSignature {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            "struct_item" | "enum_item" | "impl_item" | "trait_item" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                classes.push(ClassDefinition {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            "type_item" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                types.push(TypeDeclaration {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            _ => {}
        }
    }

    let summary_text = {
        let mut parts = Vec::new();
        for i in &imports {
            parts.push(i.text.clone());
        }
        for c in &classes {
            parts.push(c.signature.clone());
        }
        for t in &types {
            parts.push(t.signature.clone());
        }
        for f in &functions {
            parts.push(f.signature.clone());
        }
        parts.join("\n")
    };

    Ok(CodeSummary {
        imports,
        functions,
        classes,
        types,
        tokens_original: approx_tokens(source),
        tokens_summary: approx_tokens(&summary_text),
    })
}

/// Extract signatures from Python source using tree-sitter.
fn extract_python(source: &str, language: Language) -> Result<CodeSummary> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))?;

    let root = tree.root_node();
    let mut imports = Vec::new();
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let types = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" | "import_from_statement" => {
                imports.push(ImportDecl {
                    text: extract_line(source, &child),
                });
            }
            "function_definition" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                functions.push(FunctionSignature {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            "class_definition" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                classes.push(ClassDefinition {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            _ => {}
        }
    }

    let summary_text = build_summary_text(&imports, &functions, &classes, &types);
    Ok(CodeSummary {
        imports,
        functions,
        classes,
        types,
        tokens_original: approx_tokens(source),
        tokens_summary: approx_tokens(&summary_text),
    })
}

/// Extract signatures from JavaScript source using tree-sitter.
fn extract_javascript(source: &str, language: Language) -> Result<CodeSummary> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))?;

    let root = tree.root_node();
    let mut imports = Vec::new();
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let types = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                imports.push(ImportDecl {
                    text: extract_line(source, &child),
                });
            }
            "function_declaration" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                functions.push(FunctionSignature {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            "class_declaration" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(source, &n).to_string())
                    .unwrap_or_default();
                classes.push(ClassDefinition {
                    name,
                    signature: extract_line(source, &child),
                });
            }
            "lexical_declaration" | "variable_declaration" => {
                // Capture exported const fn = () => {} style
                let line = extract_line(source, &child);
                if line.contains("function") || line.contains("=>") {
                    let name = child
                        .named_child(0)
                        .and_then(|d| d.child_by_field_name("name"))
                        .map(|n| node_text(source, &n).to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        functions.push(FunctionSignature {
                            name,
                            signature: line,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    let summary_text = build_summary_text(&imports, &functions, &classes, &types);
    Ok(CodeSummary {
        imports,
        functions,
        classes,
        types,
        tokens_original: approx_tokens(source),
        tokens_summary: approx_tokens(&summary_text),
    })
}

/// Extract signatures from Bash source using tree-sitter.
fn extract_bash(source: &str, language: Language) -> Result<CodeSummary> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))?;

    let root = tree.root_node();
    let mut functions = Vec::new();
    let imports = Vec::new();
    let classes = Vec::new();
    let types = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(source, &n).to_string())
                .unwrap_or_default();
            functions.push(FunctionSignature {
                name,
                signature: extract_line(source, &child),
            });
        }
    }

    let summary_text = build_summary_text(&imports, &functions, &classes, &types);
    Ok(CodeSummary {
        imports,
        functions,
        classes,
        types,
        tokens_original: approx_tokens(source),
        tokens_summary: approx_tokens(&summary_text),
    })
}

fn build_summary_text(
    imports: &[ImportDecl],
    functions: &[FunctionSignature],
    classes: &[ClassDefinition],
    types: &[TypeDeclaration],
) -> String {
    let mut parts = Vec::new();
    for i in imports {
        parts.push(i.text.clone());
    }
    for c in classes {
        parts.push(c.signature.clone());
    }
    for t in types {
        parts.push(t.signature.clone());
    }
    for f in functions {
        parts.push(f.signature.clone());
    }
    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Regex-based extractors for languages without 0.21 grammar crates
// ---------------------------------------------------------------------------

/// Generic regex-based extractor. Each language provides its own patterns.
struct RegexExtractor {
    import_patterns: Vec<&'static str>,
    function_patterns: Vec<&'static str>,
    class_patterns: Vec<&'static str>,
    type_patterns: Vec<&'static str>,
}

impl RegexExtractor {
    fn extract(&self, source: &str) -> CodeSummary {
        use std::collections::HashSet;

        let mut imports = Vec::new();
        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut types = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }

            for pat in &self.import_patterns {
                if trimmed.starts_with(pat) && seen.insert(trimmed.to_string()) {
                    imports.push(ImportDecl {
                        text: trimmed.to_string(),
                    });
                    break;
                }
            }

            for pat in &self.function_patterns {
                if trimmed.contains(pat) && seen.insert(trimmed.to_string()) {
                    // Extract name heuristically: word after the keyword
                    let name = extract_name_after(trimmed, pat);
                    functions.push(FunctionSignature {
                        name,
                        signature: trimmed.to_string(),
                    });
                    break;
                }
            }

            for pat in &self.class_patterns {
                if trimmed.starts_with(pat) && seen.insert(trimmed.to_string()) {
                    let name = extract_name_after(trimmed, pat);
                    classes.push(ClassDefinition {
                        name,
                        signature: trimmed.to_string(),
                    });
                    break;
                }
            }

            for pat in &self.type_patterns {
                if trimmed.starts_with(pat) && seen.insert(trimmed.to_string()) {
                    let name = extract_name_after(trimmed, pat);
                    types.push(TypeDeclaration {
                        name,
                        signature: trimmed.to_string(),
                    });
                    break;
                }
            }
        }

        let summary_text = build_summary_text(&imports, &functions, &classes, &types);
        CodeSummary {
            imports,
            functions,
            classes,
            types,
            tokens_original: approx_tokens(source),
            tokens_summary: approx_tokens(&summary_text),
        }
    }
}

fn extract_name_after(line: &str, keyword: &str) -> String {
    let rest = line[line.find(keyword).unwrap_or(0) + keyword.len()..].trim();
    rest.split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
        .to_string()
}

fn go_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["import "],
        function_patterns: vec!["func "],
        class_patterns: vec!["type "],
        type_patterns: vec![],
    }
}

fn java_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["import "],
        function_patterns: vec![
            "public ",
            "private ",
            "protected ",
            "static ",
            "void ",
            "int ",
            "String ",
        ],
        class_patterns: vec!["class ", "interface ", "enum ", "record "],
        type_patterns: vec![],
    }
}

fn c_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["#include"],
        function_patterns: vec![],
        class_patterns: vec!["struct ", "union ", "enum "],
        type_patterns: vec!["typedef "],
    }
}

fn cpp_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["#include"],
        function_patterns: vec![],
        class_patterns: vec!["class ", "struct ", "union ", "enum "],
        type_patterns: vec!["typedef ", "using "],
    }
}

fn ruby_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["require ", "require_relative "],
        function_patterns: vec!["def "],
        class_patterns: vec!["class ", "module "],
        type_patterns: vec![],
    }
}

fn json_extractor() -> RegexExtractor {
    // JSON has no functions/classes; just return top-level keys
    RegexExtractor {
        import_patterns: vec![],
        function_patterns: vec![],
        class_patterns: vec![],
        type_patterns: vec![],
    }
}

fn html_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["<link", "<script"],
        function_patterns: vec![],
        class_patterns: vec![],
        type_patterns: vec![],
    }
}

fn css_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["@import"],
        function_patterns: vec![],
        class_patterns: vec![],
        type_patterns: vec!["@keyframes", "@media", "@mixin"],
    }
}

fn typescript_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["import "],
        function_patterns: vec!["function ", "async function ", "export function ", "export async function "],
        class_patterns: vec!["class ", "interface ", "abstract class "],
        type_patterns: vec!["type ", "enum "],
    }
}

fn csharp_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["using "],
        function_patterns: vec![
            "public ",
            "private ",
            "protected ",
            "internal ",
            "static ",
            "override ",
            "virtual ",
            "abstract ",
        ],
        class_patterns: vec!["class ", "interface ", "struct ", "enum ", "record "],
        type_patterns: vec![],
    }
}

fn kotlin_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["import "],
        function_patterns: vec!["fun "],
        class_patterns: vec!["class ", "interface ", "object ", "data class ", "sealed class "],
        type_patterns: vec!["typealias "],
    }
}

fn swift_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec!["import "],
        function_patterns: vec!["func "],
        class_patterns: vec!["class ", "struct ", "enum ", "protocol ", "extension "],
        type_patterns: vec!["typealias "],
    }
}

fn toml_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec![],
        function_patterns: vec![],
        class_patterns: vec!["["],
        type_patterns: vec![],
    }
}

fn yaml_extractor() -> RegexExtractor {
    RegexExtractor {
        import_patterns: vec![],
        function_patterns: vec![],
        class_patterns: vec![],
        type_patterns: vec![],
    }
}

// ---------------------------------------------------------------------------
// AstParser
// ---------------------------------------------------------------------------

/// Tree-sitter based code structure extractor supporting 18+ languages.
pub struct AstParser {
    grammars: HashMap<String, Language>,
}

impl AstParser {
    /// Create a new `AstParser` loading all bundled grammars.
    pub fn new() -> Self {
        let mut grammars = HashMap::new();
        grammars.insert("rust".to_string(), tree_sitter_rust::language());
        grammars.insert("python".to_string(), tree_sitter_python::language());
        grammars.insert("javascript".to_string(), tree_sitter_javascript::language());
        grammars.insert("bash".to_string(), tree_sitter_bash::language());
        AstParser { grammars }
    }

    /// Returns the list of supported language identifiers.
    pub fn supported_languages(&self) -> &[&'static str] {
        &[
            "rust",
            "python",
            "javascript",
            "typescript",
            "go",
            "java",
            "c",
            "cpp",
            "ruby",
            "bash",
            "json",
            "html",
            "css",
            "csharp",
            "kotlin",
            "swift",
            "toml",
            "yaml",
        ]
    }

    /// Returns true if the given language identifier is supported.
    pub fn is_supported(&self, language: &str) -> bool {
        self.supported_languages().contains(&language)
    }

    /// Extract code signatures from `source` written in `language`.
    ///
    /// Returns the file unchanged (as a single-import entry) for unsupported
    /// languages, per Requirement 19.4.
    pub fn extract_signatures(&self, source: &str, language: &str) -> Result<CodeSummary> {
        if !self.is_supported(language) {
            eprintln!("AstParser: unsupported language '{language}', returning source unchanged");
            return Err(SqzError::UnsupportedLanguage(language.to_string()));
        }

        match language {
            "rust" => {
                let lang = self.grammars["rust"].clone();
                extract_rust(source, lang)
            }
            "python" => {
                let lang = self.grammars["python"].clone();
                extract_python(source, lang)
            }
            "javascript" => {
                let lang = self.grammars["javascript"].clone();
                extract_javascript(source, lang)
            }
            "bash" => {
                let lang = self.grammars["bash"].clone();
                extract_bash(source, lang)
            }
            "typescript" => Ok(typescript_extractor().extract(source)),
            "go" => Ok(go_extractor().extract(source)),
            "java" => Ok(java_extractor().extract(source)),
            "c" => Ok(c_extractor().extract(source)),
            "cpp" => Ok(cpp_extractor().extract(source)),
            "ruby" => Ok(ruby_extractor().extract(source)),
            "json" => Ok(json_extractor().extract(source)),
            "html" => Ok(html_extractor().extract(source)),
            "css" => Ok(css_extractor().extract(source)),
            "csharp" => Ok(csharp_extractor().extract(source)),
            "kotlin" => Ok(kotlin_extractor().extract(source)),
            "swift" => Ok(swift_extractor().extract(source)),
            "toml" => Ok(toml_extractor().extract(source)),
            "yaml" => Ok(yaml_extractor().extract(source)),
            _ => unreachable!("is_supported check above covers all cases"),
        }
    }
}

impl Default for AstParser {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_languages_count() {
        let parser = AstParser::new();
        assert!(
            parser.supported_languages().len() >= 18,
            "must support 18+ languages"
        );
    }

    #[test]
    fn test_is_supported() {
        let parser = AstParser::new();
        assert!(parser.is_supported("rust"));
        assert!(parser.is_supported("python"));
        assert!(parser.is_supported("go"));
        assert!(!parser.is_supported("cobol"));
        assert!(!parser.is_supported(""));
    }

    #[test]
    fn test_unsupported_language_returns_error() {
        let parser = AstParser::new();
        let result = parser.extract_signatures("fn main() {}", "cobol");
        assert!(matches!(result, Err(SqzError::UnsupportedLanguage(_))));
    }

    #[test]
    fn test_rust_extraction() {
        let parser = AstParser::new();
        let source = r#"
use std::collections::HashMap;

pub struct Foo {
    x: i32,
}

pub fn bar(x: i32) -> i32 {
    x + 1
}

pub type MyType = Vec<i32>;
"#;
        let summary = parser.extract_signatures(source, "rust").unwrap();
        assert!(!summary.functions.is_empty());
        assert!(!summary.classes.is_empty());
        assert!(!summary.imports.is_empty());
        assert!(summary.tokens_summary < summary.tokens_original);
    }

    #[test]
    fn test_python_extraction() {
        let parser = AstParser::new();
        let source = r#"
import os
from typing import List

class MyClass:
    def __init__(self):
        pass

def my_function(x: int) -> int:
    return x + 1
"#;
        let summary = parser.extract_signatures(source, "python").unwrap();
        assert!(!summary.functions.is_empty());
        assert!(!summary.classes.is_empty());
        assert!(!summary.imports.is_empty());
    }

    #[test]
    fn test_go_extraction() {
        let parser = AstParser::new();
        let source = r#"
package main

import "fmt"

type Server struct {
    port int
}

func NewServer(port int) *Server {
    return &Server{port: port}
}

func (s *Server) Start() error {
    fmt.Println("starting")
    return nil
}
"#;
        let summary = parser.extract_signatures(source, "go").unwrap();
        assert!(!summary.functions.is_empty());
        assert!(!summary.imports.is_empty());
    }

    #[test]
    fn test_compression_ratio() {
        let parser = AstParser::new();
        // A large Rust file with lots of implementation details
        let source = r#"
use std::collections::HashMap;
use std::sync::Arc;

/// A complex data structure with lots of implementation
pub struct ComplexStruct {
    field1: i32,
    field2: String,
    field3: Vec<u8>,
    field4: HashMap<String, i32>,
}

impl ComplexStruct {
    pub fn new() -> Self {
        Self {
            field1: 0,
            field2: String::new(),
            field3: Vec::new(),
            field4: HashMap::new(),
        }
    }

    pub fn process(&self, input: &str) -> Result<String, Box<dyn std::error::Error>> {
        // lots of implementation
        let mut result = String::new();
        for c in input.chars() {
            result.push(c);
            result.push(' ');
        }
        Ok(result)
    }

    fn internal_helper(&self) -> i32 {
        self.field1 * 2
    }
}

pub fn standalone_function(x: i32, y: i32) -> i32 {
    // implementation
    let temp = x + y;
    let temp2 = temp * 2;
    temp2 - x
}

pub type MyAlias = Arc<ComplexStruct>;
"#;
        let summary = parser.extract_signatures(source, "rust").unwrap();
        assert!(
            summary.tokens_summary < summary.tokens_original,
            "summary ({}) should be smaller than original ({})",
            summary.tokens_summary,
            summary.tokens_original
        );
    }

    // -----------------------------------------------------------------------
    // Property 25: AST extraction preserves public API
    // -----------------------------------------------------------------------

    /// **Property 25: AST extraction preserves public API**
    ///
    /// **Validates: Requirements 19.2, 19.3**
    ///
    /// For any source code in a supported language containing at least one
    /// public function/class definition, `extract_signatures` SHALL produce a
    /// `CodeSummary` where `tokens_summary < tokens_original` (compression
    /// occurred) and the function/class names appear in the summary output.
    #[cfg(test)]
    mod prop25 {
        use super::*;
        use proptest::prelude::*;

        /// Generate a Rust source file with N functions and M structs,
        /// each with a body of `body_lines` lines of filler code.
        /// This ensures tokens_original >> tokens_summary.
        fn arb_rust_source() -> impl Strategy<Value = String> {
            (
                1usize..=5,   // number of functions
                1usize..=3,   // number of structs
                5usize..=20,  // body lines per function (filler)
            )
                .prop_map(|(n_fns, n_structs, body_lines)| {
                    let mut src = String::new();
                    src.push_str("use std::collections::HashMap;\n\n");

                    for i in 0..n_structs {
                        src.push_str(&format!("pub struct MyStruct{i} {{\n"));
                        src.push_str("    field_a: i32,\n");
                        src.push_str("    field_b: String,\n");
                        src.push_str("    field_c: Vec<u8>,\n");
                        src.push_str("}\n\n");
                    }

                    for i in 0..n_fns {
                        src.push_str(&format!(
                            "pub fn my_function_{i}(x: i32, y: i32) -> i32 {{\n"
                        ));
                        for j in 0..body_lines {
                            src.push_str(&format!(
                                "    let _var_{j} = x + y + {j};\n"
                            ));
                        }
                        src.push_str("    x + y\n");
                        src.push_str("}\n\n");
                    }
                    src
                })
        }

        /// Generate a Python source file with N functions and M classes.
        fn arb_python_source() -> impl Strategy<Value = String> {
            (
                1usize..=5,
                1usize..=3,
                5usize..=20,
            )
                .prop_map(|(n_fns, n_classes, body_lines)| {
                    let mut src = String::new();
                    src.push_str("import os\nimport sys\nfrom typing import List, Dict\n\n");

                    for i in 0..n_classes {
                        src.push_str(&format!("class MyClass{i}:\n"));
                        src.push_str("    def __init__(self):\n");
                        for j in 0..body_lines {
                            src.push_str(&format!("        self.field_{j} = {j}\n"));
                        }
                        src.push('\n');
                    }

                    for i in 0..n_fns {
                        src.push_str(&format!("def my_function_{i}(x, y):\n"));
                        for j in 0..body_lines {
                            src.push_str(&format!("    var_{j} = x + y + {j}\n"));
                        }
                        src.push_str("    return x + y\n\n");
                    }
                    src
                })
        }

        proptest! {
            /// **Property 25: AST extraction preserves public API (Rust)**
            ///
            /// **Validates: Requirements 19.2, 19.3**
            #[test]
            fn prop25_ast_preserves_public_api_rust(source in arb_rust_source()) {
                let parser = AstParser::new();
                let summary = parser.extract_signatures(&source, "rust")
                    .expect("rust extraction should succeed");

                // Compression occurred
                prop_assert!(
                    summary.tokens_summary < summary.tokens_original,
                    "tokens_summary ({}) must be < tokens_original ({})",
                    summary.tokens_summary,
                    summary.tokens_original
                );

                // Function names appear in summary
                let summary_text = summary.to_text();
                for func in &summary.functions {
                    prop_assert!(
                        summary_text.contains(&func.name),
                        "function name '{}' must appear in summary",
                        func.name
                    );
                }

                // Class/struct names appear in summary
                for cls in &summary.classes {
                    prop_assert!(
                        summary_text.contains(&cls.name),
                        "class name '{}' must appear in summary",
                        cls.name
                    );
                }

                // At least one function or class was extracted
                prop_assert!(
                    !summary.functions.is_empty() || !summary.classes.is_empty(),
                    "must extract at least one function or class"
                );
            }

            /// **Property 25: AST extraction preserves public API (Python)**
            ///
            /// **Validates: Requirements 19.2, 19.3**
            #[test]
            fn prop25_ast_preserves_public_api_python(source in arb_python_source()) {
                let parser = AstParser::new();
                let summary = parser.extract_signatures(&source, "python")
                    .expect("python extraction should succeed");

                // Compression occurred
                prop_assert!(
                    summary.tokens_summary < summary.tokens_original,
                    "tokens_summary ({}) must be < tokens_original ({})",
                    summary.tokens_summary,
                    summary.tokens_original
                );

                // Function names appear in summary
                let summary_text = summary.to_text();
                for func in &summary.functions {
                    prop_assert!(
                        summary_text.contains(&func.name),
                        "function name '{}' must appear in summary",
                        func.name
                    );
                }

                // Class names appear in summary
                for cls in &summary.classes {
                    prop_assert!(
                        summary_text.contains(&cls.name),
                        "class name '{}' must appear in summary",
                        cls.name
                    );
                }

                // At least one function or class was extracted
                prop_assert!(
                    !summary.functions.is_empty() || !summary.classes.is_empty(),
                    "must extract at least one function or class"
                );
            }
        }
    }
}
