CREATE TABLE projects (
  id          INTEGER PRIMARY KEY,
  uid         TEXT NOT NULL UNIQUE,
  parent_id   INTEGER REFERENCES projects(id),
  name        TEXT NOT NULL,
  color       TEXT,
  sort_order  REAL NOT NULL DEFAULT 0,
  archived_at INTEGER,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER
);

CREATE TABLE tasks (
  id             INTEGER PRIMARY KEY,
  uid            TEXT NOT NULL UNIQUE,
  project_id     INTEGER REFERENCES projects(id),
  parent_id      INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
  title          TEXT NOT NULL,
  notes          TEXT NOT NULL DEFAULT '',
  status         TEXT NOT NULL DEFAULT 'open'
                 CHECK (status IN ('open','done','cancelled')),
  priority       INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 3),
  due_date       TEXT,
  due_time       TEXT,
  scheduled_date TEXT,
  estimate_min   INTEGER,
  recur_rule     TEXT,
  recur_mode     TEXT CHECK (recur_mode IN ('due','completion')),
  recur_series   TEXT,
  sort_order     REAL NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  completed_at   INTEGER,
  deleted_at     INTEGER
);
CREATE INDEX tasks_open_due ON tasks(status, due_date) WHERE deleted_at IS NULL;
CREATE INDEX tasks_project  ON tasks(project_id, status);
CREATE INDEX tasks_parent   ON tasks(parent_id);

CREATE TABLE tags (
  id    INTEGER PRIMARY KEY,
  name  TEXT NOT NULL UNIQUE COLLATE NOCASE,
  color TEXT
);
CREATE TABLE task_tags (
  task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
  PRIMARY KEY (task_id, tag_id)
) WITHOUT ROWID;

CREATE TABLE reminders (
  id       INTEGER PRIMARY KEY,
  task_id  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  fire_at  INTEGER NOT NULL,
  fired_at INTEGER
);
CREATE INDEX reminders_pending ON reminders(fire_at) WHERE fired_at IS NULL;

CREATE TABLE task_deps (
  blocker_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  blocked_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  PRIMARY KEY (blocker_id, blocked_id)
) WITHOUT ROWID;

CREATE TABLE time_entries (
  id         INTEGER PRIMARY KEY,
  task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  started_at INTEGER NOT NULL,
  ended_at   INTEGER
);

CREATE TABLE saved_filters (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  query      TEXT NOT NULL,
  sort_order REAL NOT NULL DEFAULT 0
);

CREATE TABLE events (
  id         INTEGER PRIMARY KEY,
  task_id    INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  at         INTEGER NOT NULL,
  kind       TEXT NOT NULL,
  before     TEXT,
  after      TEXT,
  batch_id   INTEGER NOT NULL
);

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE VIRTUAL TABLE tasks_fts USING fts5(
  title, notes, content='tasks', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER tasks_fts_ai AFTER INSERT ON tasks BEGIN
  INSERT INTO tasks_fts(rowid, title, notes) VALUES (new.id, new.title, new.notes);
END;
CREATE TRIGGER tasks_fts_ad AFTER DELETE ON tasks BEGIN
  INSERT INTO tasks_fts(tasks_fts, rowid, title, notes)
  VALUES ('delete', old.id, old.title, old.notes);
END;
CREATE TRIGGER tasks_fts_au AFTER UPDATE OF title, notes ON tasks BEGIN
  INSERT INTO tasks_fts(tasks_fts, rowid, title, notes)
  VALUES ('delete', old.id, old.title, old.notes);
  INSERT INTO tasks_fts(rowid, title, notes) VALUES (new.id, new.title, new.notes);
END;
