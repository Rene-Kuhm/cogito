//! Basic usage example for Cogito
//!
//! Run with: cargo run --example basic

use cogito_core::{Cogito, CompressionOptions};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create Cogito instance
    let cogito = Cogito::new();

    // Initialize a project
    let project_path = PathBuf::from("./");
    let project = cogito.init_project(project_path)?;

    println!("Project: {} ({})", project.name, project.language);

    // Generate fingerprint
    println!("\nGenerating fingerprint...");
    let fingerprint = cogito.generate_fingerprint(&project).await?;
    println!("  Entry points: {}", fingerprint.entry_points.len());
    println!("  Data flows: {}", fingerprint.data_flows.len());
    println!("  Language: {}", fingerprint.language);

    // Compress context
    println!("\nCompressing context...");
    let large_code = r#"
        // This is a comment that adds nothing to understanding
        fn main() {
            let x = 5;
            let y = 10;
            let sum = x + y;
            println!("{}", sum);
        }
        
        fn helper_function() {
            // Another meaningless comment
            let a = 1;
            let b = 2;
        }
    "#;

    let options = CompressionOptions {
        max_tokens: 100,
        preserve_imports: true,
        preserve_types: true,
        hot_zones_priority: true,
    };

    let result = cogito.compress_context(large_code, options).await?;

    println!("  Original tokens: {}", result.original_tokens);
    println!("  Compressed tokens: {}", result.compressed_tokens);
    println!("  Savings: {:.1}%", result.savings_percent);
    println!("\nCompressed content:\n{}", result.content);

    // Add a semantic tag
    println!("\nAdding semantic tag...");
    use chrono::Utc;
    use cogito_core::types::SemanticTag;
    use uuid::Uuid;

    let tag = SemanticTag {
        id: Uuid::new_v4(),
        project_id: project.id,
        tag: "example".to_string(),
        domain: "demo".to_string(),
        paths: vec![PathBuf::from("examples/")],
        confidence: 0.9,
        created_at: Utc::now(),
    };

    let added = cogito.add_tag(tag)?;
    println!("  Added tag: {}", added.tag);

    // Search tags
    println!("\nSearching tags for 'example'...");
    let tags = cogito.search_tags(project.id, "example")?;
    for tag in tags {
        println!("  - {} ({})", tag.tag, tag.domain);
    }

    // Add a decision
    println!("\nAdding architectural decision...");
    use cogito_core::decision;

    let decision = decision::builder(project.id)
        .title("Example decision")
        .why("To demonstrate the API")
        .alternative("Not doing it")
        .tag("example")
        .build();

    let added = cogito.add_decision(decision)?;
    println!("  Added: {}", added.title);

    // Search decisions
    println!("\nSearching decisions for 'example'...");
    let decisions = cogito.search_decisions(project.id, "example")?;
    for decision in decisions {
        println!("  - {}: {}", decision.title, decision.why);
    }

    println!("\n✓ All examples completed successfully!");

    Ok(())
}
