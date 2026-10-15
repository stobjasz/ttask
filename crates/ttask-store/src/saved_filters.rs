use rusqlite::params;
use ttask_core::SavedFilter;

use crate::db::Db;
use crate::{Result, StoreError};

impl Db<'_> {
    pub fn saved_filters(&self) -> Result<Vec<SavedFilter>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, query FROM saved_filters ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SavedFilter {
                id: r.get(0)?,
                name: r.get(1)?,
                query: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Creates or replaces the filter called `name`.
    pub fn save_filter(&self, name: &str, query: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("a saved filter needs a name".into()));
        }
        self.conn.execute(
            "INSERT INTO saved_filters (name, query, sort_order)
             VALUES (?1, ?2, (SELECT coalesce(max(sort_order), 0) + 1 FROM saved_filters))
             ON CONFLICT(name) DO UPDATE SET query = excluded.query",
            params![name, query],
        )?;
        Ok(())
    }

    pub fn delete_filter(&self, name: &str) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM saved_filters WHERE name = ?1", [name])?
            == 0
        {
            return Err(StoreError::NotFound(format!("saved filter `{name}`")));
        }
        Ok(())
    }
}
