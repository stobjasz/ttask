//! Stored templates; the services layer owns the JSON body's shape.

use rusqlite::{OptionalExtension, params};

use crate::db::Db;
use crate::{Result, StoreError};

/// A template row: its name and JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRow {
    pub name: String,
    pub body: String,
}

impl Db<'_> {
    pub fn templates(&self) -> Result<Vec<TemplateRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, body FROM templates ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| {
            Ok(TemplateRow {
                name: r.get(0)?,
                body: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn template(&self, name: &str) -> Result<TemplateRow> {
        self.conn
            .query_row(
                "SELECT name, body FROM templates WHERE name = ?1",
                [name.trim()],
                |r| {
                    Ok(TemplateRow {
                        name: r.get(0)?,
                        body: r.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("template `{}`", name.trim())))
    }

    /// Creates or replaces the template called `name`.
    pub fn save_template(&self, name: &str, body: &str, now_ms: i64) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("a template needs a name".into()));
        }
        self.conn.execute(
            "INSERT INTO templates (name, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(name) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
            params![name, body, now_ms],
        )?;
        Ok(())
    }

    pub fn delete_template(&self, name: &str) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM templates WHERE name = ?1", [name.trim()])?
            == 0
        {
            return Err(StoreError::NotFound(format!("template `{}`", name.trim())));
        }
        Ok(())
    }
}
