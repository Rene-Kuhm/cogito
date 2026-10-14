//! Storage layer
//!
//! Provides SQLite persistence.

pub mod sqlite;

pub use sqlite::{Stats, db_path, open};
