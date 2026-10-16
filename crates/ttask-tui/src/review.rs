//! The guided weekly review (`W`): a fixed sequence of lists to go
//! through, acting on tasks with the usual keys; `]` / `[` move between
//! steps and `Esc` ends it.

use ttask_app::{View, ViewKind};
use ttask_core::Project;
use ttask_core::jiff::civil::Date;
use ttask_store::TaskSort;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Inbox,
    Overdue,
    Stale,
    Someday,
    Projects,
    Done,
}

pub const STEPS: [Step; 6] = [
    Step::Inbox,
    Step::Overdue,
    Step::Stale,
    Step::Someday,
    Step::Projects,
    Step::Done,
];

/// Untouched for this long counts as stale.
pub const STALE_DAYS: u32 = 30;

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Inbox => "Inbox",
            Step::Overdue => "Overdue",
            Step::Stale => "Stale",
            Step::Someday => "Someday",
            Step::Projects => "Projects without a next action",
            Step::Done => "Done",
        }
    }

    pub fn advice(self) -> &'static str {
        match self {
            Step::Inbox => "give each task a project (m) or a date (d), or delete it",
            Step::Overdue => "reschedule (d), finish (x) or drop (X) each one",
            Step::Stale => "untouched for 30 days: still worth doing?",
            Step::Someday => "bring back what you can start now (w, then empty)",
            Step::Projects => "add a next action with a, or archive the project",
            Step::Done => "review complete",
        }
    }

    /// The tasks this step shows; `None` for the steps that list something else.
    pub fn view(self) -> Option<View> {
        let view = |name: &str, query: String, sort| View {
            name: format!("Review: {name}"),
            query,
            sort,
            kind: ViewKind::Builtin,
        };
        match self {
            Step::Inbox => Some(View {
                name: "Review: Inbox".into(),
                ..View::inbox()
            }),
            Step::Overdue => Some(View {
                name: "Review: Overdue".into(),
                ..View::overdue()
            }),
            Step::Stale => Some(view(
                "Stale",
                format!("modified:<-{STALE_DAYS}d"),
                TaskSort::Created,
            )),
            Step::Someday => Some(View {
                name: "Review: Someday".into(),
                ..View::someday()
            }),
            Step::Projects | Step::Done => None,
        }
    }
}

pub struct Review {
    pub step: usize,
    /// Filled in on the Projects step.
    pub projects: Vec<Project>,
    pub project_idx: usize,
    /// When the previous review was finished.
    pub last: Option<Date>,
}

impl Review {
    pub fn current(&self) -> Step {
        STEPS[self.step]
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.projects.get(self.project_idx)
    }
}
