//! Context Compressor
//!
//! Intelligent context reduction with semantic preservation.

use crate::types::{CompressedContext, CompressionOptions, SourceRef, SourceType};
use anyhow::Result;

/// Compress context while preserving meaning
pub fn compress(content: &str, options: CompressionOptions) -> Result<CompressedContext> {
    let original_tokens = estimate_tokens(content);
    
    // Apply compression strategies
    let compressed = apply_compression(content, &options)?;
    
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

/// Estimate token count (rough approximation)
fn estimate_tokens(content: &str) -> usize {
    // Rough estimate: ~4 chars per token for English
    // For code, it's more like 3-4 chars per token
    content.chars().count() / 3
}

/// Apply compression strategies
fn apply_compression(content: &str, options: &CompressionOptions) -> Result<String> {
    let mut result = content.to_string();

    // Strategy 1: Remove extra whitespace
    result = compress_whitespace(&result);

    // Strategy 2: Remove comments (if configured)
    if !options.preserve_types {
        result = remove_comments(&result);
    }

    // Strategy 3: Shorten if over max tokens
    if estimate_tokens(&result) > options.max_tokens {
        result = truncate_to_tokens(&result, options.max_tokens);
    }

    Ok(result)
}

/// Compress whitespace
fn compress_whitespace(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut last_was_whitespace = false;

    for ch in content.chars() {
        if ch.is_whitespace() {
            if !last_was_whitespace {
                result.push(' ');
                last_was_whitespace = true;
            }
        } else {
            result.push(ch);
            last_was_whitespace = false;
        }
    }

    result
}

/// Remove comments from code
fn remove_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '/' => {
                if let Some(&next) = chars.peek() {
                    match next {
                        '/' => {
                            // Single-line comment
                            while let Some(c) = chars.next() {
                                if c == '\n' {
                                    result.push('\n');
                                    break;
                                }
                            }
                        }
                        '*' => {
                            // Multi-line comment
                            chars.next(); // consume *
                            while let Some(c) = chars.next() {
                                if c == '*' {
                                    if let Some(&'/') = chars.peek() {
                                        chars.next();
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            result.push(ch);
                        }
                    }
                } else {
                    result.push(ch);
                }
            }
            '#' => {
                // Python/shell comment
                while let Some(c) = chars.next() {
                    if c == '\n' {
                        result.push('\n');
                        break;
                    }
                }
            }
            '/' => {
                if let Some(&next) = chars.peek() {
                    if next == '/' || next == '*' {
                        continue;
                    }
                }
                result.push(ch);
            }
            _ => result.push(ch),
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
    fn test_compress() {
        let options = CompressionOptions::default();
        let content = "fn   main() { \n    println!(\"hello\"); \n}";
        let result = compress(&content, options).unwrap();
        assert!(result.compressed_tokens < result.original_tokens);
    }
}
