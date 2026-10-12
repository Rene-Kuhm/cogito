//! Context Compressor
//!
//! Intelligent context reduction with semantic preservation.

use crate::types::{CompressedContext, CompressionOptions, Language, SourceRef, SourceType};
use anyhow::Result;

pub mod ast;

/// Compress context while preserving meaning
pub fn compress(content: &str, options: CompressionOptions) -> Result<CompressedContext> {
    compress_with_language(content, options, None)
}

/// Compress with optional language hint for AST-aware compression
pub fn compress_with_language(
    content: &str,
    options: CompressionOptions,
    language: Option<Language>,
) -> Result<CompressedContext> {
    let original_tokens = estimate_tokens(content);

    // Try AST-aware compression first if language is known and is Rust
    let compressed = if matches!(language, Some(Language::Rust)) {
        match ast::compress_rust_only(content, &options) {
            Ok(r) => r.content,
            Err(_) => apply_compression(content, &options)?,
        }
    } else {
        apply_compression(content, &options)?
    };

    let compressed_tokens = estimate_tokens(&compressed);
    let savings = if original_tokens > 0 {
        ((original_tokens - compressed_tokens) as f32 / original_tokens as f32) * 100.0
    } else {
        0.0
    };

    Ok(CompressedContext {
        original_tokens,
        compressed_tokens,
        savings_percent: savings,
        content: compressed,
        sources: vec![SourceRef {
            path: std::path::PathBuf::from("."),
            source_type: SourceType::File,
            relevance: 1.0,
        }],
    })
}

/// Estimate token count (rough approximation based on character count)
fn estimate_tokens(content: &str) -> usize {
    // ~3-4 chars per token for code (slightly higher density than prose)
    content.chars().count() / 3
}

/// Apply compression strategies
fn apply_compression(content: &str, options: &CompressionOptions) -> Result<String> {
    let mut result = content.to_string();

    // Strategy 1: Remove block comments
    if !options.preserve_types {
        result = remove_block_comments(&result);
    }

    // Strategy 2: Remove single-line comments
    if !options.preserve_types {
        result = remove_line_comments(&result);
    }

    // Strategy 3: Compress extra whitespace
    result = compress_whitespace(&result);

    // Strategy 4: Truncate to max tokens if still too large
    if estimate_tokens(&result) > options.max_tokens {
        result = truncate_to_tokens(&result, options.max_tokens);
    }

    Ok(result)
}

/// Remove block comments (/* ... */)
fn remove_block_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Skip until */
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    result.push('\n');
                }
                i += 1;
            }
            i += 2; // skip */
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Remove line comments (// and #)
fn remove_line_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());

    for line in content.lines() {
        let cleaned = strip_line_comment(line);
        result.push_str(&cleaned);
        result.push('\n');
    }

    result
}

/// Strip a single comment from a line, preserving string literals
fn strip_line_comment(line: &str) -> String {
    let mut in_string = false;
    let mut string_char = '\0';
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if in_string {
            result_unchecked_push(&mut String::new(), c);
            if c == '\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' || c == '\'' || c == '`' {
            in_string = true;
            string_char = c;
            i += 1;
            continue;
        }

        // Check for //
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            break;
        }

        // Check for # (Python/Ruby/shell)
        if c == '#' {
            break;
        }

        i += 1;
    }

    // Build result preserving the line up to the comment
    let mut result = String::new();
    let mut in_string2 = false;
    let mut string_char2 = '\0';
    let mut j = 0;
    let bytes = line.as_bytes();

    while j < bytes.len() {
        let c = bytes[j] as char;

        if in_string2 {
            result.push(c);
            if c == '\\' && j + 1 < bytes.len() {
                result.push(bytes[j + 1] as char);
                j += 2;
                continue;
            }
            if c == string_char2 {
                in_string2 = false;
            }
            j += 1;
            continue;
        }

        if c == '"' || c == '\'' || c == '`' {
            in_string2 = true;
            string_char2 = c;
            result.push(c);
            j += 1;
            continue;
        }

        if c == '/' && j + 1 < bytes.len() && bytes[j + 1] == b'/' {
            break;
        }

        if c == '#' {
            break;
        }

        result.push(c);
        j += 1;
    }

    result
}

// Helper - we don't really need this, removing
fn result_unchecked_push(_s: &mut String, _c: char) {}

/// Compress whitespace
fn compress_whitespace(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut last_was_space = false;

    for ch in content.chars() {
        if ch == '\n' {
            if !last_was_space {
                result.push('\n');
                last_was_space = true;
            }
        } else if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }

    result
}

/// Truncate content to approximately max_tokens
fn truncate_to_tokens(content: &str, max_tokens: usize) -> String {
    let mut result = String::with_capacity(max_tokens * 3);
    let mut token_count = 0;

    for ch in content.chars() {
        result.push(ch);
        if ch.is_whitespace() || ch == '\n' {
            token_count += 1;
            if token_count >= max_tokens {
                break;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        let content = "fn main() { println!(\"hello\"); }";
        let tokens = estimate_tokens(content);
        assert!(tokens > 0);
    }

    #[test]
    fn test_compress_whitespace() {
        let input = "fn   main() {\n    \n    println!();\n}";
        let result = compress_whitespace(input);
        assert!(!result.contains("   "));
        assert!(result.contains('\n'));
    }

    #[test]
    fn test_remove_line_comments() {
        let input = "fn main() { // comment\n println!(); }";
        let result = remove_line_comments(input);
        assert!(!result.contains("// comment"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_preserve_strings_with_comment_chars() {
        let input = r#"let url = "http://example.com";"#;
        let result = remove_line_comments(input);
        assert!(result.contains("http://example.com"));
    }

    #[test]
    fn test_compress() {
        let options = CompressionOptions::default();
        let content = "fn   main() { \n    println!(\"hello\"); \n}";
        let result = compress(content, options).unwrap();
        assert!(result.compressed_tokens <= result.original_tokens);
    }
}
