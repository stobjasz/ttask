//! SQLite/SQLCipher storage for ttask.
//!
//! [`Store`] owns the single connection. Only the opener in this crate knows
//! whether the file is encrypted; everything above it sees the same API.

mod error;
mod migrations;
mod open;
mod tasks;

pub use error::StoreError;
pub use migrations::SCHEMA_VERSION;
pub use open::{DbMode, FileState, PasswordSource, Store, detect};
pub use tasks::TaskFilter;

pub type Result<T, E = StoreError> = std::result::Result<T, E>;
