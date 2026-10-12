//! AST-aware PHP compressor
//!
//! Uses `tree-sitter-php` + **byte-range replacement**: find the byte range
//! of every function/method body and replace it with `{ /* ... */ }`.
//!
//! Preserved: `<?php` tag, `namespace`, `use`, attributes, class/interface/
//! trait declarations, property declarations, method signatures with types.
//! Collapsed: function bodies, method bodies, closure bodies.

use super::ast_common::{apply_replacements, collect_body_ranges};
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::Parser;

/// Result of PHP AST compression
#[derive(Debug, Clone)]
pub struct PhpCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub classes_preserved: usize,
    pub interfaces_preserved: usize,
}

/// Compress PHP source using AST awareness.
pub fn compress_php(content: &str, _options: &CompressionOptions) -> Result<PhpCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter PHP language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse PHP"))?;

    let ranges = collect_body_ranges(tree.root_node(), is_function_node);
    let (compressed, applied) = apply_replacements(content, &ranges);

    let mut stats = PhpStats::default();
    count_declarations(tree.root_node(), &mut stats);

    Ok(PhpCompressResult {
        content: compressed,
        functions_collapsed: applied,
        classes_preserved: stats.classes,
        interfaces_preserved: stats.interfaces,
    })
}

#[derive(Default)]
struct PhpStats {
    classes: usize,
    interfaces: usize,
}

fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition" | "method_declaration" | "anonymous_function_creation_expression"
    )
}

fn count_declarations(node: tree_sitter::Node, stats: &mut PhpStats) {
    match node.kind() {
        "class_declaration" | "trait_declaration" => stats.classes += 1,
        "interface_declaration" => stats.interfaces += 1,
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
        .get_or_init(|| tree_sitter_php::LANGUAGE_PHP.into())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_function() {
        let code = r#"<?php

function add(int $a, int $b): int {
    $result = $a + $b;
    error_log("debug: $a + $b = $result");
    if ($a > 100) {
        return $b;
    }
    return $result * 2;
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_php(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("function add"));
        assert!(result.content.contains("int $a"));
        assert!(result.content.contains("int $b"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("$result ="));
        assert!(!result.content.contains("error_log"));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"<?php

class User {
    private int $id;
    private string $name;

    public function __construct(int $id, string $name) {
        $this->id = $id;
        $this->name = $name;
    }

    public function validate(): bool {
        return !empty($this->name);
    }

    public function getId(): int {
        return $this->id;
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_php(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.functions_collapsed >= 3);
        assert!(result.content.contains("class User"));
        assert!(result.content.contains("private int $id"));
        assert!(result.content.contains("public function validate(): bool"));
        assert!(result.content.contains("public function getId(): int"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("$this->id = $id"));
        assert!(!result.content.contains("empty($this->name)"));
    }

    #[test]
    fn test_preserves_namespace_and_use() {
        let code = r#"<?php

namespace App\Services;

use App\Models\User;
use Psr\Log\LoggerInterface;

class UserService {
    public function run() {
        return true;
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_php(code, &opts).unwrap();
        assert!(result.content.contains("<?php"));
        assert!(result.content.contains("namespace App\\Services"));
        assert!(result.content.contains("use App\\Models\\User"));
        assert!(result.content.contains("use Psr\\Log\\LoggerInterface"));
    }

    #[test]
    fn test_preserves_interface() {
        let code = r#"<?php

namespace App\Contracts;

interface UserRepository {
    public function findById(int $id): ?User;
    public function save(User $user): User;
    public function delete(int $id): void;
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_php(code, &opts).unwrap();
        assert!(result.interfaces_preserved >= 1);
        assert!(result.content.contains("interface UserRepository"));
        assert!(
            result
                .content
                .contains("public function findById(int $id): ?User")
        );
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"<?php

declare(strict_types=1);

namespace App\Services;

use App\Models\User;
use App\Contracts\UserRepository;
use Psr\Log\LoggerInterface;

final class UserService
{
    public function __construct(
        private readonly UserRepository $repository,
        private readonly LoggerInterface $logger,
    ) {
    }

    public function findById(int $id): ?User
    {
        if ($id <= 0) {
            $this->logger->warning("Invalid id: {$id}");
            return null;
        }
        $user = $this->repository->find($id);
        if ($user === null) {
            $this->logger->warning("User not found: {$id}");
            return null;
        }
        return $user;
    }

    public function create(User $user): User
    {
        if ($user->email === '') {
            throw new \InvalidArgumentException('Email required');
        }
        $existing = $this->repository->findByEmail($user->email);
        if ($existing !== null) {
            throw new \RuntimeException('User already exists');
        }
        return $this->repository->save($user);
    }

    public function delete(int $id): bool
    {
        $user = $this->findById($id);
        if ($user === null) {
            return false;
        }
        $this->repository->delete($id);
        return true;
    }

    public function listAll(): array
    {
        return $this->repository->all();
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_php(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world PHP module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("public function findById"));
        assert!(result.content.contains("public function create"));
        assert!(result.content.contains("public function delete"));
        assert!(result.content.contains("public function listAll"));
    }
}
