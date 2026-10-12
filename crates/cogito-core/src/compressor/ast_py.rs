//! AST-aware Python compressor
//!
//! Uses `tree-sitter-python` to parse the source into an AST, then walks
//! the tree collapsing function/method bodies while preserving signatures,
//! type hints, and declarations.

use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

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
        .map_err(|e| anyhow!("Failed to set tree-sitter language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Python"))?;

    let mut stats = PyStats::default();
    let mut out = String::with_capacity(content.len() / 2);

    let root = tree.root_node();
    walk(root, content, &mut out, &mut stats);

    Ok(PyCompressResult {
        content: out,
        functions_collapsed: stats.functions,
        classes_preserved: stats.classes,
        decorators_preserved: stats.decorators,
    })
}

#[derive(Default)]
struct PyStats {
    functions: usize,
    classes: usize,
    decorators: usize,
}

/// Cached tree-sitter language
static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_python::LANGUAGE.into())
        .clone()
}

fn walk(node: Node, source: &str, out: &mut String, stats: &mut PyStats) {
    let mut cursor = node.walk();
    let mut last_newline = true;
    for child in node.children(&mut cursor) {
        if render_node(child, source, out, stats, 0) {
            if !last_newline {
                out.push('\n');
            }
            last_newline = false;
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
}

fn render_node(
    node: Node,
    source: &str,
    out: &mut String,
    stats: &mut PyStats,
    indent: usize,
) -> bool {
    let text = node_text(node, source);
    let pad = "    ".repeat(indent);

    match node.kind() {
        // Imports
        "import_statement" | "import_from_statement" | "future_import_statement" => {
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
        // Function definitions
        "function_definition" => {
            stats.functions += 1;
            render_function(node, source, out, stats, indent);
            true
        }
        // Class definitions
        "class_definition" => {
            stats.classes += 1;
            render_class(node, source, out, stats, indent);
            true
        }
        // Decorators
        "decorator" => {
            stats.decorators += 1;
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
        // Decorated definitions: pass through decorator + collapse body
        "decorated_definition" => {
            render_decorated(node, source, out, stats, indent);
            true
        }
        // Statements
        "expression_statement"
        | "if_statement"
        | "for_statement"
        | "while_statement"
        | "try_statement"
        | "with_statement"
        | "assert_statement"
        | "raise_statement"
        | "return_statement"
        | "pass_statement"
        | "break_statement"
        | "continue_statement" => {
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
        // Assignments
        "assignment" | "augmented_assignment" | "expression_list" => {
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
        // Type definitions
        "type_alias" => {
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
        // Skip standalone punctuation
        "comment" => {
            // Keep docstrings as comments? Strip for now.
            false
        }
        _ => {
            out.push_str(&pad);
            out.push_str(&text);
            true
        }
    }
}

fn render_function(node: Node, source: &str, out: &mut String, stats: &mut PyStats, indent: usize) {
    stats.functions += 1;
    let pad = "    ".repeat(indent);

    // `def name(params) -> ReturnType:`
    if let Some(name_node) = node.child_by_field_name("name") {
        out.push_str(&pad);
        out.push_str("def ");
        out.push_str(&node_text(name_node, source));
    } else {
        out.push_str(&pad);
        out.push_str("def ");
    }

    if let Some(params) = node.child_by_field_name("parameters") {
        out.push_str(&node_text(params, source));
    } else {
        out.push_str("()");
    }

    if let Some(ret) = node.child_by_field_name("return_type") {
        out.push_str(" -> ");
        out.push_str(&node_text(ret, source));
    }

    out.push_str(": /* ... */\n");
}

fn render_class(node: Node, source: &str, out: &mut String, stats: &mut PyStats, indent: usize) {
    let pad = "    ".repeat(indent);

    // `class Name(Base):`
    if let Some(name_node) = node.child_by_field_name("name") {
        out.push_str(&pad);
        out.push_str("class ");
        out.push_str(&node_text(name_node, source));
    } else {
        out.push_str(&pad);
        out.push_str("class ");
    }

    if let Some(supers) = node.child_by_field_name("superclasses") {
        out.push_str(&node_text(supers, source));
    }

    out.push_str(":\n");

    // Body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            render_node(child, source, out, stats, indent + 1);
            out.push('\n');
        }
    }
}

fn render_decorated(
    node: Node,
    source: &str,
    out: &mut String,
    stats: &mut PyStats,
    indent: usize,
) {
    let pad = "    ".repeat(indent);

    // Render decorators + definition
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            stats.decorators += 1;
            out.push_str(&pad);
            out.push_str(&node_text(child, source));
            out.push('\n');
        } else if child.kind() == "function_definition" {
            render_function(child, source, out, stats, indent);
        } else if child.kind() == "class_definition" {
            render_class(child, source, out, stats, indent);
        }
    }
}

fn node_text(node: Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    source[start..end].to_string()
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
        assert!(result.content.contains("/* ... */"));
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
        // Method bodies should NOT be in output
        assert!(!result.content.contains("self.id = id"));
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

        // Should be at least 30% smaller
        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        // All public APIs preserved
        assert!(result.content.contains("class User"));
        assert!(result.content.contains("class UserRepository"));
        assert!(result.content.contains("def find_by_id"));
        assert!(result.content.contains("def find_by_email"));
        assert!(result.content.contains("def create"));
    }
}
