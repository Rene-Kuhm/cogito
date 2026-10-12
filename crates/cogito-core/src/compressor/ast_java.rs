//! AST-aware Java compressor
//!
//! Uses `tree-sitter-java` to parse the source, finds method/constructor
//! bodies, and replaces their byte ranges with `{ /* ... */ }`. Everything
//! else is preserved verbatim — this is far more robust than re-rendering.
//!
//! Preserves: packages, imports, annotations, class/interface/enum/record
//! declarations, field declarations, method signatures.
//! Collapses: method bodies, constructor bodies.

use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

/// Result of Java AST compression
#[derive(Debug, Clone)]
pub struct JavaCompressResult {
    pub content: String,
    pub methods_collapsed: usize,
    pub classes_preserved: usize,
    pub interfaces_preserved: usize,
    pub annotations_preserved: usize,
}

/// Compress Java source using AST awareness.
pub fn compress_java(content: &str, _options: &CompressionOptions) -> Result<JavaCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter Java language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Java"))?;

    let mut stats = JavaStats::default();
    let mut replacements: Vec<(usize, usize)> = Vec::new();

    collect_replacements(tree.root_node(), &mut replacements, &mut stats);

    replacements.sort_by_key(|(s, _)| *s);
    let compressed = apply_replacements(content, &replacements);

    Ok(JavaCompressResult {
        content: compressed,
        methods_collapsed: stats.methods,
        classes_preserved: stats.classes,
        interfaces_preserved: stats.interfaces,
        annotations_preserved: stats.annotations,
    })
}

#[derive(Default)]
struct JavaStats {
    methods: usize,
    classes: usize,
    interfaces: usize,
    annotations: usize,
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_java::LANGUAGE.into())
        .clone()
}

fn collect_replacements(node: Node, replacements: &mut Vec<(usize, usize)>, stats: &mut JavaStats) {
    match node.kind() {
        "method_declaration" | "constructor_declaration" => {
            stats.methods += 1;
            if let Some(body) = node.child_by_field_name("body") {
                replacements.push((body.start_byte(), body.end_byte()));
            }
        }
        "class_declaration" | "enum_declaration" | "record_declaration" => {
            stats.classes += 1;
        }
        "interface_declaration" => stats.interfaces += 1,
        "annotation" | "marker_annotation" => stats.annotations += 1,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_replacements(child, replacements, stats);
    }
}

fn apply_replacements(source: &str, ranges: &[(usize, usize)]) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (start, end) in ranges {
        if *start < cursor {
            continue;
        }
        out.push_str(&source[cursor..*start]);
        out.push_str("{ /* ... */ }");
        cursor = *end;
    }
    out.push_str(&source[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_method() {
        let code = r#"
package com.example.demo;

import java.util.List;

public class Calculator {
    public int add(int a, int b) {
        int result = a + b;
        System.out.println("debug: " + a + "+" + b);
        if (a > 100) {
            return b;
        }
        return result * 2;
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.methods_collapsed >= 1);
        assert!(result.content.contains("class Calculator"));
        assert!(result.content.contains("public int add"));
        assert!(result.content.contains("int a, int b"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("int result ="));
        assert!(!result.content.contains("System.out.println"));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"
package com.example.service;

import java.util.HashMap;
import java.util.Map;

public class UserService {
    private final Map<Integer, String> cache = new HashMap<>();

    public String get(int id) {
        if (cache.containsKey(id)) {
            return cache.get(id);
        }
        String user = loadUser(id);
        cache.put(id, user);
        return user;
    }

    public boolean delete(int id) {
        if (!cache.containsKey(id)) {
            return false;
        }
        cache.remove(id);
        return true;
    }

    private String loadUser(int id) {
        return "user-" + id;
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.methods_collapsed >= 3);
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("public String get"));
        assert!(result.content.contains("public boolean delete"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("cache.containsKey"));
    }

    #[test]
    fn test_preserves_imports() {
        let code = r#"
package com.example.app;

import java.util.List;
import java.util.Optional;

public class Helper {
    public void doWork() { }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        assert!(result.content.contains("package com.example.app"));
        assert!(result.content.contains("import java.util.List"));
        assert!(result.content.contains("import java.util.Optional"));
    }

    #[test]
    fn test_preserves_interface() {
        let code = r#"
package com.example.repository;

import java.util.List;
import java.util.Optional;

public interface UserRepository {
    Optional<User> findById(int id);
    User save(User user);
    void deleteById(int id);
    List<User> findAll();
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        assert!(result.interfaces_preserved >= 1);
        assert!(result.content.contains("interface UserRepository"));
        assert!(result.content.contains("Optional<User> findById(int id)"));
        assert!(result.content.contains("List<User> findAll"));
    }

    #[test]
    fn test_preserves_annotations() {
        let code = r#"
package com.example.controller;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api/users")
public class UserController {
    @GetMapping("/{id}")
    public String get(@PathVariable int id) {
        return "user-" + id;
    }

    @PostMapping
    public String create() {
        return "created";
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        assert!(result.annotations_preserved >= 3);
        assert!(result.content.contains("@RestController"));
        assert!(result.content.contains("@RequestMapping"));
        assert!(result.content.contains("@GetMapping"));
        assert!(result.content.contains("@PostMapping"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
package com.example.service;

import java.util.List;
import java.util.Optional;
import org.springframework.stereotype.Service;

@Service
public class UserService {

    private final UserRepository repository;

    public UserService(UserRepository repository) {
        this.repository = repository;
    }

    public Optional<User> findById(long id) {
        if (id <= 0) {
            return Optional.empty();
        }
        Optional<User> user = repository.findById(id);
        if (user.isEmpty()) {
            return Optional.empty();
        }
        return user;
    }

    public User create(User user) {
        if (user == null) {
            throw new IllegalArgumentException("User cannot be null");
        }
        Optional<User> existing = repository.findByEmail(user.getEmail());
        if (existing.isPresent()) {
            throw new IllegalStateException("User already exists");
        }
        return repository.save(user);
    }

    public boolean delete(long id) {
        Optional<User> user = repository.findById(id);
        if (user.isEmpty()) {
            return false;
        }
        repository.deleteById(id);
        return true;
    }

    public List<User> listAll() {
        return repository.findAll();
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_java(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Java module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("public Optional<User> findById"));
        assert!(result.content.contains("public User create"));
        assert!(result.content.contains("public boolean delete"));
        assert!(result.content.contains("public List<User> listAll"));
    }
}
