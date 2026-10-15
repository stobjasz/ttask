-- wait:<date> hides a task from the usual views until that date;
-- someday parks it indefinitely (only the Someday view shows it).
ALTER TABLE tasks ADD COLUMN wait_until TEXT;
ALTER TABLE tasks ADD COLUMN someday INTEGER NOT NULL DEFAULT 0;
