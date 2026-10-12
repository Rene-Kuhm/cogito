//! Shared helpers for byte-range AST compressors
//!
//! The **byte-range replacement** approach is the preferred strategy for
//! tree-sitter-based compressors: instead of re-rendering a code
//! representation from the AST (which is fragile and loses formatting),
//! we:
//!
//! 1. Parse the source into an AST.
//! 2. Walk the tree and collect `(start_byte, end_byte)` ranges of function
//!    bodies.
//! 3. Copy the source verbatim, replacing only those ranges with
//!    `{ /* ... */ }`.
//!
//! This preserves *everything* else exactly: signature formatting, comments
//! on signatures, decorators/annotations, generics, attributes, string
//! literals, and whitespace.

use tree_sitter::Node;

/// A half-open byte range `[start, end)` to be replaced.
pub type Range = (usize, usize);

/// Apply byte-range replacements to `source`.
///
/// `ranges` must be sorted by start byte. Overlapping (nested) ranges are
/// skipped — the outer range wins, which is what we want when a function
/// contains another function inside its body.
///
/// Returns `(compressed_source, replacements_applied)`.
pub fn apply_replacements(source: &str, ranges: &[Range]) -> (String, usize) {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut applied = 0usize;

    for (start, end) in ranges {
        if *start < cursor || *end > source.len() {
            // Overlapping or out-of-bounds — skip
            continue;
        }
        out.push_str(&source[cursor..*start]);
        out.push_str("{ /* ... */ }");
        cursor = *end;
        applied += 1;
    }

    out.push_str(&source[cursor..]);
    (out, applied)
}

/// Walk the tree collecting the byte range of every `body` field found on
/// nodes matching `is_function`, then return the ranges sorted by start.
///
/// Nested functions are handled naturally: since we sort by start and skip
/// overlaps during replacement, an outer body that contains an inner one
/// simply swallows the inner range.
pub fn collect_body_ranges<F>(root: Node, is_function: F) -> Vec<Range>
where
    F: Fn(&str) -> bool,
{
    let mut ranges = Vec::new();
    collect_recursive(root, &is_function, &mut ranges);
    ranges.sort_by_key(|(s, _)| *s);
    ranges
}

fn collect_recursive<F>(node: Node, is_function: &F, ranges: &mut Vec<Range>)
where
    F: Fn(&str) -> bool,
{
    if is_function(node.kind())
        && let Some(body) = node.child_by_field_name("body")
    {
        // Only replace block-like bodies, never single expressions
        if is_block_like(body.kind()) {
            ranges.push((body.start_byte(), body.end_byte()));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_recursive(child, is_function, ranges);
    }
}

/// Kinds that look like a statement block we can replace with `{ /* ... */ }`.
fn is_block_like(kind: &str) -> bool {
    matches!(
        kind,
        "block"
            | "statement_block"
            | "function_body"
            | "compound_statement"
            | "class_body"
            | "declaration_list"
    )
}

/// Remove lines that are only whitespace at the very start/end.
pub fn trim_blank_edge_lines(s: &str) -> String {
    let trimmed_start = s.trim_start_matches('\n');
    trimmed_start
        .trim_end_matches(['\n', ' ', '\t'])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_replacements_empty() {
        let (out, n) = apply_replacements("hello", &[]);
        assert_eq!(out, "hello");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_apply_replacements_single() {
        let src = "fn foo() { body }";
        // Replace "{ body }" (index 9..17)
        let (out, n) = apply_replacements(src, &[(9, 17)]);
        assert_eq!(out, "fn foo() { /* ... */ }");
        assert_eq!(n, 1);
    }

    #[test]
    fn test_apply_replacements_multiple() {
        let src = "a{b}c{d}e";
        let (out, n) = apply_replacements(src, &[(1, 4), (5, 8)]);
        assert_eq!(out, "a{ /* ... */ }c{ /* ... */ }e");
        assert_eq!(n, 2);
    }

    #[test]
    fn test_apply_replacements_skips_overlaps() {
        let src = "a{outer{inner}}b";
        // Outer range 1..15 (a{outer{inner}}b -> from index 1 to 15)
        // Inner range 7..14 — should be skipped because outer wins
        let (out, n) = apply_replacements(src, &[(1, 15), (7, 14)]);
        assert_eq!(out, "a{ /* ... */ }b");
        assert_eq!(n, 1);
    }
}
