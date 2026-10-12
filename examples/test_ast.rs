use cogito_harness_core::Language;
use cogito_harness_core::compressor::compress_with_language;
use cogito_harness_core::types::CompressionOptions;

fn main() {
    let code = r#"
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub const MAX_SIZE: usize = 1024;
pub static VERSION: &str = "0.1.0";

pub struct User {
    pub id: u64,
    pub name: String,
}

impl User {
    pub fn new(name: String) -> Self {
        Self {
            id: generate_id(),
            name: name,
        }
    }
    pub fn validate(&self) -> bool {
        if self.name.is_empty() {
            return false;
        }
        true
    }
}

pub trait Handler {
    fn handle(&self, req: Request) -> Response;
    fn validate(&self) -> bool {
        self.check_basic()
    }
}
"#;
    let opts = CompressionOptions::default();
    let result = compress_with_language(code, opts, Some(Language::Rust)).unwrap();
    println!("=== AST output ===");
    println!("{}", result.content);
    println!("\n=== Original size: {} bytes", code.len());
    println!("=== Compressed size: {} bytes", result.content.len());
    println!(
        "=== Savings: {:.1}%",
        100.0 * (1.0 - result.content.len() as f64 / code.len() as f64)
    );
}
