//! AST-aware Kotlin compressor
//!
//! Uses `tree-sitter-kotlin-ng` + **byte-range replacement**.
//!
//! Unlike most tree-sitter grammars, Kotlin's `function_declaration` does
//! **not** expose the body via a named field — the `function_body` node is
//! a plain child. We therefore look it up by kind.
//!
//! Preserved: `package`, `import`, annotations, class/object/interface
//! declarations, property declarations, function signatures.
//! Collapsed: function bodies, init block bodies.

use super::ast_common::{apply_replacements, collect_ranges};
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

/// Result of Kotlin AST compression
#[derive(Debug, Clone)]
pub struct KotlinCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub classes_preserved: usize,
}

/// Compress Kotlin source using AST awareness.
pub fn compress_kotlin(
    content: &str,
    _options: &CompressionOptions,
) -> Result<KotlinCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter Kotlin language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Kotlin"))?;

    let ranges = collect_ranges(tree.root_node(), |node| {
        if !is_function_node(node.kind()) {
            return None;
        }
        find_child_by_kind(node, "function_body").map(|b| (b.start_byte(), b.end_byte()))
    });

    let (compressed, applied) = apply_replacements(content, &ranges);

    let mut classes = 0usize;
    count_declarations(tree.root_node(), &mut classes);

    Ok(KotlinCompressResult {
        content: compressed,
        functions_collapsed: applied,
        classes_preserved: classes,
    })
}

fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration" | "anonymous_function" | "lambda_literal"
    )
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

fn count_declarations(node: Node, classes: &mut usize) {
    if matches!(
        node.kind(),
        "class_declaration" | "object_declaration" | "interface_declaration"
    ) {
        *classes += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_declarations(child, classes);
    }
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_kotlin_ng::LANGUAGE.into())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_function() {
        let code = r#"
package com.example.demo

import kotlin.math.abs

fun add(a: Int, b: Int): Int {
    val result = a + b
    println("debug: $a + $b = $result")
    if (a > 100) {
        return b
    }
    return result * 2
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_kotlin(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("fun add"));
        assert!(result.content.contains("a: Int"));
        assert!(result.content.contains("b: Int"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("val result"));
        assert!(!result.content.contains("println"));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"
package com.example

class UserService(private val repository: UserRepository) {

    fun findById(id: Int): User? {
        if (id <= 0) {
            return null
        }
        val user = repository.find(id)
        if (user == null) {
            throw IllegalStateException("not found")
        }
        return user
    }

    fun create(name: String): User {
        val existing = repository.findByName(name)
        if (existing != null) {
            throw IllegalStateException("exists")
        }
        return repository.save(User(name))
    }

    fun delete(id: Int): Boolean {
        val user = findById(id) ?: return false
        repository.delete(id)
        return true
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_kotlin(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.functions_collapsed >= 3);
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("fun findById(id: Int): User?"));
        assert!(result.content.contains("fun create(name: String): User"));
        assert!(result.content.contains("fun delete(id: Int): Boolean"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("repository.find"));
    }

    #[test]
    fn test_preserves_package_and_imports() {
        let code = r#"
package com.example.app

import kotlin.collections.List
import kotlinx.coroutines.runBlocking

fun main() {
    println("hello")
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_kotlin(code, &opts).unwrap();
        assert!(result.content.contains("package com.example.app"));
        assert!(result.content.contains("import kotlin.collections.List"));
        assert!(
            result
                .content
                .contains("import kotlinx.coroutines.runBlocking")
        );
    }

    #[test]
    fn test_preserves_annotations_and_data_class() {
        let code = r#"
package com.example

import javax.persistence.Entity

@Entity
data class User(
    val id: Long,
    val name: String,
    val email: String,
)

interface UserRepository {
    fun findById(id: Long): User?
    fun save(user: User): User
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_kotlin(code, &opts).unwrap();
        assert!(result.content.contains("@Entity"));
        assert!(result.content.contains("data class User"));
        assert!(result.content.contains("interface UserRepository"));
        assert!(result.content.contains("fun findById(id: Long): User?"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
package com.example.service

import com.example.repository.UserRepository
import org.springframework.stereotype.Service

@Service
class UserService(
    private val repository: UserRepository,
) {

    fun findById(id: Long): User? {
        if (id <= 0) {
            return null
        }
        val user = repository.findById(id)
        if (user == null) {
            return null
        }
        return user
    }

    fun create(name: String, email: String): User {
        require(name.isNotEmpty()) { "Name is required" }
        require(email.contains("@")) { "Invalid email" }
        val existing = repository.findByEmail(email)
        if (existing != null) {
            throw IllegalStateException("User already exists")
        }
        return repository.save(User(name = name, email = email))
    }

    fun delete(id: Long): Boolean {
        val user = findById(id) ?: return false
        repository.delete(id)
        return true
    }

    fun listAll(): List<User> {
        return repository.findAll()
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_kotlin(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Kotlin module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("fun findById"));
        assert!(result.content.contains("fun create"));
        assert!(result.content.contains("fun listAll"));
    }
}
