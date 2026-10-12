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

/// Walk the tree calling `extract` on every node; collect the `Some` ranges.
/// Ranges are returned sorted by start byte.
///
/// This is the most flexible form: languages whose grammar doesn't use a
/// `body` field (e.g. Kotlin) can look up the body child by kind directly.
///
/// Nested functions are handled naturally: since we sort by start and skip
/// overlaps during replacement, an outer body that contains an inner one
/// simply swallows the inner range.
pub fn collect_ranges<F>(root: Node, extract: F) -> Vec<Range>
where
    F: Fn(Node) -> Option<Range>,
{
    let mut ranges = Vec::new();
    collect_recursive(root, &extract, &mut ranges);
    ranges.sort_by_key(|(s, _)| *s);
    ranges
}

fn collect_recursive<F>(node: Node, extract: &F, ranges: &mut Vec<Range>)
where
    F: Fn(Node) -> Option<Range>,
{
    if let Some(range) = extract(node) {
        ranges.push(range);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_recursive(child, extract, ranges);
    }
}

/// Convenience wrapper: collect the `body` field of every node whose kind
/// matches `is_function`, restricted to block-like bodies.
pub fn collect_body_ranges<F>(root: Node, is_function: F) -> Vec<Range>
where
    F: Fn(&str) -> bool,
{
    collect_ranges(root, |node| {
        if !is_function(node.kind()) {
            return None;
        }
        let body = node.child_by_field_name("body")?;
        if is_block_like(body.kind()) {
            Some((body.start_byte(), body.end_byte()))
        } else {
            None
        }
    })
}

/// Kinds that look like a statement block we can replace with `{ /* ... */ }`.
pub fn is_block_like(kind: &str) -> bool {
    matches!(
        kind,
        "block" | "statement_block" | "function_body" | "compound_statement" | "body_statement"
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
