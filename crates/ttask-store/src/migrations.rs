use rusqlite::Connection;

use crate::{Result, StoreError};

/// Forward-only migrations; entry `i` moves the schema from version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_checklist_undo.sql"),
    include_str!("../migrations/0003_wait_someday.sql"),
];

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn schema_version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(crate) fn check_supported(version: u32) -> Result<()> {
    if version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Brings the schema up to [`SCHEMA_VERSION`], one transaction per migration.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)?;
    check_supported(current)?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
