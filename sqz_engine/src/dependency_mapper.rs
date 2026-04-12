//! Dependency graph builder that parses import/require/use statements
//! to map relationships between files in a project.
//!
//! Supports the same 18 languages as `AstParser` via regex-based import
//! extraction. The graph is cached and incrementally updated on file changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A directed dependency graph mapping files to their imports and reverse dependencies.
#[derive(Debug, Clone)]
pub struct DependencyMapper {
    /// path → set of paths that `path` imports
    dependencies: HashMap<PathBuf, HashSet<PathBuf>>,
    /// path → set of paths that import `path`
    dependents: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl DependencyMapper {
    /// Create an empty dependency mapper.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Parse import statements from `source` for the file at `path` and add
    /// the discovered edges to the graph. If the file was already tracked,
    /// its old edges are removed first (incremental update).
    pub fn add_file(&mut self, path: &Path, source: &str) {
        // Remove stale edges if the file was previously tracked
        self.remove_file(path);

        let lang = detect_language(path);
        let raw_imports = parse_imports(source, lang.as_deref());

        let dir = path.parent().unwrap_or(Path::new(""));
        let resolved: HashSet<PathBuf> = raw_imports
            .into_iter()
            .filter_map(|imp| resolve_import(&imp, dir))
            .collect();

        // Insert forward edges
        self.dependencies.insert(path.to_path_buf(), resolved.clone());

        // Insert reverse edges
        for dep in &resolved {
            self.dependents
                .entry(dep.clone())
                .or_default()
                .insert(path.to_path_buf());
        }
    }

    /// Remove a file and all its edges from the graph.
    pub fn remove_file(&mut self, path: &Path) {
        if let Some(old_deps) = self.dependencies.remove(path) {
            for dep in &old_deps {
                if let Some(rev) = self.dependents.get_mut(dep) {
                    rev.remove(path);
                    if rev.is_empty() {
                        self.dependents.remove(dep);
                    }
                }
            }
        }
        // Also clean up any reverse entries pointing to this file
        if let Some(old_rev) = self.dependents.remove(path) {
            for src in &old_rev {
                if let Some(fwd) = self.dependencies.get_mut(src) {
                    fwd.remove(path);
                }
            }
        }
    }

    /// Returns the set of files that `path` imports (direct dependencies).
    pub fn dependencies_of(&self, path: &Path) -> Vec<PathBuf> {
        let mut deps: Vec<PathBuf> = self
            .dependencies
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        deps.sort();
        deps
    }

    /// Returns the set of files that import `path` (reverse dependencies).
    pub fn dependents_of(&self, path: &Path) -> Vec<PathBuf> {
        let mut deps: Vec<PathBuf> = self
            .dependents
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        deps.sort();
        deps
    }

    /// Produce a compact dependency summary string suitable for inclusion
    /// in file read output.
    pub fn summary(&self, path: &Path) -> String {
        let deps = self.dependencies_of(path);
        let revs = self.dependents_of(path);

        if deps.is_empty() && revs.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();

        if !deps.is_empty() {
            let names: Vec<String> = deps
                .iter()
                .map(|p| short_name(p))
                .collect();
            parts.push(format!("imports: {}", names.join(", ")));
        }

        if !revs.is_empty() {
            let names: Vec<String> = revs
                .iter()
                .map(|p| short_name(p))
                .collect();
            parts.push(format!("imported by: {}", names.join(", ")));
        }

        format!("[deps] {}", parts.join(" | "))
    }

    /// Returns the number of tracked files.
    pub fn file_count(&self) -> usize {
        self.dependencies.len()
    }
}

impl Default for DependencyMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the file name (or last two path components) for compact display.
fn short_name(path: &Path) -> String {
    let components: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    if components.len() <= 2 {
        path.to_string_lossy().to_string()
    } else {
        components[components.len() - 2..].join("/")
    }
}

// ---------------------------------------------------------------------------
// Language detection (mirrors ast_parser logic)
// ---------------------------------------------------------------------------

fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" | "mts" => "typescript",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        "rb" => "ruby",
        "sh" | "bash" => "bash",
        "json" => "json",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        "cs" => "csharp",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => return None,
    };
    Some(lang.to_string())
}

// ---------------------------------------------------------------------------
// Import parsing — regex-based patterns per language
// ---------------------------------------------------------------------------

/// Parse raw import specifiers from source code. Returns the module/path
/// strings as written in the source (not yet resolved to file paths).
fn parse_imports(source: &str, language: Option<&str>) -> Vec<String> {
    let lang = match language {
        Some(l) => l,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match lang {
            "rust" => parse_rust_import(trimmed, &mut imports),
            "python" => parse_python_import(trimmed, &mut imports),
            "javascript" | "typescript" => parse_js_ts_import(trimmed, &mut imports),
            "go" => parse_go_import(trimmed, &mut imports),
            "java" | "kotlin" => parse_java_kotlin_import(trimmed, &mut imports),
            "c" | "cpp" => parse_c_cpp_import(trimmed, &mut imports),
            "ruby" => parse_ruby_import(trimmed, &mut imports),
            "csharp" => parse_csharp_import(trimmed, &mut imports),
            "swift" => parse_swift_import(trimmed, &mut imports),
            "css" => parse_css_import(trimmed, &mut imports),
            "html" => parse_html_import(trimmed, &mut imports),
            _ => {} // json, toml, yaml, bash — no meaningful imports
        }
    }

    imports
}

/// Rust: `use crate::foo::bar;` or `use super::baz;` or `mod foo;`
fn parse_rust_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("use ") {
        let spec = rest.trim_end_matches(';').trim();
        // Extract the path portion (before any `{` or `as`)
        let path = spec.split('{').next().unwrap_or(spec);
        let path = path.split(" as ").next().unwrap_or(path).trim();
        if !path.is_empty() {
            imports.push(path.to_string());
        }
    } else if let Some(rest) = line.strip_prefix("mod ") {
        let name = rest.trim_end_matches(';').trim();
        if !name.is_empty() && !line.contains('{') {
            imports.push(name.to_string());
        }
    }
}

/// Python: `import foo` or `from foo import bar` or `from . import baz`
fn parse_python_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("from ") {
        // `from foo.bar import baz`
        if let Some(module) = rest.split(" import").next() {
            let module = module.trim();
            if !module.is_empty() {
                imports.push(module.to_string());
            }
        }
    } else if let Some(rest) = line.strip_prefix("import ") {
        // `import foo, bar` or `import foo.bar`
        for part in rest.split(',') {
            let module = part.split(" as ").next().unwrap_or(part).trim();
            if !module.is_empty() {
                imports.push(module.to_string());
            }
        }
    }
}

/// JS/TS: `import ... from '...'`, `require('...')`, `import '...'`
fn parse_js_ts_import(line: &str, imports: &mut Vec<String>) {
    // Static import: import X from 'path' or import 'path'
    if line.starts_with("import ") || line.starts_with("export ") {
        if let Some(path) = extract_quoted_string(line, "from ") {
            imports.push(path);
        } else if line.starts_with("import ") {
            // `import './side-effect'`
            if let Some(path) = extract_first_quoted(line) {
                imports.push(path);
            }
        }
    }
    // CommonJS require
    if line.contains("require(") {
        if let Some(path) = extract_require_path(line) {
            imports.push(path);
        }
    }
}

/// Go: `import "path"` or `import ( "path" )`
fn parse_go_import(line: &str, imports: &mut Vec<String>) {
    let trimmed = line.trim();
    if trimmed.starts_with("import ") || trimmed.starts_with("\"") {
        if let Some(path) = extract_first_quoted(trimmed) {
            imports.push(path);
        }
    }
}

/// Java/Kotlin: `import foo.bar.Baz;`
fn parse_java_kotlin_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("import ") {
        let rest = rest.strip_prefix("static ").unwrap_or(rest);
        let path = rest.trim_end_matches(';').trim();
        if !path.is_empty() {
            imports.push(path.to_string());
        }
    }
}

/// C/C++: `#include <header>` or `#include "header"`
fn parse_c_cpp_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("#include") {
        let rest = rest.trim();
        if let Some(path) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            imports.push(path.to_string());
        } else if let Some(path) = rest.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
            imports.push(path.to_string());
        }
    }
}

/// Ruby: `require 'foo'` or `require_relative 'foo'`
fn parse_ruby_import(line: &str, imports: &mut Vec<String>) {
    if line.starts_with("require ") || line.starts_with("require_relative ") {
        if let Some(path) = extract_first_quoted(line) {
            imports.push(path);
        }
    }
}

/// C#: `using Foo.Bar;`
fn parse_csharp_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("using ") {
        // Skip `using static` or `using var` (not imports)
        if rest.starts_with("var ") || rest.contains('=') {
            return;
        }
        let rest = rest.strip_prefix("static ").unwrap_or(rest);
        let ns = rest.trim_end_matches(';').trim();
        if !ns.is_empty() {
            imports.push(ns.to_string());
        }
    }
}

/// Swift: `import Foundation`
fn parse_swift_import(line: &str, imports: &mut Vec<String>) {
    if let Some(rest) = line.strip_prefix("import ") {
        let module = rest.trim();
        if !module.is_empty() {
            imports.push(module.to_string());
        }
    }
}

/// CSS: `@import url('...')` or `@import '...'`
fn parse_css_import(line: &str, imports: &mut Vec<String>) {
    if line.starts_with("@import") {
        if let Some(path) = extract_first_quoted(line) {
            imports.push(path);
        }
    }
}

/// HTML: `<script src="...">` or `<link href="...">`
fn parse_html_import(line: &str, imports: &mut Vec<String>) {
    for attr in &["src=\"", "href=\""] {
        if let Some(idx) = line.find(attr) {
            let start = idx + attr.len();
            if let Some(end) = line[start..].find('"') {
                let path = &line[start..start + end];
                if !path.is_empty() {
                    imports.push(path.to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// String extraction helpers
// ---------------------------------------------------------------------------

/// Extract the quoted string after a keyword, e.g. `from 'foo'` → `foo`.
fn extract_quoted_string(line: &str, keyword: &str) -> Option<String> {
    let idx = line.find(keyword)?;
    let rest = &line[idx + keyword.len()..];
    extract_first_quoted(rest)
}

/// Extract the first single- or double-quoted string from text.
fn extract_first_quoted(text: &str) -> Option<String> {
    for quote in &['\'', '"'] {
        if let Some(start) = text.find(*quote) {
            let rest = &text[start + 1..];
            if let Some(end) = rest.find(*quote) {
                let val = &rest[..end];
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Extract path from `require('...')` or `require("...")`.
fn extract_require_path(line: &str) -> Option<String> {
    let idx = line.find("require(")?;
    let rest = &line[idx + "require(".len()..];
    // Find closing paren
    let paren_end = rest.find(')')?;
    let inner = &rest[..paren_end];
    extract_first_quoted(inner)
}

// ---------------------------------------------------------------------------
// Import resolution — best-effort path mapping
// ---------------------------------------------------------------------------

/// Attempt to resolve a raw import specifier to a relative file path.
/// This is best-effort: we normalize relative paths and convert module
/// paths to plausible file paths. Returns `None` for unresolvable imports
/// (e.g. standard library modules).
fn resolve_import(raw: &str, _base_dir: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Skip standard library / external package imports that aren't file paths
    // Heuristic: if it starts with `.` or `/` or contains a file extension, treat as path
    if trimmed.starts_with('.')
        || trimmed.starts_with('/')
        || trimmed.contains(".rs")
        || trimmed.contains(".py")
        || trimmed.contains(".js")
        || trimmed.contains(".ts")
        || trimmed.contains(".go")
        || trimmed.contains(".rb")
        || trimmed.contains(".h")
        || trimmed.contains(".c")
        || trimmed.contains(".css")
        || trimmed.contains(".html")
    {
        let path = PathBuf::from(trimmed);
        return Some(path);
    }

    // For Rust crate:: and super:: paths, convert to relative path
    if trimmed.starts_with("crate::") || trimmed.starts_with("super::") {
        let converted = trimmed
            .replace("crate::", "src/")
            .replace("super::", "../")
            .replace("::", "/");
        return Some(PathBuf::from(converted));
    }

    // For module-style imports (e.g. `foo.bar` in Python/Java), convert dots to slashes
    if trimmed.contains('.') && !trimmed.contains('/') {
        let converted = trimmed.replace('.', "/");
        return Some(PathBuf::from(converted));
    }

    // For C/C++ includes with path separators
    if trimmed.contains('/') {
        return Some(PathBuf::from(trimmed));
    }

    // Single-word imports (could be stdlib) — still track them for graph completeness
    Some(PathBuf::from(trimmed))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_mapper_is_empty() {
        let mapper = DependencyMapper::new();
        assert_eq!(mapper.file_count(), 0);
        assert!(mapper.dependencies_of(Path::new("foo.rs")).is_empty());
        assert!(mapper.dependents_of(Path::new("foo.rs")).is_empty());
    }

    #[test]
    fn test_add_rust_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
use crate::engine;
use crate::types;
use std::collections::HashMap;

pub fn main() {}
"#;
        mapper.add_file(Path::new("src/lib.rs"), source);
        let deps = mapper.dependencies_of(Path::new("src/lib.rs"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("engine")));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("types")));
    }

    #[test]
    fn test_add_python_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
import os
from typing import List
from .utils import helper
"#;
        mapper.add_file(Path::new("app/main.py"), source);
        let deps = mapper.dependencies_of(Path::new("app/main.py"));
        assert!(!deps.is_empty());
        assert!(deps.iter().any(|p| p.to_string_lossy() == "os"));
    }

    #[test]
    fn test_add_javascript_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
import React from 'react';
import { useState } from 'react';
const fs = require('fs');
import './styles.css';
"#;
        mapper.add_file(Path::new("src/app.js"), source);
        let deps = mapper.dependencies_of(Path::new("src/app.js"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "react"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "fs"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "./styles.css"));
    }

    #[test]
    fn test_add_typescript_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
import { Component } from '@angular/core';
import { MyService } from './my-service';
export { default } from './utils';
"#;
        mapper.add_file(Path::new("src/app.ts"), source);
        let deps = mapper.dependencies_of(Path::new("src/app.ts"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "@angular/core"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "./my-service"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "./utils"));
    }

    #[test]
    fn test_add_go_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
package main

import "fmt"
import "os"
"#;
        mapper.add_file(Path::new("main.go"), source);
        let deps = mapper.dependencies_of(Path::new("main.go"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "fmt"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "os"));
    }

    #[test]
    fn test_add_java_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
import java.util.List;
import com.example.MyClass;
"#;
        mapper.add_file(Path::new("src/Main.java"), source);
        let deps = mapper.dependencies_of(Path::new("src/Main.java"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("java/util/List")));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("com/example/MyClass")));
    }

    #[test]
    fn test_add_c_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
#include <stdio.h>
#include "myheader.h"
"#;
        mapper.add_file(Path::new("src/main.c"), source);
        let deps = mapper.dependencies_of(Path::new("src/main.c"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "stdio.h"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "myheader.h"));
    }

    #[test]
    fn test_add_ruby_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
require 'json'
require_relative 'helper'
"#;
        mapper.add_file(Path::new("lib/app.rb"), source);
        let deps = mapper.dependencies_of(Path::new("lib/app.rb"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "json"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "helper"));
    }

    #[test]
    fn test_add_csharp_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
using System;
using System.Collections.Generic;
"#;
        mapper.add_file(Path::new("src/Program.cs"), source);
        let deps = mapper.dependencies_of(Path::new("src/Program.cs"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "System"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("System/Collections/Generic")));
    }

    #[test]
    fn test_add_swift_file() {
        let mut mapper = DependencyMapper::new();
        let source = "import Foundation\nimport UIKit\n";
        mapper.add_file(Path::new("Sources/App.swift"), source);
        let deps = mapper.dependencies_of(Path::new("Sources/App.swift"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "Foundation"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "UIKit"));
    }

    #[test]
    fn test_add_kotlin_file() {
        let mut mapper = DependencyMapper::new();
        let source = "import com.example.MyClass\nimport kotlin.collections.List\n";
        mapper.add_file(Path::new("src/Main.kt"), source);
        let deps = mapper.dependencies_of(Path::new("src/Main.kt"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("com/example/MyClass")));
    }

    #[test]
    fn test_add_css_file() {
        let mut mapper = DependencyMapper::new();
        let source = "@import 'reset.css';\n@import url('theme.css');\n";
        mapper.add_file(Path::new("styles/main.css"), source);
        let deps = mapper.dependencies_of(Path::new("styles/main.css"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("reset.css")));
    }

    #[test]
    fn test_add_html_file() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
<link href="styles.css" rel="stylesheet">
<script src="app.js"></script>
"#;
        mapper.add_file(Path::new("index.html"), source);
        let deps = mapper.dependencies_of(Path::new("index.html"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "styles.css"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "app.js"));
    }

    #[test]
    fn test_reverse_dependencies() {
        let mut mapper = DependencyMapper::new();

        mapper.add_file(
            Path::new("src/a.rs"),
            "use crate::shared;\n",
        );
        mapper.add_file(
            Path::new("src/b.rs"),
            "use crate::shared;\n",
        );

        let revs = mapper.dependents_of(Path::new("src/shared"));
        assert_eq!(revs.len(), 2);
    }

    #[test]
    fn test_remove_file_cleans_edges() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(
            Path::new("src/a.rs"),
            "use crate::b;\n",
        );
        assert!(!mapper.dependencies_of(Path::new("src/a.rs")).is_empty());

        mapper.remove_file(Path::new("src/a.rs"));
        assert!(mapper.dependencies_of(Path::new("src/a.rs")).is_empty());
        assert_eq!(mapper.file_count(), 0);
    }

    #[test]
    fn test_incremental_update() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(Path::new("src/a.rs"), "use crate::old_dep;\n");
        assert!(mapper
            .dependencies_of(Path::new("src/a.rs"))
            .iter()
            .any(|p| p.to_string_lossy().contains("old_dep")));

        // Re-add with different imports
        mapper.add_file(Path::new("src/a.rs"), "use crate::new_dep;\n");
        let deps = mapper.dependencies_of(Path::new("src/a.rs"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("new_dep")));
        assert!(!deps.iter().any(|p| p.to_string_lossy().contains("old_dep")));
    }

    #[test]
    fn test_summary_empty() {
        let mapper = DependencyMapper::new();
        assert_eq!(mapper.summary(Path::new("foo.rs")), "");
    }

    #[test]
    fn test_summary_with_deps() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(
            Path::new("src/main.rs"),
            "use crate::engine;\nuse crate::types;\n",
        );
        let summary = mapper.summary(Path::new("src/main.rs"));
        assert!(summary.starts_with("[deps]"));
        assert!(summary.contains("imports:"));
    }

    #[test]
    fn test_summary_with_reverse_deps() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(Path::new("src/a.rs"), "use crate::shared;\n");
        mapper.add_file(Path::new("src/b.rs"), "use crate::shared;\n");

        let summary = mapper.summary(Path::new("src/shared"));
        assert!(summary.contains("imported by:"));
    }

    #[test]
    fn test_detect_language_extensions() {
        assert_eq!(detect_language(Path::new("foo.rs")).as_deref(), Some("rust"));
        assert_eq!(detect_language(Path::new("foo.py")).as_deref(), Some("python"));
        assert_eq!(detect_language(Path::new("foo.js")).as_deref(), Some("javascript"));
        assert_eq!(detect_language(Path::new("foo.ts")).as_deref(), Some("typescript"));
        assert_eq!(detect_language(Path::new("foo.go")).as_deref(), Some("go"));
        assert_eq!(detect_language(Path::new("foo.java")).as_deref(), Some("java"));
        assert_eq!(detect_language(Path::new("foo.c")).as_deref(), Some("c"));
        assert_eq!(detect_language(Path::new("foo.cpp")).as_deref(), Some("cpp"));
        assert_eq!(detect_language(Path::new("foo.rb")).as_deref(), Some("ruby"));
        assert_eq!(detect_language(Path::new("foo.cs")).as_deref(), Some("csharp"));
        assert_eq!(detect_language(Path::new("foo.kt")).as_deref(), Some("kotlin"));
        assert_eq!(detect_language(Path::new("foo.swift")).as_deref(), Some("swift"));
        assert_eq!(detect_language(Path::new("foo.css")).as_deref(), Some("css"));
        assert_eq!(detect_language(Path::new("foo.html")).as_deref(), Some("html"));
        assert_eq!(detect_language(Path::new("foo.sh")).as_deref(), Some("bash"));
        assert_eq!(detect_language(Path::new("foo.toml")).as_deref(), Some("toml"));
        assert_eq!(detect_language(Path::new("foo.yaml")).as_deref(), Some("yaml"));
        assert_eq!(detect_language(Path::new("foo.json")).as_deref(), Some("json"));
        assert_eq!(detect_language(Path::new("foo.xyz")), None);
    }

    #[test]
    fn test_unknown_extension_no_imports() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(Path::new("data.xyz"), "some random content\n");
        assert!(mapper.dependencies_of(Path::new("data.xyz")).is_empty());
    }

    #[test]
    fn test_cpp_includes() {
        let mut mapper = DependencyMapper::new();
        let source = "#include <iostream>\n#include \"mylib.h\"\n";
        mapper.add_file(Path::new("src/main.cpp"), source);
        let deps = mapper.dependencies_of(Path::new("src/main.cpp"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "iostream"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "mylib.h"));
    }

    #[test]
    fn test_python_from_import() {
        let mut mapper = DependencyMapper::new();
        let source = "from os.path import join\nfrom . import utils\n";
        mapper.add_file(Path::new("app/main.py"), source);
        let deps = mapper.dependencies_of(Path::new("app/main.py"));
        assert!(deps.iter().any(|p| p.to_string_lossy().contains("os")));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "."));
    }

    #[test]
    fn test_default_trait() {
        let mapper = DependencyMapper::default();
        assert_eq!(mapper.file_count(), 0);
    }

    #[test]
    fn test_file_count() {
        let mut mapper = DependencyMapper::new();
        mapper.add_file(Path::new("a.rs"), "use crate::b;\n");
        mapper.add_file(Path::new("b.rs"), "use crate::c;\n");
        assert_eq!(mapper.file_count(), 2);
        mapper.remove_file(Path::new("a.rs"));
        assert_eq!(mapper.file_count(), 1);
    }

    #[test]
    fn test_csharp_using_var_skipped() {
        let mut mapper = DependencyMapper::new();
        let source = "using System;\nusing var stream = new FileStream();\n";
        mapper.add_file(Path::new("Program.cs"), source);
        let deps = mapper.dependencies_of(Path::new("Program.cs"));
        // Should have System but not the `using var` statement
        assert!(deps.iter().any(|p| p.to_string_lossy() == "System"));
        assert!(!deps.iter().any(|p| p.to_string_lossy().contains("stream")));
    }

    #[test]
    fn test_js_require_and_import() {
        let mut mapper = DependencyMapper::new();
        let source = r#"
import defaultExport from './module';
const path = require('path');
"#;
        mapper.add_file(Path::new("index.js"), source);
        let deps = mapper.dependencies_of(Path::new("index.js"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "./module"));
        assert!(deps.iter().any(|p| p.to_string_lossy() == "path"));
    }

    #[test]
    fn test_short_name() {
        assert_eq!(short_name(Path::new("a.rs")), "a.rs");
        assert_eq!(short_name(Path::new("src/lib.rs")), "src/lib.rs");
        assert_eq!(short_name(Path::new("foo/bar/baz.rs")), "bar/baz.rs");
    }
}
