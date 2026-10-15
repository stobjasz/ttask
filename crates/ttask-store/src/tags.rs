use rusqlite::{OptionalExtension, params};
use ttask_core::TagInfo;
use ttask_core::quickadd::is_tag_name;

use crate::db::Db;
use crate::{Result, StoreError};

impl Db<'_> {
    /// Returns the tag's id, creating it on first use. Names are case-insensitive.
    pub fn ensure_tag(&self, name: &str) -> Result<i64> {
        if !is_tag_name(name) {
            return Err(StoreError::Invalid(format!(
                "invalid tag `{name}`: start with a letter, then letters, digits, _ or -"
            )));
        }
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn
            .execute("INSERT INTO tags (name) VALUES (?1)", [name])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Tags in use or defined, with open-task counts, alphabetically.
    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT g.name, g.color,
                    (SELECT count(*) FROM task_tags tt JOIN tasks t ON t.id = tt.task_id
                      WHERE tt.tag_id = g.id AND t.status = 'open' AND t.deleted_at IS NULL)
               FROM tags g ORDER BY g.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(TagInfo {
                name: r.get(0)?,
                color: r.get(1)?,
                open_tasks: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Renames a tag; renaming onto an existing tag merges the two.
    pub fn rename_tag(&self, old: &str, new: &str) -> Result<()> {
        let old_id = self.tag_id(old)?;
        if !is_tag_name(new) {
            return Err(StoreError::Invalid(format!("invalid tag `{new}`")));
        }
        let target: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 AND id <> ?2",
                params![new, old_id],
                |r| r.get(0),
            )
            .optional()?;
        match target {
            Some(new_id) => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO task_tags (task_id, tag_id)
                     SELECT task_id, ?2 FROM task_tags WHERE tag_id = ?1",
                    params![old_id, new_id],
                )?;
                self.conn
                    .execute("DELETE FROM tags WHERE id = ?1", [old_id])?;
            }
            None => {
                self.conn.execute(
                    "UPDATE tags SET name = ?2 WHERE id = ?1",
                    params![old_id, new],
                )?;
            }
        }
        Ok(())
    }

    /// Removes a tag from every task and deletes it.
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        let id = self.tag_id(name)?;
        self.conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_tag_color(&self, name: &str, color: Option<&str>) -> Result<()> {
        let id = self.tag_id(name)?;
        self.conn.execute(
            "UPDATE tags SET color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
        Ok(())
    }

    fn tag_id(&self, name: &str) -> Result<i64> {
        self.conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("tag `{name}`")))
    }
}
