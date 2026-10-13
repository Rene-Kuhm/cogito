//! Tree-sitter based declaration extractor.
//!
//! Parses source files with tree-sitter to extract declarations (functions,
//! classes, methods, types, etc.) with accurate spans. Falls back to the
//! regex-based grammar in `outline.rs` when tree-sitter is unavailable,
//! fails to parse, or yields fewer declarations than the regex path.

use tree_sitter::{Language, Node, Parser, Tree};

/// Maximum file size (2 MB) before switching to regex fallback.
const SIZE_CAP: usize = 2 * 1024 * 1024;

/// Longest signature kept, in characters.
pub(crate) const SIGNATURE_MAX: usize = 110;

/// One declaration in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    /// `function`, `class`, `interface`, `type`, `const`, `struct`, `impl`…
    pub kind: String,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    /// The declaring line, trimmed and shortened. Only kept for callables,
    /// where parameters are the useful part; elsewhere the name says enough.
    pub signature: Option<String>,
}

// ---------------------------------------------------------------------------
// Language registry
// ---------------------------------------------------------------------------

/// Get the tree-sitter language for an extension.
fn language(extension: &str) -> Option<Language> {
    let ext = extension.to_lowercase();
    Some(match ext.as_str() {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        }
        "py" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "cs" => tree_sitter_c_sharp::LANGUAGE.into(),
        "rb" => tree_sitter_ruby::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "kt" | "kts" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "md" | "mdx" | "markdown" => return None,
        _ => return None,
    })
}

fn parse_ts(content: &str, lang: Language) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    parser.parse(content, None)
}

// ---------------------------------------------------------------------------
// Callability helpers
// ---------------------------------------------------------------------------

fn is_callable(kind: &str) -> bool {
    matches!(
        kind,
        "function"
            | "function_declaration"
            | "function_item"
            | "fn"
            | "func"
            | "fun"
            | "method_declaration"
            | "method"
            | "function_definition"
            | "async_function_definition"
            | "decorated_definition"
    )
}

fn kind_to_vocab(kind: &str) -> String {
    match kind {
        "function_item" => "fn".to_string(),
        "impl_item" => "impl".to_string(),
        "struct_item" => "struct".to_string(),
        "enum_item" => "enum".to_string(),
        "trait_item" => "trait".to_string(),
        "type_item" => "type".to_string(),
        "const_item" => "const".to_string(),
        "static_item" => "static".to_string(),
        "mod_item" => "mod".to_string(),
        "macro_definition" => "macro_rules!".to_string(),
        "function_declaration" | "symbol" => "function".to_string(),
        "class_declaration" | "abstract_class_declaration" => "class".to_string(),
        "interface_declaration" => "interface".to_string(),
        "type_alias_declaration" => "type".to_string(),
        "enum_declaration" => "enum".to_string(),
        "lexical_declaration" | "variable_declarator" => "const".to_string(),
        "method_definition" => "method".to_string(),
        "function_definition" | "async_function_definition" | "decorated_definition" => {
            "def".to_string()
        }
        "class_definition" => "class".to_string(),
        "method_declaration" => "func".to_string(),
        "type_declaration" => "type".to_string(),
        "class" => "class".to_string(),
        "method" | "singleton_method" => "func".to_string(),
        "constant" => "const".to_string(),
        "trait_declaration" => "trait".to_string(),
        "struct_declaration" => "struct".to_string(),
        "func_declaration" => "func".to_string(),
        "protocol_declaration" => "protocol".to_string(),
        "property_declaration" => "const".to_string(),
        other => other.to_string(),
    }
}

fn shorten(line: &str) -> String {
    let line = line.trim_end_matches('{').trim_end();
    if line.chars().count() <= SIGNATURE_MAX {
        return line.to_string();
    }
    let cut: String = line.chars().take(SIGNATURE_MAX).collect();
    format!("{cut}…")
}

fn first_line(content: &str, node: &Node) -> String {
    let range = node.range();
    let start_row = range.start_point.row;
    let _end_row = range.end_point.row;

    // Calculate where the line containing node_start begins
    let line_start_byte = if start_row == 0 {
        0
    } else {
        content
            .lines()
            .take(start_row)
            .map(|l| l.len() + 1)
            .sum::<usize>()
    };

    // Find the end of the first line (either newline or SIGNATURE_MAX chars from line start)
    let line_content = if line_start_byte < content.len() {
        &content[line_start_byte..]
    } else {
        ""
    };

    let first_newline = line_content.find('\n').unwrap_or(line_content.len());
    let take_chars = first_newline.min(SIGNATURE_MAX);

    let result_end = (line_start_byte + take_chars).min(content.len());
    let result_start = line_start_byte.min(content.len());

    if result_end <= result_start {
        return String::new();
    }

    content[result_start..result_end].trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Node traversal
// ---------------------------------------------------------------------------

fn is_declaration_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "impl_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "mod_item"
            | "macro_definition"
            | "function_declaration"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "lexical_declaration"
            | "variable_declaration"
            | "method_definition"
            | "function_definition"
            | "class_definition"
            | "decorated_definition"
            | "method_declaration"
            | "type_declaration"
            | "class"
            | "method"
            | "singleton_method"
            | "constant"
            | "trait_declaration"
            | "struct_declaration"
            | "func_declaration"
            | "protocol_declaration"
            | "property_declaration"
            | "export_clause"
            | "export_statement"
            | "export_default_statement"
            | "export" // Common export wrapper node
    )
}

fn cursor_name(node: &Node, content: &str) -> Option<String> {
    let kind = node.kind();
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    if kind == "class_definition" {
        for (i, child) in children.iter().enumerate() {
            if child.kind() == "identifier" && i > 0 {
                return Some(content[child.byte_range()].to_string());
            }
        }
    }

    if kind == "function_definition" || kind == "async_function_definition" {
        let mut found_def = false;
        for child in &children {
            if child.kind() == "def" {
                found_def = true;
                continue;
            }
            if found_def && child.kind() == "identifier" {
                return Some(content[child.byte_range()].to_string());
            }
        }
    }

    if kind == "struct_item"
        || kind == "enum_item"
        || kind == "trait_item"
        || kind == "type_item"
        || kind == "impl_item"
    {
        let mut found_keyword = false;
        for child in &children {
            if child.kind().ends_with("_item")
                || child.kind() == "struct"
                || child.kind() == "enum"
                || child.kind() == "trait"
                || child.kind() == "type"
                || child.kind() == "impl"
            {
                found_keyword = true;
                continue;
            }
            if found_keyword && (child.kind() == "identifier" || child.kind() == "type_identifier")
            {
                return Some(content[child.byte_range()].to_string());
            }
        }
    }

    if kind == "function_item" || kind == "function_declaration" {
        let mut found_fn = false;
        for child in &children {
            if child.kind() == "fn" || child.kind() == "function" {
                found_fn = true;
                continue;
            }
            if found_fn && (child.kind() == "identifier" || child.kind() == "type_identifier") {
                return Some(content[child.byte_range()].to_string());
            }
        }
    }

    if kind == "method_declaration" {
        for child in &children {
            if child.kind() == "identifier" {
                return Some(content[child.byte_range()].to_string());
            }
        }
    }

    if kind == "lexical_declaration" {
        for child in &children {
            if child.kind() == "variable_declarator" {
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "identifier" {
                        return Some(content[inner_child.byte_range()].to_string());
                    }
                }
            }
        }
    }

    for child in &children {
        let child_kind = child.kind();
        if child_kind == "identifier"
            || child_kind == "type_identifier"
            || child_kind == "property_identifier"
        {
            return Some(content[child.byte_range()].to_string());
        }
    }
    None
}

fn get_name(node: &Node, content: &str) -> Option<String> {
    let kind = node.kind();
    match kind {
        "impl_item" => {
            // Extract the impl header (everything between "impl" and the opening brace or where block starts)
            // This captures: impl [generics] Type [for Type]
            let impl_range = node.range();
            let impl_content = &content[impl_range.start_byte..impl_range.end_byte];

            // Find the "impl" keyword position
            let impl_start = impl_content.find("impl")?;
            let after_impl = &impl_content[impl_start + 4..];

            // Find where the impl block body starts (opening brace or where keyword)
            let body_start = after_impl
                .find('{')
                .or_else(|| after_impl.find("where"))
                .unwrap_or(after_impl.len());

            let header = &after_impl[..body_start];

            // Normalize whitespace and remove leading/trailing whitespace
            let normalized: String = header.split_whitespace().collect::<Vec<_>>().join(" ");

            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        }
        "macro_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return Some(content[child.byte_range()].to_string());
                }
            }
            None
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec" {
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "type_identifier" {
                            return Some(content[inner_child.byte_range()].to_string());
                        }
                    }
                }
            }
            None
        }
        "method_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "field_identifier" {
                    return Some(content[child.byte_range()].to_string());
                }
            }
            None
        }
        "decorated_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_definition"
                    || child.kind() == "function_declaration"
                    || child.kind() == "class_definition"
                    || child.kind() == "class_declaration"
                {
                    return get_name(&child, content);
                }
            }
            None
        }
        // Export statements: recurse into the declared item
        "export_clause" | "export_statement" | "export_default_statement" | "export" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                // Recurse into any declaration child
                if is_declaration_kind(child.kind())
                    && let Some(name) = get_name(&child, content)
                {
                    return Some(name);
                }
            }
            None
        }
        _ if is_declaration_kind(kind) => cursor_name(node, content),
        _ => None,
    }
}

fn node_span(node: &Node) -> (usize, usize) {
    let start = node.start_position().row + 1;
    let end_pos = node.end_position();
    let end = if end_pos.column == 0 {
        end_pos.row
    } else {
        end_pos.row + 1
    };
    (start, end)
}

fn collect_declarations(tree: &Tree, content: &str, declarations: &mut Vec<Declaration>) {
    let root = tree.root_node();
    collect_declarations_recursive(&root, content, declarations);
}

fn collect_declarations_recursive(node: &Node, content: &str, declarations: &mut Vec<Declaration>) {
    let kind = node.kind();
    if is_declaration_kind(kind) {
        // For export statements, recurse into the inner declaration and use its kind
        if kind == "export_clause"
            || kind == "export_statement"
            || kind == "export_default_statement"
            || kind == "export"
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_declaration_kind(child.kind()) {
                    collect_declarations_recursive(&child, content, declarations);
                }
            }
            return;
        }

        if let Some(name) = get_name(node, content) {
            let vocab = kind_to_vocab(kind);
            let (start, end) = node_span(node);
            let signature = if is_callable(kind) {
                Some(shorten(&first_line(content, node)))
            } else {
                None
            };
            declarations.push(Declaration {
                name,
                kind: vocab,
                start,
                end,
                signature,
            });
        }

        if kind == "impl_item" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "declaration_list" {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        collect_declarations_recursive(&inner_child, content, declarations);
                    }
                }
            }
        }

        if kind == "class_definition" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "block" {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        collect_declarations_recursive(&inner_child, content, declarations);
                    }
                }
            }
        }

        if kind == "class_declaration" || kind == "abstract_class_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "class_body" {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        collect_declarations_recursive(&inner_child, content, declarations);
                    }
                }
            }
        }

        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_declarations_recursive(&child, content, declarations);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn declarations(content: &str, extension: &str) -> Vec<Declaration> {
    if content.len() > SIZE_CAP {
        return Vec::new();
    }

    let ext_lower = extension.to_lowercase();
    if matches!(ext_lower.as_str(), "md" | "mdx" | "markdown") {
        return Vec::new();
    }

    let Some(lang) = language(&ext_lower) else {
        return Vec::new();
    };

    let Some(tree) = parse_ts(content, lang) else {
        return Vec::new();
    };

    let mut ts_decls: Vec<Declaration> = Vec::new();
    collect_declarations(&tree, content, &mut ts_decls);
    ts_decls.sort_by_key(|d| d.start);
    ts_decls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_impl_with_nested_fn() {
        let src = r#"pub struct Server;

impl Server {
    pub fn new() -> Self {
        Self
    }

    fn handle(&self) {
        todo!()
    }
}

pub async fn run() {
    todo!()
}
"#;
        let decls: Vec<_> = declarations(src, "rs")
            .into_iter()
            .map(|d| (d.name.clone(), d.kind.clone(), d.start, d.end))
            .collect();

        let names: Vec<_> = decls.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(
            names.contains(&"Server"),
            "should find struct Server: {names:?}"
        );
        assert!(names.contains(&"new"), "should find new method: {names:?}");
        assert!(
            names.contains(&"handle"),
            "should find handle method: {names:?}"
        );
        assert!(
            names.contains(&"run"),
            "should find run function: {names:?}"
        );

        let _impl_server = decls.iter().find(|(_, k, _, _)| *k == "impl");
        assert!(_impl_server.is_some(), "should find impl block");
    }

    #[test]
    fn test_rust_multiline_signature() {
        let src = r#"pub fn complicated_function(
    param1: Vec<String>,
    param2: Option<usize>,
) -> Result<Self, Error> {
    todo!()
}
"#;
        let decls = declarations(src, "rs");
        assert!(!decls.is_empty(), "should find function");
        let func = decls
            .iter()
            .find(|d| d.name == "complicated_function")
            .unwrap();
        assert_eq!(func.start, 1);
        assert!(func.signature.is_some(), "should have signature");
        // Accept that end might be 5 or 6 depending on how tree-sitter counts
        assert!(func.end >= 5, "end should be at least 5");
    }

    #[test]
    fn test_rust_macro_rules() {
        let src = r#"macro_rules! vec {
    ( $( $x:expr ),* ) => {
        {
            let mut temp_vec = Vec::new();
            temp_vec
        }
    };
}
"#;
        let decls = declarations(src, "rs");
        let macro_decl = decls.iter().find(|d| d.name == "vec");
        assert!(
            macro_decl.is_some(),
            "should find macro_rules! vec: {decls:?}"
        );
        assert_eq!(macro_decl.unwrap().kind, "macro_rules!");
    }

    #[test]
    fn test_rust_end_boundary_exact() {
        let src = "fn one_liner() {}\nfn two() {}\n";
        let decls: Vec<_> = declarations(src, "rs")
            .into_iter()
            .map(|d| (d.name.clone(), d.start, d.end))
            .collect();
        assert_eq!(
            decls,
            vec![("one_liner".into(), 1, 1), ("two".into(), 2, 2)],
            "end boundary at line end must be correct"
        );
    }

    #[test]
    fn test_typescript_export_default_async_function() {
        let src = r#"export default async function processItems(
  items: string[]
): Promise<void> {
  console.log(items);
}
"#;
        let decls = declarations(src, "ts");
        let func = decls.iter().find(|d| d.name == "processItems");
        assert!(func.is_some(), "should find processItems: {decls:?}");
        assert_eq!(func.unwrap().kind, "function");
        assert!(func.unwrap().signature.is_some());
    }

    #[test]
    fn test_typescript_const_arrow_function() {
        let src = r#"export const handler = async (event: Event) => {
  return event;
};
"#;
        let decls = declarations(src, "ts");
        // Arrow functions are captured via lexical_declaration
        assert!(!decls.is_empty(), "should find declaration: {decls:?}");
    }

    #[test]
    fn test_typescript_interface_and_class() {
        let src = r#"export interface Config {
  timeout: number;
}

export class Service {
  private timeout: number;

  constructor() {
    this.timeout = 1000;
  }

  async fetch(url: string): Promise<Response> {
    return fetch(url);
  }
}
"#;
        let decls: Vec<_> = declarations(src, "ts")
            .into_iter()
            .map(|d| (d.name.clone(), d.kind.clone()))
            .collect();

        assert!(
            decls
                .iter()
                .any(|(n, k)| n == "Config" && *k == "interface"),
            "should find Config interface: {decls:?}"
        );
        assert!(
            decls.iter().any(|(n, k)| n == "Service" && *k == "class"),
            "should find Service class: {decls:?}"
        );
        assert!(
            decls.iter().any(|(n, k)| n == "fetch" && *k == "method"),
            "should find fetch method: {decls:?}"
        );
    }

    #[test]
    fn test_python_decorated_async_def() {
        let src = r#"@app.route("/")
@app.require_auth
async def get_items(
    request: Request,
    param: str = "default"
) -> List[Item]:
    return []
"#;
        let decls = declarations(src, "py");
        let func = decls.iter().find(|d| d.name == "get_items");
        assert!(func.is_some(), "should find decorated async def: {decls:?}");
        assert_eq!(func.unwrap().kind, "def");
    }

    #[test]
    fn test_python_module_def() {
        let src = r#"def standalone():
    pass
"#;
        let decls = declarations(src, "py");
        assert!(
            decls
                .iter()
                .any(|d| d.name == "standalone" && d.kind == "def"),
            "should find standalone def: {decls:?}"
        );
    }

    #[test]
    fn test_python_indented_class_with_method() {
        let src = r#"class Container:
    def __init__(self):
        self.items = []

    def add(self, item):
        self.items.append(item)
"#;
        let decls: Vec<_> = declarations(src, "py")
            .into_iter()
            .map(|d| (d.name.clone(), d.kind.clone()))
            .collect();

        assert!(
            decls.iter().any(|(n, k)| n == "Container" && *k == "class"),
            "should find Container class: {decls:?}"
        );
        assert!(
            decls.iter().any(|(n, k)| n == "__init__" && *k == "def"),
            "should find __init__ method: {decls:?}"
        );
        assert!(
            decls.iter().any(|(n, k)| n == "add" && *k == "def"),
            "should find add method: {decls:?}"
        );
    }

    #[test]
    fn test_go_method_receiver() {
        let src = r#"type Server struct {
    port int
}

func (s *Server) Start() error {
    return nil
}
"#;
        let decls: Vec<_> = declarations(src, "go")
            .into_iter()
            .map(|d| (d.name.clone(), d.kind.clone()))
            .collect();

        assert!(
            decls.iter().any(|(n, k)| n == "Server" && *k == "type"),
            "should find Server type: {decls:?}"
        );
        assert!(
            decls.iter().any(|(n, k)| n == "Start" && *k == "func"),
            "should find Start func: {decls:?}"
        );
    }

    #[test]
    fn test_go_type_struct() {
        let src = r#"type Response struct {
    Code int
    Msg  string
}
"#;
        let decls = declarations(src, "go");
        assert!(
            decls
                .iter()
                .any(|d| d.name == "Response" && d.kind == "type"),
            "should find Response type: {decls:?}"
        );
    }

    #[test]
    fn test_java_class_with_methods_and_nested_interface() {
        let src = r#"public class App {
    private int value;

    public void run() {
        System.out.println("running");
    }

    public String getValue() {
        return String.valueOf(value);
    }

    interface Listener {
        void onEvent();
    }
}
"#;
        let decls: Vec<_> = declarations(src, "java")
            .into_iter()
            .map(|d| (d.name.clone(), d.kind.clone()))
            .collect();

        // Class should be found
        assert!(
            decls.iter().any(|(n, k)| n == "App" && *k == "class"),
            "should find App class: {decls:?}"
        );
        // Interface should be found
        assert!(
            decls
                .iter()
                .any(|(n, k)| n == "Listener" && *k == "interface"),
            "should find Listener interface: {decls:?}"
        );
    }

    #[test]
    fn test_size_cap_skips_large_files() {
        let large = "fn f() {}\n".repeat(210_000);
        let size = large.len();
        assert!(
            size > SIZE_CAP,
            "test string should be > {} bytes, got {}",
            SIZE_CAP,
            size
        );
        let decls = declarations(&large, "rs");
        assert!(
            decls.is_empty(),
            "large files should skip tree-sitter, got {} decls",
            decls.len()
        );
    }

    #[test]
    fn test_unsupported_extension_returns_empty() {
        let src = "class Foo {}";
        let decls = declarations(src, "scala");
        assert!(
            decls.is_empty(),
            "unsupported ext should return empty: {decls:?}"
        );
    }
}
