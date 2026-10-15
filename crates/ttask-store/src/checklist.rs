use rusqlite::{OptionalExtension, params};
use ttask_core::{ChecklistItem, TaskId};

use crate::db::Db;
use crate::{Result, StoreError};

impl Db<'_> {
    pub fn checklist(&self, task: TaskId) -> Result<Vec<ChecklistItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, checked FROM checklist_items WHERE task_id = ?1
             ORDER BY sort_order, id",
        )?;
        let rows = stmt.query_map([task], |r| {
            Ok(ChecklistItem {
                id: r.get(0)?,
                text: r.get(1)?,
                checked: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_checklist_item(&self, task: TaskId, text: &str) -> Result<ChecklistItem> {
        self.task(task)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(StoreError::Invalid("checklist item needs text".into()));
        }
        self.conn.execute(
            "INSERT INTO checklist_items (task_id, text, sort_order)
             VALUES (?1, ?2, (SELECT coalesce(max(sort_order), 0) + 1 FROM checklist_items WHERE task_id = ?1))",
            params![task, text],
        )?;
        Ok(ChecklistItem {
            id: self.conn.last_insert_rowid(),
            text: text.to_owned(),
            checked: false,
        })
    }

    /// The task an item belongs to.
    pub fn checklist_item_task(&self, item: i64) -> Result<TaskId> {
        self.conn
            .query_row(
                "SELECT task_id FROM checklist_items WHERE id = ?1",
                [item],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("checklist item {item}")))
    }

    pub fn set_checklist_checked(&self, item: i64, checked: bool) -> Result<()> {
        self.checklist_item_task(item)?;
        self.conn.execute(
            "UPDATE checklist_items SET checked = ?2 WHERE id = ?1",
            params![item, checked],
        )?;
        Ok(())
    }

    pub fn edit_checklist_item(&self, item: i64, text: &str) -> Result<()> {
        self.checklist_item_task(item)?;
        self.conn.execute(
            "UPDATE checklist_items SET text = ?2 WHERE id = ?1",
            params![item, text.trim()],
        )?;
        Ok(())
    }

    pub fn remove_checklist_item(&self, item: i64) -> Result<()> {
        self.checklist_item_task(item)?;
        self.conn
            .execute("DELETE FROM checklist_items WHERE id = ?1", [item])?;
        Ok(())
    }
}
