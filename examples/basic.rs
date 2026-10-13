//! Basic usage example for Cogito
//!
//! Run with: cargo run --example basic

use cogito_harness_core::Cogito;
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

    // Build and render the project map
    println!("\nBuilding project map...");
    let map = cogito_harness_core::map::build(&project.path, &project.name, &fingerprint);
    let map_text = cogito_harness_core::map::render(&map);
    println!("\nProject Map:\n{}", map_text);

    // Search for symbols
    println!("\nSearching for 'map'...");
    let results = cogito_harness_core::map::search_paths(&project.path, &fingerprint, "map");
    let search_text = cogito_harness_core::map::render_search(&results, "map");
    println!("{}", search_text);

    println!("\n✓ All examples completed successfully!");

    Ok(())
}
