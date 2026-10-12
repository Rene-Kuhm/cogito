//! Storage layer
//!
//! Provides pluggable backends. Currently:
//! - SQLite (primary, recommended)
//! - JSON files (legacy, for migration only)

pub mod sqlite;

pub use sqlite::{MigrateReport, Stats, db_path, open};
