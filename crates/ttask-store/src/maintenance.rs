//! Backups, encryption changes, integrity checks and cleanup.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use secrecy::{ExposeSecret, SecretString};

use crate::migrations::schema_version;
use crate::open::{DbMode, FileState, Store, detect, sibling};
use crate::{Result, StoreError};

/// Copies the whole database into a new file at `dest`.
///
/// `key`: `None` keeps the source's encryption (same key, or plain);
/// `Some("")` writes a plain file; `Some(password)` encrypts with it.
pub(crate) fn export_to(conn: &Connection, dest: &Path, key: Option<&str>) -> Result<()> {
    if detect(dest)? != FileState::Missing {
        return Err(StoreError::AlreadyExists(dest.to_owned()));
    }
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let dest_str = dest
        .to_str()
        .ok_or_else(|| StoreError::Invalid("backup path is not valid UTF-8".into()))?;
    match key {
        // Without a KEY clause SQLCipher reuses the main database's key.
        None => conn.execute("ATTACH DATABASE ?1 AS ttask_export", [dest_str])?,
        Some(key) => conn.execute(
            "ATTACH DATABASE ?1 AS ttask_export KEY ?2",
            params![dest_str, key],
        )?,
    };
    let result = (|| -> Result<()> {
        conn.query_row("SELECT sqlcipher_export('ttask_export')", [], |_| Ok(()))?;
        // sqlcipher_export copies schema and data, not user_version.
        let version = schema_version(conn)?;
        conn.pragma_update(Some("ttask_export"), "user_version", version)?;
        Ok(())
    })();
    conn.execute("DETACH DATABASE ttask_export", [])?;
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}

/// Outcome of `ttask db check`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckReport {
    pub problems: Vec<String>,
}

impl CheckReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Counts from `ttask gc`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GcReport {
    pub purged_tasks: usize,
    pub purged_projects: usize,
    pub compacted_events: usize,
}

impl Store {
    /// Writes a consistent copy (same encryption) while the database is open.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        export_to(&self.conn, dest, None)
    }

    /// Rewrites the file with new encryption: `Some(password)` encrypts or
    /// changes the password, `None` decrypts to plain. The old file is kept
    /// as `<name>.bak` until [`remove_backup_copy`] runs. Consumes the store;
    /// reopen it afterwards.
    pub fn convert(self, new_password: Option<&SecretString>) -> Result<PathBuf> {
        let path = self.path().to_owned();
        let tmp = sibling(&path, "tmp");
        let bak = sibling(&path, "bak");
        if detect(&tmp)? != FileState::Missing {
            fs::remove_file(&tmp)?;
        }
        let key = new_password.map(|p| p.expose_secret()).unwrap_or("");
        if new_password.is_some_and(|p| p.expose_secret().is_empty()) {
            return Err(StoreError::Invalid("the new password is empty".into()));
        }
        export_to(&self.conn, &tmp, Some(key))?;
        // Closing the last connection checkpoints and removes the WAL.
        drop(self);
        fsync(&tmp)?;
        if detect(&bak)? != FileState::Missing {
            fs::remove_file(&bak)?;
        }
        fs::rename(&path, &bak)?;
        if let Err(err) = fs::rename(&tmp, &path) {
            // Put the original back so the user is never left without a database.
            let _ = fs::rename(&bak, &path);
            return Err(err.into());
        }
        Ok(bak)
    }

    pub fn check(&self) -> Result<CheckReport> {
        let mut problems: Vec<String> = self
            .conn
            .prepare("PRAGMA integrity_check")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|line| line != "ok")
            .collect();
        if self.mode() == DbMode::Encrypted {
            problems.extend(
                self.conn
                    .prepare("PRAGMA cipher_integrity_check")?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        let fk: Vec<String> = self
            .conn
            .prepare("PRAGMA foreign_key_check")?
            .query_map([], |r| {
                Ok(format!(
                    "foreign key violation in {} row {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        problems.extend(fk);
        Ok(CheckReport { problems })
    }

    /// Purges rows soft-deleted before `deleted_before_ms` and history older
    /// than `events_before_ms`, then reclaims space.
    pub fn gc(&mut self, deleted_before_ms: i64, events_before_ms: i64) -> Result<GcReport> {
        let report = self.write(|db| {
            let conn = db.conn();
            let purged_tasks = conn.execute(
                "DELETE FROM tasks WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
                [deleted_before_ms],
            )?;
            let purged_projects = conn.execute(
                "DELETE FROM projects WHERE deleted_at IS NOT NULL AND deleted_at < ?1
                   AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.project_id = projects.id)
                   AND NOT EXISTS (SELECT 1 FROM projects c WHERE c.parent_id = projects.id)",
                [deleted_before_ms],
            )?;
            let mut compacted_events = db.compact_events(events_before_ms)?;
            compacted_events += conn.execute(
                "DELETE FROM events WHERE undone = 0 AND task_id IS NOT NULL
                   AND task_id NOT IN (SELECT id FROM tasks)",
                [],
            )?;
            conn.execute(
                "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM task_tags) AND color IS NULL",
                [],
            )?;
            Ok(GcReport {
                purged_tasks,
                purged_projects,
                compacted_events,
            })
        })?;
        self.conn.execute_batch("VACUUM;")?;
        Ok(report)
    }

    /// Changes whenever another connection commits; the TUI polls it.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?)
    }
}

/// Deletes the `<name>.bak` left by [`Store::convert`], if any.
pub fn remove_backup_copy(path: &Path) -> Result<bool> {
    let bak = sibling(path, "bak");
    if detect(&bak)? == FileState::Missing {
        return Ok(false);
    }
    fs::remove_file(bak)?;
    Ok(true)
}

fn fsync(path: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}
