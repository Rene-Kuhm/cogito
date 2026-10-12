//! AST-aware Python compressor
//!
//! Uses `tree-sitter-python` + **byte-range replacement**: find the byte
//! range of every function/method body and replace it with a placeholder,
//! leaving everything else verbatim.
//!
//! Note: Python uses indentation-based blocks, so the "body" is a `block`
//! node whose range includes the indented statements. Replacing it with
//! `{ /* ... */ }` would break Python's indentation rules, so we replace
//! with a single-line `...` placeholder instead (valid Python).

use super::ast_common::collect_body_ranges;
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::Parser;

/// Result of Python AST compression
#[derive(Debug, Clone)]
pub struct PyCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub classes_preserved: usize,
    pub decorators_preserved: usize,
}

/// Compress Python source using AST awareness.
pub fn compress_python(content: &str, _options: &CompressionOptions) -> Result<PyCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter Python language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Python"))?;

    let ranges = collect_python_bodies(tree.root_node(), content);
    let (compressed, applied) = apply_python_replacements(content, &ranges);

    let mut stats = PyStats::default();
    count_declarations(tree.root_node(), &mut stats);

    Ok(PyCompressResult {
        content: compressed,
        functions_collapsed: applied,
        classes_preserved: stats.classes,
        decorators_preserved: stats.decorators,
    })
}

#[derive(Default)]
struct PyStats {
    classes: usize,
    decorators: usize,
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_python::LANGUAGE.into())
        .clone()
}

/// Collect body ranges for Python functions. `class_definition` bodies are
/// intentionally excluded — we want to descend into them and collapse each
/// method separately.
fn collect_python_bodies(root: tree_sitter::Node, _source: &str) -> Vec<(usize, usize)> {
    collect_body_ranges(root, |kind| kind == "function_definition")
}

/// Python-specific: replace the body with a single-line ellipsis, preserving
/// the indentation of the first line so the result stays syntactically valid.
fn apply_python_replacements(source: &str, ranges: &[(usize, usize)]) -> (String, usize) {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut applied = 0usize;

    for (start, end) in ranges {
        if *start < cursor || *end > source.len() {
            continue;
        }
        out.push_str(&source[cursor..*start]);
        // Match the leading indentation of the body's first line so the
        // placeholder is a valid Python statement.
        let indent = leading_indent(source, *start);
        out.push_str(&indent);
        out.push_str("...");
        cursor = *end;
        applied += 1;
    }

    out.push_str(&source[cursor..]);
    (out, applied)
}

fn leading_indent(source: &str, pos: usize) -> String {
    // Walk to the start of the line containing `pos`
    let line_start = source[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &source[line_start..pos];
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn count_declarations(node: tree_sitter::Node, stats: &mut PyStats) {
    match node.kind() {
        "class_definition" => stats.classes += 1,
        "decorator" => stats.decorators += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_declarations(child, stats);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_function() {
        let code = r#"
def add(a: int, b: int) -> int:
    result = a + b
    print(f"debug: a={a} b={b}")
    if a > 100:
        return b
    return result * 2
"#;
        let opts = CompressionOptions::default();
        let result = compress_python(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("def add"));
        assert!(result.content.contains("a: int"));
        assert!(result.content.contains("b: int"));
        assert!(result.content.contains("-> int"));
        // Body should NOT be in output
        assert!(!result.content.contains("result = a + b"));
        assert!(!result.content.contains("print(f"));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"
class User:
    def __init__(self, id: int, name: str):
        self.id = id
        self.name = name

    def validate(self) -> bool:
        if not self.name:
            return False
        return True

    def get_id(self) -> int:
        return self.id
"#;
        let opts = CompressionOptions::default();
        let result = compress_python(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.functions_collapsed >= 3);
        assert!(result.content.contains("class User"));
        assert!(result.content.contains("def __init__"));
        assert!(result.content.contains("def validate"));
        assert!(result.content.contains("def get_id"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("self.id = id"));
        assert!(!result.content.contains("return self.id"));
    }

    #[test]
    fn test_preserves_imports() {
        let code = r#"
import os
from typing import List, Optional

def process(data: List[str]) -> Optional[str]:
    if not data:
        return None
    return data[0]
"#;
        let opts = CompressionOptions::default();
        let result = compress_python(code, &opts).unwrap();
        assert!(result.content.contains("import os"));
        assert!(result.content.contains("from typing import"));
    }

    #[test]
    fn test_preserves_decorators() {
        let code = r#"
@staticmethod
@deprecated
def legacy_method(x: int) -> int:
    return x * 2
"#;
        let opts = CompressionOptions::default();
        let result = compress_python(code, &opts).unwrap();
        assert!(result.decorators_preserved >= 2);
        assert!(result.content.contains("@staticmethod"));
        assert!(result.content.contains("@deprecated"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
from typing import List, Optional, Dict
from dataclasses import dataclass


@dataclass
class User:
    id: int
    name: str
    email: str


class UserRepository:
    def __init__(self, db_connection):
        self.db = db_connection

    def find_by_id(self, user_id: int) -> Optional[User]:
        query = "SELECT * FROM users WHERE id = ?"
        result = self.db.execute(query, (user_id,))
        if result is None:
            return None
        return User(**result)

    def find_by_email(self, email: str) -> Optional[User]:
        query = "SELECT * FROM users WHERE email = ?"
        result = self.db.execute(query, (email,))
        if result is None:
            return None
        return User(**result)

    def create(self, user: User) -> User:
        existing = self.find_by_email(user.email)
        if existing is not None:
            raise ValueError("User already exists")
        self.db.execute(
            "INSERT INTO users (id, name, email) VALUES (?, ?, ?)",
            (user.id, user.name, user.email),
        )
        return user
"#;
        let opts = CompressionOptions::default();
        let result = compress_python(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Python module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class User"));
        assert!(result.content.contains("class UserRepository"));
        assert!(result.content.contains("def find_by_id"));
        assert!(result.content.contains("def find_by_email"));
        assert!(result.content.contains("def create"));
    }
}
