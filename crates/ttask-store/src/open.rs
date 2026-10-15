use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use secrecy::{ExposeSecret, SecretString};

use crate::migrations::{migrate, schema_version};
use crate::{Result, StoreError};

/// Every plain SQLite file starts with these 16 bytes; SQLCipher files start
/// with a random salt instead.
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

const BUSY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbMode {
    Plain,
    Encrypted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Missing,
    Exists(DbMode),
}

/// Classifies a path by its first 16 bytes without opening it as a database.
pub fn detect(path: &Path) -> io::Result<FileState> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(FileState::Missing),
        Err(err) => return Err(err),
    };
    let mut header = [0u8; 16];
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    // An empty file is what SQLite itself creates before the first write.
    let mode = if filled == 0 || &header == SQLITE_HEADER {
        DbMode::Plain
    } else {
        DbMode::Encrypted
    };
    Ok(FileState::Exists(mode))
}

/// Supplies the password for an encrypted database, possibly over several attempts.
pub trait PasswordSource {
    /// Returns the password for `attempt` (1-based).
    fn password(&mut self, attempt: u32) -> Result<SecretString>;

    /// How many attempts to make before reporting [`StoreError::WrongPassword`].
    fn max_attempts(&self) -> u32 {
        1
    }
}

/// A fixed password gets exactly one attempt.
impl PasswordSource for SecretString {
    fn password(&mut self, _attempt: u32) -> Result<SecretString> {
        Ok(self.clone())
    }
}

/// The open connection to one ttask database file.
pub struct Store {
    pub(crate) conn: Connection,
    mode: DbMode,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Creates a new database at `path`, encrypted when `password` is given.
    /// Refuses to touch an existing file.
    pub fn create(path: &Path, password: Option<&SecretString>) -> Result<Store> {
        if detect(path)? != FileState::Missing {
            return Err(StoreError::AlreadyExists(path.to_owned()));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
        let mut conn = connect(path, flags, password)?;
        migrate(&mut conn)?;
        let mode = if password.is_some() {
            DbMode::Encrypted
        } else {
            DbMode::Plain
        };
        Ok(Store { conn, mode })
    }

    /// Opens an existing database, asking `passwords` only if the file is encrypted,
    /// then applies any pending migrations.
    pub fn open(path: &Path, passwords: &mut dyn PasswordSource) -> Result<Store> {
        let mode = match detect(path)? {
            FileState::Missing => return Err(StoreError::Missing(path.to_owned())),
            FileState::Exists(mode) => mode,
        };
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE;
        let mut conn = match mode {
            DbMode::Plain => connect(path, flags, None)?,
            DbMode::Encrypted => unlock(path, flags, passwords)?,
        };
        migrate(&mut conn)?;
        Ok(Store { conn, mode })
    }

    pub fn mode(&self) -> DbMode {
        self.mode
    }

    pub fn schema_version(&self) -> Result<u32> {
        schema_version(&self.conn)
    }
}

fn unlock(path: &Path, flags: OpenFlags, passwords: &mut dyn PasswordSource) -> Result<Connection> {
    let attempts = passwords.max_attempts().max(1);
    for attempt in 1..=attempts {
        let password = passwords.password(attempt)?;
        match connect(path, flags, Some(&password)) {
            Err(StoreError::NotADatabase) => continue,
            other => return other,
        }
    }
    Err(StoreError::WrongPassword)
}

/// Opens a connection, applies the key first when given, and proves the key
/// (or plain file) is readable before applying the session pragmas.
fn connect(path: &Path, flags: OpenFlags, key: Option<&SecretString>) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, flags)?;
    // SQLCipher logs to stderr by default (e.g. "hmac check failed" on a wrong
    // password), which would leak into CLI output. Errors surface as results.
    conn.execute_batch("PRAGMA cipher_log_level = NONE;")?;
    if let Some(key) = key {
        conn.pragma_update(None, "key", key.expose_secret())?;
    }
    // With a wrong key this is the first read, and fails with SQLITE_NOTADB.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(conn)
}
