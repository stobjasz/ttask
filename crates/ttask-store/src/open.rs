use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use secrecy::{ExposeSecret, SecretString};

use crate::db::Db;
use crate::maintenance::export_to;
use crate::migrations::{SCHEMA_VERSION, check_supported, migrate, schema_version};
use crate::{Result, StoreError};

/// Every plain SQLite file starts with these 16 bytes; SQLCipher files start
/// with a random salt instead.
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

const BUSY_TIMEOUT: Duration = Duration::from_secs(3);

/// The SQLite `application_id` stamped into every ttask file ("ttsk").
pub const APPLICATION_ID: i32 = 0x7474_736B;

/// SQLite and SQLCipher files are whole pages, and pages are at least this big.
const MIN_PAGE_SIZE: u64 = 512;

/// Tables every ttask schema has; they identify files from before the
/// application id was stamped.
const TTASK_TABLES: &[&str] = &["tasks", "projects", "events", "meta"];

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

/// Checks that `path` is a ttask database without needing its password, and
/// reports whether it is encrypted. A plain file is opened and its identity
/// checked; an encrypted one can only be checked for size until it is
/// unlocked, which [`Store::open`] then does.
pub fn inspect(path: &Path) -> Result<DbMode> {
    let mode = check_file(path)?;
    if mode == DbMode::Plain {
        let conn = connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE, None)?;
        check_identity(&conn, path)?;
    }
    Ok(mode)
}

/// The checks that need no connection: the path is a non-empty file and,
/// when it does not start with the SQLite header, has an SQLCipher file's size.
fn check_file(path: &Path) -> Result<DbMode> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(StoreError::Missing(path.to_owned()));
        }
        Err(err) => return Err(err.into()),
    };
    if meta.is_dir() {
        return Err(not_ttask(path, "it is a directory"));
    }
    let mode = match detect(path)? {
        FileState::Missing => return Err(StoreError::Missing(path.to_owned())),
        FileState::Exists(mode) => mode,
    };
    // A database still being created can have an empty main file while its
    // first pages sit in the WAL.
    let wal_len = fs::metadata(sibling_raw(path, "-wal")).map_or(0, |m| m.len());
    if meta.len() == 0 && wal_len == 0 {
        return Err(not_ttask(path, "the file is empty"));
    }
    if mode == DbMode::Encrypted && (meta.len() < MIN_PAGE_SIZE || meta.len() % MIN_PAGE_SIZE != 0)
    {
        return Err(not_ttask(
            path,
            "it is neither an SQLite nor an encrypted database",
        ));
    }
    Ok(mode)
}

/// Accepts a file stamped with [`APPLICATION_ID`], or an unstamped one with
/// ttask's tables; refuses other applications' SQLite databases.
fn check_identity(conn: &Connection, path: &Path) -> Result<()> {
    let app_id: i32 = conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if app_id == APPLICATION_ID {
        return Ok(());
    }
    if app_id != 0 {
        return Err(not_ttask(path, "it belongs to another application"));
    }
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if tables.is_empty() {
        return Err(not_ttask(path, "the database is empty"));
    }
    let has_all = TTASK_TABLES.iter().all(|t| tables.iter().any(|n| n == t));
    if !has_all || schema_version(conn)? == 0 {
        return Err(not_ttask(
            path,
            "it is an SQLite database from another application",
        ));
    }
    Ok(())
}

fn not_ttask(path: &Path, reason: &str) -> StoreError {
    StoreError::NotTtask {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
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
    path: PathBuf,
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
        configure(&conn)?;
        migrate(&mut conn)?;
        let mode = if password.is_some() {
            DbMode::Encrypted
        } else {
            DbMode::Plain
        };
        Ok(Store {
            conn,
            mode,
            path: path.to_owned(),
        })
    }

    /// Opens an existing database, asking `passwords` only if the file is encrypted,
    /// then applies any pending migrations. Files that are not ttask databases
    /// are refused before anything is written to them.
    pub fn open(path: &Path, passwords: &mut dyn PasswordSource) -> Result<Store> {
        let mode = check_file(path)?;
        // CREATE is needed even for an existing file: ATTACHed databases
        // (backups, conversions) inherit the flags and must be creatable.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
        let mut conn = match mode {
            DbMode::Plain => connect(path, flags, None)?,
            DbMode::Encrypted => unlock(path, flags, passwords)?,
        };
        check_identity(&conn, path)?;
        configure(&conn)?;
        let version = schema_version(&conn)?;
        check_supported(version)?;
        if version > 0 && version < SCHEMA_VERSION {
            // Keep a copy from before the upgrade so a failed one can be undone by hand.
            let backup = sibling(path, &format!("pre-v{}.bak", SCHEMA_VERSION));
            if detect(&backup)? == FileState::Missing {
                export_to(&conn, &backup, None)?;
            }
        }
        migrate(&mut conn)?;
        Ok(Store {
            conn,
            mode,
            path: path.to_owned(),
        })
    }

    pub fn mode(&self) -> DbMode {
        self.mode
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> Result<u32> {
        schema_version(&self.conn)
    }

    /// Read access outside a transaction.
    pub fn db(&self) -> Db<'_> {
        Db { conn: &self.conn }
    }

    /// Runs `f` in one immediate transaction: all of it commits or none does.
    pub fn write<T>(&mut self, f: impl FnOnce(&Db<'_>) -> Result<T>) -> Result<T> {
        self.write_with(f)
    }

    /// [`Store::write`] for callers with their own error type.
    pub fn write_with<T, E: From<StoreError>>(
        &mut self,
        f: impl FnOnce(&Db<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from)?;
        let out = f(&Db { conn: &tx })?;
        tx.commit().map_err(StoreError::from)?;
        Ok(out)
    }
}

/// `dir/name.db` + `ext` → `dir/name.db.ext`.
pub(crate) fn sibling(path: &Path, ext: &str) -> PathBuf {
    sibling_raw(path, &format!(".{ext}"))
}

/// `dir/name.db` + `suffix` → `dir/name.db<suffix>` (SQLite's `-wal`).
fn sibling_raw(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
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
/// (or plain file) is readable. Writes nothing; see [`configure`].
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
    Ok(conn)
}

/// Session pragmas; switching to WAL rewrites the file header, so this runs
/// only once the file is known to be ttask's.
fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(())
}
