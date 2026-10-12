//! AST-aware C# compressor
//!
//! Uses `tree-sitter-c-sharp` to parse the source, finds method/constructor
//! bodies, and replaces their byte ranges with `{ /* ... */ }`. Everything
//! else is preserved verbatim — this is far more robust than re-rendering.
//!
//! Preserves: namespaces, using directives, attributes, class/interface/struct
//! declarations, field declarations, property declarations, method signatures.
//! Collapses: method bodies, constructor bodies.

use super::ast_common::apply_replacements;
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

/// Result of C# AST compression
#[derive(Debug, Clone)]
pub struct CSharpCompressResult {
    pub content: String,
    pub methods_collapsed: usize,
    pub classes_preserved: usize,
    pub interfaces_preserved: usize,
    pub attributes_preserved: usize,
}

/// Compress C# source using AST awareness.
pub fn compress_csharp(
    content: &str,
    _options: &CompressionOptions,
) -> Result<CSharpCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter C# language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse C#"))?;

    let mut stats = CSharpStats::default();
    let mut replacements: Vec<(usize, usize)> = Vec::new();

    collect_replacements(tree.root_node(), &mut replacements, &mut stats);
    replacements.sort_by_key(|(s, _)| *s);

    let (compressed, _applied) = apply_replacements(content, &replacements);

    Ok(CSharpCompressResult {
        content: compressed,
        methods_collapsed: stats.methods,
        classes_preserved: stats.classes,
        interfaces_preserved: stats.interfaces,
        attributes_preserved: stats.attributes,
    })
}

#[derive(Default)]
struct CSharpStats {
    methods: usize,
    classes: usize,
    interfaces: usize,
    attributes: usize,
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_c_sharp::LANGUAGE.into())
        .clone()
}

/// Walk the tree, collecting (start, end) byte ranges of bodies to replace,
/// and counting what we see.
fn collect_replacements(
    node: Node,
    replacements: &mut Vec<(usize, usize)>,
    stats: &mut CSharpStats,
) {
    match node.kind() {
        "method_declaration" | "constructor_declaration" | "local_function_statement" => {
            stats.methods += 1;
            // Replace the body block with a placeholder
            if let Some(body) = node.child_by_field_name("body") {
                // Preserve indentation: find the start of the body line
                replacements.push((body.start_byte(), body.end_byte()));
            }
        }
        "class_declaration" => stats.classes += 1,
        "interface_declaration" => stats.interfaces += 1,
        "struct_declaration" | "record_declaration" => stats.classes += 1,
        "attribute_list" | "attribute" => stats.attributes += 1,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_replacements(child, replacements, stats);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_method() {
        let code = r#"
using System;

namespace Demo
{
    public class Calculator
    {
        public int Add(int a, int b)
        {
            var result = a + b;
            Console.WriteLine($"Adding {a} + {b} = {result}");
            if (a > 100)
            {
                return b;
            }
            return result * 2;
        }
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.methods_collapsed >= 1);
        assert!(result.content.contains("class Calculator"));
        assert!(result.content.contains("public int Add"));
        assert!(result.content.contains("int a, int b"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("var result"));
        assert!(!result.content.contains("Console.WriteLine"));
    }

    #[test]
    fn test_collapse_class_with_multiple_methods() {
        let code = r#"
using System.Collections.Generic;

namespace Service
{
    public class UserService
    {
        private readonly Dictionary<int, string> _cache = new();

        public string Get(int id)
        {
            if (_cache.TryGetValue(id, out var cached))
                return cached;
            var user = LoadUser(id);
            _cache[id] = user;
            return user;
        }

        public bool Delete(int id)
        {
            if (!_cache.ContainsKey(id))
                return false;
            _cache.Remove(id);
            return true;
        }

        private string LoadUser(int id) { return $"user-{id}"; }
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.methods_collapsed >= 3);
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("public string Get"));
        assert!(result.content.contains("public bool Delete"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("_cache.TryGetValue"));
    }

    #[test]
    fn test_preserves_using_and_namespace() {
        let code = r#"
using System;
using System.Linq;
using System.Threading.Tasks;

namespace MyApp.Services
{
    public class Helper
    {
        public void DoWork() { /* ... */ }
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        assert!(result.content.contains("using System"));
        assert!(result.content.contains("using System.Linq"));
        assert!(result.content.contains("namespace MyApp.Services"));
    }

    #[test]
    fn test_preserves_interface() {
        let code = r#"
namespace MyApp
{
    public interface IUserRepository
    {
        string Get(int id);
        bool Save(int id, string name);
        void Delete(int id);
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        assert!(result.interfaces_preserved >= 1);
        assert!(result.content.contains("interface IUserRepository"));
        assert!(result.content.contains("string Get(int id)"));
    }

    #[test]
    fn test_preserves_attributes() {
        let code = r#"
namespace MyApp
{
    [ApiController]
    [Route("api/[controller]")]
    public class UserController
    {
        [HttpGet("{id}")]
        public string Get(int id) { return $"user-{id}"; }
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        assert!(result.attributes_preserved >= 3);
        assert!(result.content.contains("[ApiController]"));
        assert!(result.content.contains("[Route("));
        assert!(result.content.contains("[HttpGet("));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace MyApp.Services
{
    public interface IUserRepository
    {
        Task<User> GetByIdAsync(int id);
        Task<User> CreateAsync(User user);
        Task<bool> DeleteAsync(int id);
        Task<List<User>> ListAsync();
    }

    public class UserService : IUserRepository
    {
        private readonly IUserRepository _repo;
        private readonly ILogger<UserService> _logger;

        public UserService(IUserRepository repo, ILogger<UserService> logger)
        {
            _repo = repo;
            _logger = logger;
        }

        public async Task<User> GetByIdAsync(int id)
        {
            if (id <= 0)
            {
                _logger.LogWarning($"Invalid id: {id}");
                return null;
            }
            var user = await _repo.GetByIdAsync(id);
            if (user == null)
            {
                _logger.LogWarning($"User not found: {id}");
                return null;
            }
            return user;
        }

        public async Task<User> CreateAsync(User user)
        {
            if (user == null)
            {
                throw new ArgumentNullException(nameof(user));
            }
            var existing = await _repo.GetByIdAsync(user.Id);
            if (existing != null)
            {
                throw new InvalidOperationException("User already exists");
            }
            return await _repo.CreateAsync(user);
        }

        public async Task<bool> DeleteAsync(int id)
        {
            var user = await _repo.GetByIdAsync(id);
            if (user == null)
            {
                return false;
            }
            return await _repo.DeleteAsync(id);
        }

        public async Task<List<User>> ListAsync()
        {
            return await _repo.ListAsync();
        }
    }
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_csharp(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world C# module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("interface IUserRepository"));
        assert!(result.content.contains("GetByIdAsync"));
        assert!(result.content.contains("CreateAsync"));
        assert!(result.content.contains("DeleteAsync"));
    }
}
