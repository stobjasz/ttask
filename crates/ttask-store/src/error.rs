use std::io;
use std::path::PathBuf;

use rusqlite::ErrorCode;
use ttask_core::TaskId;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("no database at {}", .0.display())]
    Missing(PathBuf),

    #[error("a file already exists at {}", .0.display())]
    AlreadyExists(PathBuf),

    #[error("wrong password")]
    WrongPassword,

    #[error("password unavailable: {0}")]
    PasswordUnavailable(String),

    #[error("the file is not a ttask database, or it is corrupt")]
    NotADatabase,

    #[error(
        "database schema version {found} is newer than this ttask supports ({supported}); upgrade ttask"
    )]
    NewerSchema { found: u32, supported: u32 },

    #[error("the database is locked by another process")]
    Locked,

    #[error("task {0} not found")]
    TaskNotFound(TaskId),

    #[error("{0} not found")]
    NotFound(String),

    #[error("{0}")]
    Invalid(String),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Sqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        match err.sqlite_error_code() {
            Some(ErrorCode::NotADatabase) => StoreError::NotADatabase,
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StoreError::Locked,
            _ => StoreError::Sqlite(err),
        }
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        StoreError::Invalid(format!("corrupt journal entry: {err}"))
    }
}
