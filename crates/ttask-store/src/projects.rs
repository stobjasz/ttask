use rusqlite::{OptionalExtension, Row, params};
use ttask_core::{Project, ProjectId, new_uid};

use crate::db::Db;
use crate::{Result, StoreError};

const PROJECT_SELECT: &str = "SELECT p.id, p.uid, p.parent_id, p.name, p.color,
       p.archived_at IS NOT NULL AS archived,
       CASE WHEN pp.id IS NULL THEN p.name ELSE pp.name || '/' || p.name END AS path,
       (SELECT count(*) FROM tasks t WHERE t.project_id = p.id AND t.status = 'open'
          AND t.deleted_at IS NULL) AS open_tasks
  FROM projects p LEFT JOIN projects pp ON pp.id = p.parent_id";

impl Db<'_> {
    /// Finds or creates `parent` or `parent/child`; projects nest one level.
    pub fn ensure_project_path(&self, segments: &[String], now_ms: i64) -> Result<ProjectId> {
        match segments {
            [name] => self.ensure_project(name, None, now_ms),
            [parent, child] => {
                let parent_id = self.ensure_project(parent, None, now_ms)?;
                self.ensure_project(child, Some(parent_id), now_ms)
            }
            _ => Err(StoreError::Invalid(
                "projects nest one level: project or project/sub".into(),
            )),
        }
    }

    fn ensure_project(
        &self,
        name: &str,
        parent: Option<ProjectId>,
        now_ms: i64,
    ) -> Result<ProjectId> {
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return Err(StoreError::Invalid(format!(
                "invalid project name `{name}`"
            )));
        }
        let existing: Option<ProjectId> = self
            .conn
            .query_row(
                "SELECT id FROM projects WHERE name = ?1 COLLATE NOCASE AND parent_id IS ?2
                   AND deleted_at IS NULL",
                params![name, parent],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let sort_order: f64 = self.conn.query_row(
            "SELECT coalesce(max(sort_order), 0) + 1 FROM projects WHERE parent_id IS ?1",
            [parent],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO projects (uid, parent_id, name, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![new_uid(), parent, name, sort_order, now_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Looks up `name` or `parent/name`, case-insensitively.
    pub fn find_project(&self, path: &str) -> Result<Option<Project>> {
        let found = match path.split_once('/') {
            Some((parent, name)) => self.conn.query_row(
                &format!(
                    "{PROJECT_SELECT} WHERE p.deleted_at IS NULL AND p.name = ?1 COLLATE NOCASE
                       AND pp.name = ?2 COLLATE NOCASE"
                ),
                params![name, parent],
                project_from_row,
            ),
            // A bare name prefers a top-level project over a subproject.
            None => self.conn.query_row(
                &format!(
                    "{PROJECT_SELECT} WHERE p.deleted_at IS NULL AND p.name = ?1 COLLATE NOCASE
                     ORDER BY p.parent_id IS NOT NULL, p.id LIMIT 1"
                ),
                [path],
                project_from_row,
            ),
        };
        Ok(found.optional()?)
    }

    pub fn project(&self, id: ProjectId) -> Result<Project> {
        self.conn
            .query_row(
                &format!("{PROJECT_SELECT} WHERE p.id = ?1 AND p.deleted_at IS NULL"),
                [id],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("project {id}")))
    }

    /// Projects in sidebar order: each top-level project followed by its children.
    pub fn projects(&self, include_archived: bool) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(&format!(
            "{PROJECT_SELECT} WHERE p.deleted_at IS NULL AND (?1 OR p.archived_at IS NULL)
             ORDER BY coalesce(pp.sort_order, p.sort_order), coalesce(pp.id, p.id),
                      p.parent_id IS NOT NULL, p.sort_order, p.name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([include_archived], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn rename_project(&self, id: ProjectId, new_name: &str, now_ms: i64) -> Result<()> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.contains('/') {
            return Err(StoreError::Invalid(format!(
                "invalid project name `{new_name}`"
            )));
        }
        let project = self.project(id)?;
        let clash: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE name = ?1 COLLATE NOCASE
               AND parent_id IS ?2 AND id <> ?3 AND deleted_at IS NULL)",
            params![new_name, project.parent_id, id],
            |r| r.get(0),
        )?;
        if clash {
            return Err(StoreError::Invalid(format!(
                "a project named `{new_name}` already exists"
            )));
        }
        self.conn.execute(
            "UPDATE projects SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, new_name, now_ms],
        )?;
        Ok(())
    }

    pub fn set_project_archived(&self, id: ProjectId, archived: bool, now_ms: i64) -> Result<()> {
        self.project(id)?;
        self.conn.execute(
            "UPDATE projects SET archived_at = CASE WHEN ?2 THEN ?3 END, updated_at = ?3
             WHERE id = ?1 OR parent_id = ?1",
            params![id, archived, now_ms],
        )?;
        Ok(())
    }

    pub fn set_project_color(&self, id: ProjectId, color: Option<&str>, now_ms: i64) -> Result<()> {
        self.project(id)?;
        self.conn.execute(
            "UPDATE projects SET color = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, color, now_ms],
        )?;
        Ok(())
    }

    /// Deletes a project; its tasks move to the parent project (or the inbox)
    /// and its subprojects are deleted the same way. Returns the moved task count.
    pub fn delete_project(&self, id: ProjectId, now_ms: i64) -> Result<usize> {
        let project = self.project(id)?;
        let children: Vec<ProjectId> = self
            .conn
            .prepare("SELECT id FROM projects WHERE parent_id = ?1 AND deleted_at IS NULL")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for child in children {
            self.conn.execute(
                "UPDATE tasks SET project_id = ?2, updated_at = ?3 WHERE project_id = ?1",
                params![child, id, now_ms],
            )?;
            self.conn.execute(
                "UPDATE projects SET deleted_at = ?2 WHERE id = ?1",
                params![child, now_ms],
            )?;
        }
        let moved = self.conn.execute(
            "UPDATE tasks SET project_id = ?2, updated_at = ?3 WHERE project_id = ?1",
            params![id, project.parent_id, now_ms],
        )?;
        self.conn.execute(
            "UPDATE projects SET deleted_at = ?2 WHERE id = ?1",
            params![id, now_ms],
        )?;
        Ok(moved)
    }
}

fn project_from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        uid: row.get("uid")?,
        parent_id: row.get("parent_id")?,
        name: row.get("name")?,
        path: row.get("path")?,
        color: row.get("color")?,
        archived: row.get("archived")?,
        open_tasks: row.get("open_tasks")?,
    })
}
