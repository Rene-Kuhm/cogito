//! Scan the current repo and display fingerprint
//!
//! Run with: cargo run --example scan_repo

use cogito_core::{Cogito, Language, Project};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let project_path = PathBuf::from(".");

    // Detect language from Cargo.toml
    let project = if project_path.join("Cargo.toml").exists() {
        Project {
            id: uuid::Uuid::new_v4(),
            path: project_path.clone(),
            name: project_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("cogito")
                .to_string(),
            language: Language::Rust,
            fingerprint: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    } else {
        let cogito = Cogito::new();
        cogito.init_project(project_path)?
    };

    println!("╔════════════════════════════════════════╗");
    println!("║   COGITO - Code Fingerprint Report    ║");
    println!("╚════════════════════════════════════════╝");
    println!();
    println!("Project: {}", project.name);
    println!("Language: {}", project.language);
    println!();

    let cogito = Cogito::new();
    let fingerprint = cogito.generate_fingerprint(&project).await?;

    println!("📍 Entry Points ({}):", fingerprint.entry_points.len());
    for ep in fingerprint.entry_points.iter().take(10) {
        println!("   • {}", ep.display());
    }
    println!();

    println!("🔀 Data Flows ({}):", fingerprint.data_flows.len());
    let unique_targets: std::collections::HashSet<_> =
        fingerprint.data_flows.iter().map(|f| &f.to).collect();
    for target in unique_targets.iter().take(10) {
        println!("   → {}", target.display());
    }
    println!();

    println!("🔥 Hot Zones ({}):", fingerprint.hot_zones.len());
    for zone in fingerprint.hot_zones.iter().take(5) {
        let display_path = zone.path.strip_prefix(&project.path).unwrap_or(&zone.path);
        println!(
            "   • {} ({} changes)",
            display_path.display(),
            zone.change_count
        );
    }
    println!();

    println!("❄️  Cold Zones ({}):", fingerprint.cold_zones.len());
    for zone in fingerprint.cold_zones.iter().take(5) {
        let display_path = zone.path.strip_prefix(&project.path).unwrap_or(&zone.path);
        println!(
            "   • {} ({} changes)",
            display_path.display(),
            zone.change_count
        );
    }
    println!();

    println!("🧊 Frozen Zones ({}):", fingerprint.frozen_zones.len());
    for zone in fingerprint.frozen_zones.iter().take(5) {
        let display_path = zone.path.strip_prefix(&project.path).unwrap_or(&zone.path);
        println!("   • {}", display_path.display());
    }
    println!();

    println!("════════════════════════════════════════");
    println!("Generated at: {}", fingerprint.generated_at);

    Ok(())
}
