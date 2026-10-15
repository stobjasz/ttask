//! Application services: the single entry point both the CLI and (later) the
//! TUI use to change data.

use ttask_core::{Clock, NewTask, Task, TaskId};
use ttask_store::{Store, StoreError};

pub use ttask_store::TaskFilter;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("a task needs a title")]
    EmptyTitle,

    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

pub struct App {
    store: Store,
    clock: Box<dyn Clock>,
}

impl App {
    pub fn new(store: Store, clock: impl Clock + 'static) -> App {
        App {
            store,
            clock: Box::new(clock),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn add_task(&mut self, title: &str) -> Result<Task> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::EmptyTitle);
        }
        let now = self.clock.now_ms();
        Ok(self.store.insert_task(&NewTask::titled(title), now)?)
    }

    pub fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        Ok(self.store.list_tasks(filter)?)
    }

    pub fn complete_task(&mut self, id: TaskId) -> Result<Task> {
        let now = self.clock.now_ms();
        Ok(self.store.complete_task(id, now)?)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use ttask_core::{FixedClock, Status};

    use super::*;

    fn app(dir: &TempDir) -> App {
        let store = Store::create(&dir.path().join("t.db"), None).unwrap();
        App::new(store, FixedClock(42))
    }

    #[test]
    fn add_trims_title_and_stamps_clock() {
        let dir = TempDir::new().unwrap();
        let mut app = app(&dir);
        let task = app.add_task("  Pay rent \n").unwrap();
        assert_eq!(task.title, "Pay rent");
        assert_eq!(task.created_at, 42);
    }

    #[test]
    fn add_rejects_blank_title() {
        let dir = TempDir::new().unwrap();
        let mut app = app(&dir);
        assert!(matches!(app.add_task("   "), Err(AppError::EmptyTitle)));
        assert!(app.list_tasks(TaskFilter::All).unwrap().is_empty());
    }

    #[test]
    fn complete_moves_task_out_of_open_list() {
        let dir = TempDir::new().unwrap();
        let mut app = app(&dir);
        let task = app.add_task("Call plumber").unwrap();
        let done = app.complete_task(task.id).unwrap();
        assert_eq!(done.status, Status::Done);
        assert!(app.list_tasks(TaskFilter::Open).unwrap().is_empty());
    }
}
