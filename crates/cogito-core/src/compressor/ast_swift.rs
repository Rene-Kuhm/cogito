//! AST-aware Swift compressor
//!
//! Uses `tree-sitter-swift` + **byte-range replacement**: find the byte
//! range of every function/method/init/closure body and replace it with
//! `{ /* ... */ }`.
//!
//! Preserved: `import`, class/struct/enum/protocol declarations, property
//! declarations, function signatures with return types, attributes.
//! Collapsed: function bodies, init bodies, deinit bodies, closures.

use super::ast_common::{apply_replacements, collect_body_ranges};
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::Parser;

/// Result of Swift AST compression
#[derive(Debug, Clone)]
pub struct SwiftCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub classes_preserved: usize,
    pub protocols_preserved: usize,
}

/// Compress Swift source using AST awareness.
pub fn compress_swift(content: &str, _options: &CompressionOptions) -> Result<SwiftCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter Swift language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Swift"))?;

    let ranges = collect_body_ranges(tree.root_node(), is_function_node);
    let (compressed, applied) = apply_replacements(content, &ranges);

    let mut stats = SwiftStats::default();
    count_declarations(tree.root_node(), &mut stats);

    Ok(SwiftCompressResult {
        content: compressed,
        functions_collapsed: applied,
        classes_preserved: stats.classes,
        protocols_preserved: stats.protocols,
    })
}

#[derive(Default)]
struct SwiftStats {
    classes: usize,
    protocols: usize,
}

/// Swift function-like node kinds whose body field is a `function_body`.
fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration" | "init_declaration" | "deinit_declaration" | "closure_expression"
    )
}

fn count_declarations(node: tree_sitter::Node, stats: &mut SwiftStats) {
    match node.kind() {
        "class_declaration" | "struct_declaration" | "enum_declaration" | "actor_declaration" => {
            stats.classes += 1
        }
        "protocol_declaration" => stats.protocols += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_declarations(child, stats);
    }
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_swift::LANGUAGE.into())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_function() {
        let code = r#"
import Foundation

func add(a: Int, b: Int) -> Int {
    let result = a + b
    print("debug: \(a) + \(b) = \(result)")
    if a > 100 {
        return b
    }
    return result * 2
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_swift(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("func add"));
        assert!(result.content.contains("a: Int"));
        assert!(result.content.contains("b: Int"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("let result"));
        assert!(!result.content.contains("print("));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"
import Foundation

final class UserService {
    private let repository: UserRepository

    init(repository: UserRepository) {
        self.repository = repository
    }

    func findById(_ id: Int) -> User? {
        if id <= 0 {
            return nil
        }
        let user = repository.find(id)
        if user == nil {
            return nil
        }
        return user
    }

    func delete(_ id: Int) -> Bool {
        guard let _ = findById(id) else {
            return false
        }
        repository.delete(id)
        return true
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_swift(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.functions_collapsed >= 3);
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("func findById(_ id: Int) -> User?"));
        assert!(result.content.contains("func delete(_ id: Int) -> Bool"));
        assert!(result.content.contains("init(repository: UserRepository)"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("self.repository = repository"));
        assert!(!result.content.contains("repository.find("));
    }

    #[test]
    fn test_preserves_imports_and_protocol() {
        let code = r#"
import Foundation
import Combine

protocol UserRepository {
    func find(_ id: Int) -> User?
    func save(_ user: User) throws
    func delete(_ id: Int)
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_swift(code, &opts).unwrap();
        assert!(result.content.contains("import Foundation"));
        assert!(result.content.contains("import Combine"));
        assert!(result.content.contains("protocol UserRepository"));
        assert!(result.content.contains("func find(_ id: Int) -> User?"));
    }

    #[test]
    fn test_preserves_struct_and_properties() {
        let code = r#"
import Foundation

struct User: Codable {
    let id: Int
    let name: String
    let email: String
}

enum Status: String {
    case active
    case inactive
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_swift(code, &opts).unwrap();
        assert!(result.content.contains("struct User: Codable"));
        assert!(result.content.contains("let id: Int"));
        assert!(result.content.contains("enum Status: String"));
        assert!(result.content.contains("case active"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
import Foundation

protocol UserRepository {
    func find(_ id: Int) -> User?
    func findByEmail(_ email: String) -> User?
    func save(_ user: User) throws -> User
    func delete(_ id: Int)
    func all() -> [User]
}

final class UserService {
    private let repository: UserRepository
    private let logger: Logger

    init(repository: UserRepository, logger: Logger) {
        self.repository = repository
        self.logger = logger
    }

    func findById(_ id: Int) -> User? {
        guard id > 0 else {
            logger.warning("Invalid id: \(id)")
            return nil
        }
        guard let user = repository.find(id) else {
            logger.warning("User not found: \(id)")
            return nil
        }
        return user
    }

    func create(name: String, email: String) throws -> User {
        guard !name.isEmpty else {
            throw ServiceError.invalidName
        }
        if let existing = repository.findByEmail(email) {
            throw ServiceError.alreadyExists(existing.id)
        }
        return try repository.save(User(id: 0, name: name, email: email))
    }

    func delete(_ id: Int) -> Bool {
        guard findById(id) != nil else {
            return false
        }
        repository.delete(id)
        return true
    }

    func listAll() -> [User] {
        return repository.all()
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_swift(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Swift module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("protocol UserRepository"));
        assert!(result.content.contains("func findById"));
        assert!(result.content.contains("func create"));
        assert!(result.content.contains("func listAll"));
    }
}
