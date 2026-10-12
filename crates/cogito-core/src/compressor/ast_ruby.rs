//! AST-aware Ruby compressor
//!
//! Uses `tree-sitter-ruby` + **byte-range replacement**: find the byte range
//! of every method/block body and replace it with `...`-style placeholder.
//!
//! Ruby blocks use `def ... end` (or `do ... end`), so we replace the body
//! with a single `# /* ... */` comment line, which is valid in any Ruby
//! block position.
//!
//! Preserved: `require`, `class`, `module`, `attr_*`, method signatures,
//! constants, instance variables in the class body.
//! Collapsed: method bodies, block bodies, lambda bodies.

use super::ast_common::{collect_ranges, is_block_like};
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::Parser;

/// Result of Ruby AST compression
#[derive(Debug, Clone)]
pub struct RubyCompressResult {
    pub content: String,
    pub methods_collapsed: usize,
    pub classes_preserved: usize,
}

/// Compress Ruby source using AST awareness.
pub fn compress_ruby(content: &str, _options: &CompressionOptions) -> Result<RubyCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter Ruby language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Ruby"))?;

    let ranges = collect_ranges(tree.root_node(), |node| {
        if !is_function_node(node.kind()) {
            return None;
        }
        let body = node.child_by_field_name("body")?;
        if is_block_like(body.kind()) {
            Some((body.start_byte(), body.end_byte()))
        } else {
            None
        }
    });

    let (compressed, applied) = super::ast_common::apply_replacements(content, &ranges);

    let mut classes = 0usize;
    count_declarations(tree.root_node(), &mut classes);

    Ok(RubyCompressResult {
        content: compressed,
        methods_collapsed: applied,
        classes_preserved: classes,
    })
}

fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "method" | "singleton_method" | "do_block" | "block" | "lambda"
    )
}

fn count_declarations(node: tree_sitter::Node, classes: &mut usize) {
    if matches!(node.kind(), "class" | "module" | "singleton_class") {
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
        .get_or_init(|| tree_sitter_ruby::LANGUAGE.into())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_method() {
        let code = r#"
class Calculator
  def add(a, b)
    result = a + b
    puts "debug: #{a} + #{b} = #{result}"
    if a > 100
      return b
    end
    result * 2
  end
end
"#;
        let opts = CompressionOptions::default();
        let result = compress_ruby(code, &opts).unwrap();
        assert!(result.methods_collapsed >= 1);
        assert!(result.content.contains("class Calculator"));
        assert!(result.content.contains("def add(a, b)"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("result = a + b"));
        assert!(!result.content.contains("puts"));
    }

    #[test]
    fn test_collapse_multiple_methods() {
        let code = r#"
class User
  def initialize(id, name)
    @id = id
    @name = name
  end

  def validate?
    !@name.nil? && !@name.empty?
  end

  def to_s
    "User(#{@id}, #{@name})"
  end
end
"#;
        let opts = CompressionOptions::default();
        let result = compress_ruby(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.methods_collapsed >= 3);
        assert!(result.content.contains("class User"));
        assert!(result.content.contains("def initialize"));
        assert!(result.content.contains("def validate?"));
        assert!(result.content.contains("def to_s"));
        // Bodies should NOT be in output
        assert!(!result.content.contains("@id = id"));
    }

    #[test]
    fn test_preserves_require_and_module() {
        let code = r#"
require 'json'
require_relative 'helper'

module MyApp
  class User
    attr_reader :id, :name

    def initialize(id, name)
      @id = id
      @name = name
    end
  end
end
"#;
        let opts = CompressionOptions::default();
        let result = compress_ruby(code, &opts).unwrap();
        assert!(result.content.contains("require 'json'"));
        assert!(result.content.contains("require_relative"));
        assert!(result.content.contains("module MyApp"));
        assert!(result.content.contains("attr_reader"));
        assert!(result.content.contains("def initialize"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
require 'json'
require 'logger'

module MyApp
  class UserService
    def initialize(repository, logger)
      @repository = repository
      @logger = logger
    end

    def find_by_id(id)
      return nil if id <= 0
      user = @repository.find(id)
      if user.nil?
        @logger.warn("User not found: #{id}")
        return nil
      end
      user
    end

    def create(user)
      raise ArgumentError, "User cannot be nil" if user.nil?
      existing = @repository.find_by_email(user.email)
      unless existing.nil?
        raise "User already exists"
      end
      @repository.save(user)
    end

    def delete(id)
      user = find_by_id(id)
      return false if user.nil?
      @repository.delete(id)
      true
    end

    def list_all
      @repository.all
    end
  end
end
"#;
        let opts = CompressionOptions::default();
        let result = compress_ruby(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Ruby module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("def find_by_id"));
        assert!(result.content.contains("def create"));
        assert!(result.content.contains("def delete"));
        assert!(result.content.contains("def list_all"));
    }
}
