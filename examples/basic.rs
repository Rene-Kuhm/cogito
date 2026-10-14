//! Basic usage example for Cogito
//!
//! Run with: cargo run --example basic

use cogito_harness_core::{Cogito, index};
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

    // Search using the index
    println!("\nSearching for 'map'...");
    let _ = cogito.sync_index(&project).await?;
    if let Some(conn) = cogito.connection() {
        let hits = index::search(&conn, project.id, "map", 12)?;
        let modules = fingerprint.modules.clone();
        let files_first = false; // simplified for this example
        let search_text =
            cogito_harness_core::map::render_search(&hits, &modules, "map", files_first);
        println!("{}", search_text);
    }

    println!("\n✓ All examples completed successfully!");

    Ok(())
}
