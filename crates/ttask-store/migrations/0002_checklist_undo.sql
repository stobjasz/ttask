-- Undo/redo: an undone batch stays in the journal until a new action
-- discards it. Batch 0 holds imported history, which is never undoable.
-- task_id is no longer a foreign key: undoing a creation deletes the task row,
-- and its history must survive for redo.
CREATE TABLE events_new (
  id         INTEGER PRIMARY KEY,
  task_id    INTEGER,
  at         INTEGER NOT NULL,
  kind       TEXT NOT NULL,
  before     TEXT,
  after      TEXT,
  batch_id   INTEGER NOT NULL,
  undone     INTEGER NOT NULL DEFAULT 0
);
INSERT INTO events_new (id, task_id, at, kind, before, after, batch_id)
  SELECT id, task_id, at, kind, before, after, batch_id FROM events;
DROP TABLE events;
ALTER TABLE events_new RENAME TO events;
CREATE INDEX events_batch ON events(batch_id);
CREATE INDEX events_task ON events(task_id);

CREATE TABLE checklist_items (
  id         INTEGER PRIMARY KEY,
  task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  text       TEXT NOT NULL,
  checked    INTEGER NOT NULL DEFAULT 0,
  sort_order REAL NOT NULL DEFAULT 0
);
CREATE INDEX checklist_task ON checklist_items(task_id, sort_order);

CREATE INDEX task_tags_tag ON task_tags(tag_id);
CREATE INDEX task_deps_blocked ON task_deps(blocked_id);
CREATE INDEX time_entries_task ON time_entries(task_id);
CREATE INDEX projects_parent ON projects(parent_id);
