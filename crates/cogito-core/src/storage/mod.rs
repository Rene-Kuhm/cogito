//! Storage layer
//!
//! Provides pluggable backends. Currently:
//! - SQLite (primary, recommended)
//! - JSON files (legacy, for migration only)

pub mod sqlite;

pub use sqlite::{db_path, open, MigrateReport, Stats};
