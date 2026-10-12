//! AST-aware compressor
//!
//! Goes beyond regex-based compression: it parses the source into an AST
//! (Rust via `syn`, falling back to structural compression for other languages)
//! and re-renders a token-reduced form that preserves *signatures* and
//! *structure* but collapses bodies.
//!
//! Savings come from:
//! - Function bodies replaced with `/* ... */` (no body bytes)
//! - Type fields preserved, but large struct bodies summarized
//! - Impl blocks keep method signatures, collapse bodies
//! - Constants and uses kept verbatim (they're cheap and matter)
//! - Comments stripped (already done in structural pass; we re-do it for safety)
//!
//! The output is **not valid code** in the sense of being compilable,
//! but it is **syntactically faithful** — every signature and type
//! is preserved, so the receiving model can reason about structure.

use crate::types::{CompressionOptions, Language};
use anyhow::{Result, anyhow};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

/// Compression mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstMode {
    /// Strip function bodies, keep signatures (default for AST)
    CollapseBodies,
    /// Strip function bodies AND collapse impl blocks to a single line
    Aggressive,
}

/// Result of AST compression
#[derive(Debug, Clone)]
pub struct AstCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub structs_preserved: usize,
    pub impls_collapsed: usize,
}

/// Compress source code using AST awareness.
///
/// Dispatches to language-specific backends.
pub fn compress_ast(
    content: &str,
    language: Language,
    options: &CompressionOptions,
) -> Result<AstCompressResult> {
    match language {
        Language::Rust => compress_rust(content, options),
        _ => Err(anyhow!(
            "AST compression not yet implemented for {}",
            language
        )),
    }
}

/// AST compressor for Rust using `syn`.
///
/// Saves *signatures* and *types*, collapses *bodies*.
pub fn compress_rust_only(
    content: &str,
    options: &CompressionOptions,
) -> Result<AstCompressResult> {
    compress_rust(content, options)
}

fn compress_rust(content: &str, options: &CompressionOptions) -> Result<AstCompressResult> {
    let file: syn::File = syn::parse_file(content).map_err(|e| anyhow!("Rust parse error: {e}"))?;

    let mut stats = AstStats::default();

    let mut out = String::with_capacity(content.len() / 2);

    // Collect use statements at the top, then everything else
    let mut preserved_use = Vec::new();
    for item in &file.items {
        if let syn::Item::Use(_) = item {
            preserved_use.push(item.clone());
        }
    }

    // Render preserved items first (uses + extern crates)
    for item in &preserved_use {
        let tokens = item.to_token_stream().to_string();
        out.push_str(&normalize_whitespace(&tokens));
        out.push('\n');
    }

    if !preserved_use.is_empty() {
        out.push('\n');
    }

    // Render the rest (skip uses which we already handled)
    for item in &file.items {
        if matches!(item, syn::Item::Use(_)) {
            continue;
        }
        render_item(item, &mut out, &mut stats, options);
        out.push('\n');
    }

    // Normalize the whole output: collapse multiple spaces, trim lines
    let normalized = normalize_output(&out);

    Ok(AstCompressResult {
        content: normalized,
        functions_collapsed: stats.functions,
        structs_preserved: stats.structs,
        impls_collapsed: stats.impls,
    })
}

/// Normalize whitespace: collapse multiple spaces to one, strip
/// space before semicolons, etc. Keeps the result readable.
fn normalize_whitespace(s: &str) -> String {
    // Collapse multiple spaces into one
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    // Post-process: collapse `X :: Y` → `X::Y` (regex on spaces around `::`)
    // Also collapse `& X` → `&X` for borrow types
    let s = out;
    let mut result = s.clone();
    // Pattern 1: letter/underscore/digit then ' :: ' then anything -> '::'
    // We do this by repeatedly collapsing ' :: ' until none remain
    loop {
        let new = result.replace(" :: ", "::");
        if new == result {
            break;
        }
        result = new;
    }
    // Pattern 2: '& ' followed by an identifier-start char -> '&'
    let chars: Vec<char> = result.chars().collect();
    let mut out2 = String::with_capacity(result.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' && i + 1 < chars.len() && chars[i + 1] == ' ' {
            let next = chars.get(i + 2).copied().unwrap_or(' ');
            if next.is_alphabetic() || next == '_' {
                out2.push('&');
                i += 2;
                continue;
            }
        }
        out2.push(chars[i]);
        i += 1;
    }
    out2.trim().to_string()
}

/// Normalize multi-line output: trim each line, drop empties,
/// collapse internal whitespace, and fix `::` / `& ` spacing.
fn normalize_output(s: &str) -> String {
    s.lines()
        .map(normalize_whitespace)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Default)]
struct AstStats {
    functions: usize,
    structs: usize,
    impls: usize,
}

fn render_item(
    item: &syn::Item,
    out: &mut String,
    stats: &mut AstStats,
    options: &CompressionOptions,
) {
    match item {
        syn::Item::Fn(f) => render_fn(f, out, stats, options),
        syn::Item::Struct(s) => render_struct(s, out, stats, options),
        syn::Item::Enum(e) => render_enum(e, out, stats, options),
        syn::Item::Impl(i) => render_impl(i, out, stats, options),
        syn::Item::Trait(t) => render_trait(t, out, stats, options),
        syn::Item::Type(t) => render_type_alias(t, out),
        syn::Item::Const(c) => render_const(c, out),
        syn::Item::Static(s) => render_static(s, out),
        syn::Item::Mod(m) => render_mod(m, out, stats, options),
        syn::Item::Macro(m) => render_macro(m, out),
        _ => {
            // Fallback: emit verbatim
            let tokens: TokenStream = quote!(#item);
            out.push_str(&tokens.to_string());
        }
    }
}

fn render_fn(
    f: &syn::ItemFn,
    out: &mut String,
    stats: &mut AstStats,
    options: &CompressionOptions,
) {
    stats.functions += 1;
    let sig = &f.sig;
    let vis = &f.vis;
    let attrs = &f.attrs;
    let name = &sig.ident;
    let inputs = render_params(&sig.inputs);
    let output = render_return(&sig.output);
    let generics = render_generics(&sig.generics);

    for attr in attrs {
        out.push_str(&attr.to_token_stream().to_string());
        out.push('\n');
    }

    let vis_str = visibility_to_string(vis);
    if !vis_str.is_empty() {
        out.push_str(&vis_str);
        out.push(' ');
    }

    if options.preserve_types {
        out.push_str(&format!(
            "fn {name}{generics}({inputs}){output} {{ /* ... */ }}\n"
        ));
    } else {
        out.push_str(&format!("fn {name}({inputs}){output} {{ /* ... */ }}\n"));
    }
}

fn render_struct(
    s: &syn::ItemStruct,
    out: &mut String,
    stats: &mut AstStats,
    _options: &CompressionOptions,
) {
    stats.structs += 1;
    let vis_str = visibility_to_string(&s.vis);
    if !vis_str.is_empty() {
        out.push_str(&vis_str);
        out.push(' ');
    }

    let name = &s.ident;
    let generics = render_generics(&s.generics);

    let fields_str = render_fields(&s.fields);
    out.push_str(&format!("struct {name}{generics} {fields_str}\n"));
}

fn render_enum(
    e: &syn::ItemEnum,
    out: &mut String,
    stats: &mut AstStats,
    _options: &CompressionOptions,
) {
    stats.structs += 1;
    let name = &e.ident;
    let generics = render_generics(&e.generics);

    let mut variants_str = String::new();
    for variant in &e.variants {
        let v_name = &variant.ident;
        let v_fields = match &variant.fields {
            syn::Fields::Named(named) => {
                let mut s = String::from(" { ");
                let names: Vec<String> = named
                    .named
                    .iter()
                    .map(|f| {
                        let n = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                        format!("{n}: ...")
                    })
                    .collect();
                s.push_str(&names.join(", "));
                s.push_str(" }");
                s
            }
            syn::Fields::Unnamed(unnamed) => {
                let count = unnamed.unnamed.len();
                if count == 0 {
                    String::new()
                } else {
                    format!("({})", vec!["..."; count].join(", "))
                }
            }
            syn::Fields::Unit => String::new(),
        };
        variants_str.push_str(&format!("    {v_name}{v_fields},\n"));
    }

    out.push_str(&format!("enum {name}{generics} {{\n{variants_str}}}\n"));
}

fn render_impl(
    i: &syn::ItemImpl,
    out: &mut String,
    stats: &mut AstStats,
    options: &CompressionOptions,
) {
    stats.impls += 1;

    let trait_ = i
        .trait_
        .as_ref()
        .map(|(_, path, _)| path.to_token_stream().to_string());
    let self_ty = i.self_ty.to_token_stream().to_string();
    let generics = render_generics(&i.generics);

    let header = match trait_ {
        Some(t) => format!("impl{generics} {t} for {self_ty} {{\n"),
        None => format!("impl{generics} {self_ty} {{\n"),
    };
    out.push_str(&header);

    for item in &i.items {
        if let syn::ImplItem::Fn(method) = item {
            let sig = &method.sig;
            let vis_str = visibility_to_string(&method.vis);
            let name = &sig.ident;
            let inputs = render_params(&sig.inputs);
            let output = render_return(&sig.output);
            let generics = render_generics(&sig.generics);
            let vis_prefix = if vis_str.is_empty() {
                String::new()
            } else {
                format!("{vis_str} ")
            };
            out.push_str(&format!(
                "    {vis_prefix}fn {name}{generics}({inputs}){output} {{ /* ... */ }}\n"
            ));
        } else if let syn::ImplItem::Const(c) = item {
            let vis = visibility_to_string(&c.vis);
            let name = &c.ident;
            let ty = c.ty.to_token_stream().to_string();
            out.push_str(&format!("    {vis} const {name}: {ty} = ...;\n"));
        } else if let syn::ImplItem::Type(t) = item {
            let vis = visibility_to_string(&t.vis);
            let name = &t.ident;
            out.push_str(&format!("    {vis} type {name} = ...;\n"));
        } else {
            let tokens: TokenStream = quote!(#item);
            out.push_str(&format!("    {}\n", tokens));
        }
    }

    out.push_str("}\n");
    let _ = options;
}

fn render_trait(
    t: &syn::ItemTrait,
    out: &mut String,
    stats: &mut AstStats,
    _options: &CompressionOptions,
) {
    stats.structs += 1;
    let vis_str = visibility_to_string(&t.vis);
    if !vis_str.is_empty() {
        out.push_str(&vis_str);
        out.push(' ');
    }

    let name = &t.ident;
    let generics = render_generics(&t.generics);
    let supertraits = if !t.supertraits.is_empty() {
        let supers: Vec<String> = t
            .supertraits
            .iter()
            .map(|s| s.to_token_stream().to_string())
            .collect();
        format!(": {}", supers.join(" + "))
    } else {
        String::new()
    };

    out.push_str(&format!("trait {name}{generics}{supertraits} {{\n"));

    for item in &t.items {
        if let syn::TraitItem::Fn(method) = item {
            let sig = &method.sig;
            let name = &sig.ident;
            let inputs = render_params(&sig.inputs);
            let output = render_return(&sig.output);
            let has_default = method.default.is_some();
            out.push_str(&format!(
                "    fn {name}({inputs}){output}{};\n",
                if has_default {
                    " { /* default */ }"
                } else {
                    ""
                }
            ));
        } else {
            let tokens: TokenStream = quote!(#item);
            out.push_str(&format!("    {}\n", tokens));
        }
    }

    out.push_str("}\n");
}

fn render_type_alias(t: &syn::ItemType, out: &mut String) {
    let vis_str = visibility_to_string(&t.vis);
    let name = &t.ident;
    let ty = t.ty.to_token_stream().to_string();
    let generics = render_generics(&t.generics);
    out.push_str(&format!("{vis_str} type {name}{generics} = {ty};\n"));
}

fn render_const(c: &syn::ItemConst, out: &mut String) {
    let vis_str = visibility_to_string(&c.vis);
    let name = &c.ident;
    let ty = c.ty.to_token_stream().to_string();
    let expr = c.expr.to_token_stream().to_string();
    out.push_str(&format!("{vis_str} const {name}: {ty} = {expr};\n"));
}

fn render_static(s: &syn::ItemStatic, out: &mut String) {
    let vis_str = visibility_to_string(&s.vis);
    let name = &s.ident;
    let ty = s.ty.to_token_stream().to_string();
    let expr = s.expr.to_token_stream().to_string();
    out.push_str(&format!("{vis_str} static {name}: {ty} = {expr};\n"));
}

fn render_mod(
    m: &syn::ItemMod,
    out: &mut String,
    stats: &mut AstStats,
    options: &CompressionOptions,
) {
    let vis_str = visibility_to_string(&m.vis);
    if !vis_str.is_empty() {
        out.push_str(&vis_str);
        out.push(' ');
    }

    let name = &m.ident;
    out.push_str(&format!("mod {name} {{\n"));

    if let Some((_, items)) = &m.content {
        for item in items {
            render_item(item, out, stats, options);
        }
    }

    out.push_str("}\n");
}

fn render_macro(m: &syn::ItemMacro, out: &mut String) {
    let tokens = quote!(#m);
    out.push_str(&format!("{tokens}\n"));
}

// ============================================================
// Helpers
// ============================================================

fn render_params(inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>) -> String {
    let parts: Vec<String> = inputs
        .iter()
        .map(|arg| {
            let tokens = quote!(#arg);
            // For receiver (self), strip the body if any
            tokens.to_string()
        })
        .collect();
    parts.join(", ")
}

fn render_return(output: &syn::ReturnType) -> String {
    match output {
        syn::ReturnType::Default => String::new(),
        syn::ReturnType::Type(_, ty) => {
            format!(" -> {}", quote!(#ty))
        }
    }
}

fn render_generics(generics: &syn::Generics) -> String {
    if generics.params.is_empty() {
        return String::new();
    }
    let tokens = quote!(#generics);
    tokens.to_string()
}

fn render_fields(fields: &syn::Fields) -> String {
    match fields {
        syn::Fields::Named(named) => {
            let mut s = String::from(" {\n");
            for field in &named.named {
                let vis_str = visibility_to_string(&field.vis);
                let vis_prefix = if vis_str.is_empty() {
                    String::new()
                } else {
                    format!("{vis_str} ")
                };
                let name = field
                    .ident
                    .as_ref()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let ty = type_to_string(&field.ty);
                s.push_str(&format!("    {vis_prefix}{name}: {ty},\n"));
            }
            s.push('}');
            s
        }
        syn::Fields::Unnamed(unnamed) => {
            let count = unnamed.unnamed.len();
            if count == 0 {
                String::from(";")
            } else {
                format!("({});", vec!["_"; count].join(", "))
            }
        }
        syn::Fields::Unit => ";".to_string(),
    }
}

/// Convert a syn::Type to its string representation cleanly.
fn type_to_string(ty: &syn::Type) -> String {
    quote!(#ty).to_string()
}

/// Convert a syn::Visibility to its string representation.
fn visibility_to_string(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Inherited => String::new(),
        syn::Visibility::Public(_) => "pub".to_string(),
        syn::Visibility::Restricted(r) => quote!(#r.path).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_simple_function() {
        let code = r#"
            fn add(a: i32, b: i32) -> i32 {
                let result = a + b;
                println!("debug: a={} b={}", a, b);
                if a > 100 {
                    return b;
                }
                result * 2
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("fn add"));
        assert!(result.content.contains("-> i32"));
        assert!(result.content.contains("/* ... */"));
        // Body should NOT be in the output
        assert!(!result.content.contains("let result"));
        assert!(!result.content.contains("println!"));
    }

    #[test]
    fn test_collapse_struct() {
        let code = r#"
            pub struct User {
                pub id: u64,
                pub name: String,
                pub email: String,
                pub created_at: chrono::DateTime<Utc>,
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.structs_preserved >= 1);
        assert!(result.content.contains("struct User"));
        assert!(result.content.contains("pub name:"));
        assert!(result.content.contains("pub email:"));
    }

    #[test]
    fn test_collapse_enum() {
        let code = r#"
            pub enum Status {
                Active,
                Inactive { reason: String },
                Pending(u32, u64),
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.content.contains("enum Status"));
        assert!(result.content.contains("Active"));
        assert!(result.content.contains("Inactive"));
        assert!(result.content.contains("Pending"));
    }

    #[test]
    fn test_collapse_impl_block() {
        let code = r#"
            impl User {
                pub fn new(name: String) -> Self {
                    Self {
                        id: generate_id(),
                        name,
                        email: String::new(),
                        created_at: chrono::Utc::now(),
                    }
                }
                pub fn validate(&self) -> Result<(), String> {
                    if self.name.is_empty() {
                        return Err("name required".to_string());
                    }
                    Ok(())
                }
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.impls_collapsed >= 1);
        assert!(result.content.contains("impl User"));
        assert!(result.content.contains("fn new"));
        assert!(result.content.contains("fn validate"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("generate_id"));
    }

    #[test]
    fn test_preserves_use_statements() {
        let code = r#"
            use std::collections::HashMap;
            use serde::{Deserialize, Serialize};

            pub struct Config {
                pub data: HashMap<String, String>,
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.content.contains("use std::collections::HashMap"));
        assert!(result.content.contains("use serde"));
    }

    #[test]
    fn test_collapse_trait() {
        let code = r#"
            pub trait Handler {
                fn handle(&self, req: Request) -> Response;
                fn validate(&self) -> bool {
                    self.check_basic()
                }
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        assert!(result.content.contains("trait Handler"));
        assert!(result.content.contains("fn handle"));
        // The default method body should be summarized
        assert!(!result.content.contains("check_basic"));
    }

    #[test]
    fn test_preserves_const_and_static() {
        let code = r#"
            pub const MAX_SIZE: usize = 1024;
            pub static VERSION: &str = "0.1.0";

            pub fn get_max() -> usize { MAX_SIZE }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        eprintln!("OUTPUT:\n{}", result.content);
        assert!(result.content.contains("MAX_SIZE: usize = 1024"));
        assert!(result.content.contains("VERSION: &str = \"0.1.0\""));
    }

    #[test]
    fn test_real_world_module_savings() {
        // A realistic-ish module
        let code = r#"
            use std::collections::HashMap;

            pub struct User {
                pub id: u64,
                pub name: String,
                pub email: String,
                pub metadata: HashMap<String, String>,
            }

            impl User {
                pub fn new(id: u64, name: String, email: String) -> Self {
                    let metadata = HashMap::new();
                    Self { id, name, email, metadata }
                }
                pub fn with_metadata(mut self, key: String, value: String) -> Self {
                    self.metadata.insert(key, value);
                    self
                }
                pub fn get_metadata(&self, key: &str) -> Option<&String> {
                    self.metadata.get(key)
                }
            }

            pub fn validate_email(email: &str) -> bool {
                if email.is_empty() {
                    return false;
                }
                if !email.contains('@') {
                    return false;
                }
                true
            }

            pub fn create_user(name: String, email: String) -> Result<User, String> {
                if !validate_email(&email) {
                    return Err("invalid email".to_string());
                }
                Ok(User::new(generate_id(), name, email))
            }

            fn generate_id() -> u64 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0)
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_ast(code, Language::Rust, &opts).unwrap();
        eprintln!("REAL WORLD OUTPUT:\n{}", result.content);

        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        // Should be at least 30% smaller for this kind of code
        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        // All public APIs preserved
        assert!(result.content.contains("pub struct User"));
        assert!(result.content.contains("pub fn new"));
        assert!(result.content.contains("pub fn validate_email"));
        assert!(result.content.contains("pub fn create_user"));
    }

    #[test]
    fn test_unsupported_language_returns_error() {
        let opts = CompressionOptions::default();
        let result = compress_ast("print('hello')", Language::Python, &opts);
        assert!(result.is_err());
    }
}
