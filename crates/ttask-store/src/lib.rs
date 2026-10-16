//! SQLite/SQLCipher storage for ttask.
//!
//! [`Store`] owns the single connection. Only the opener in this crate knows
//! whether the file is encrypted; everything above it sees the same API.
//! Repository methods live on [`Db`], obtained from [`Store::db`] for reads or
//! [`Store::write`] for one atomic transaction.

mod checklist;
mod db;
mod error;
mod extras;
mod journal;
mod maintenance;
mod migrations;
mod open;
mod projects;
mod query;
mod reports;
mod saved_filters;
mod tags;
mod tasks;
mod templates;
mod transfer;

pub use db::Db;
pub use error::StoreError;
pub use extras::{Reminder, TimeEntry};
pub use journal::{
    EventKind, HistoryEntry, SnapshotChecklistItem, SnapshotReminder, TaskSnapshot, UndoOutcome,
};
pub use maintenance::{CheckReport, GcReport, remove_backup_copy};
pub use migrations::SCHEMA_VERSION;
pub use open::{APPLICATION_ID, DbMode, FileState, PasswordSource, Store, detect, inspect};
pub use query::{QueryCtx, SearchHit, TaskQuery, TaskSort};
pub use reports::{DueRow, TimeRow};
pub use tasks::TaskFields;
pub use templates::TemplateRow;
pub use transfer::{EventRecord, ProjectRecord};

pub type Result<T, E = StoreError> = std::result::Result<T, E>;
