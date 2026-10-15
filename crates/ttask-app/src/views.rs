//! Built-in views are saved filters shipped with the app, so users can copy
//! and tweak them.

use ttask_core::{Project, SavedFilter, Task, TaskId};
use ttask_store::TaskSort;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub name: String,
    pub query: String,
    pub sort: TaskSort,
    pub kind: ViewKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Builtin,
    Project(i64),
    Tag(String),
    Saved(i64),
    Search,
}

impl View {
    fn builtin(name: &str, query: &str, sort: TaskSort) -> View {
        View {
            name: name.into(),
            query: query.into(),
            sort,
            kind: ViewKind::Builtin,
        }
    }

    pub fn inbox() -> View {
        View::builtin("Inbox", "#inbox", TaskSort::Manual)
    }

    pub fn today() -> View {
        View::builtin("Today", "due:<=today or sched:<=today", TaskSort::Smart)
    }

    pub fn upcoming() -> View {
        View::builtin("Upcoming", "due:>today or sched:>today", TaskSort::Smart)
    }

    pub fn overdue() -> View {
        View::builtin("Overdue", "is:overdue", TaskSort::Smart)
    }

    pub fn all() -> View {
        View::builtin("All", "", TaskSort::Smart)
    }

    pub fn logbook() -> View {
        View::builtin(
            "Logbook",
            "status:done or status:cancelled",
            TaskSort::Completed,
        )
    }

    /// Sidebar order.
    pub fn builtins() -> Vec<View> {
        vec![
            View::inbox(),
            View::today(),
            View::upcoming(),
            View::overdue(),
            View::all(),
            View::logbook(),
        ]
    }

    /// Looks up a built-in by name, case-insensitively (`ttask ls today`).
    pub fn builtin_named(name: &str) -> Option<View> {
        View::builtins()
            .into_iter()
            .find(|v| v.name.eq_ignore_ascii_case(name))
            .or_else(|| (name.eq_ignore_ascii_case("done")).then(View::logbook))
    }

    pub fn project(project: &Project) -> View {
        View {
            name: project.path.clone(),
            query: format!("project:\"{}\"", project.path),
            sort: TaskSort::Manual,
            kind: ViewKind::Project(project.id),
        }
    }

    pub fn tag(tag: &str) -> View {
        View {
            name: format!("+{tag}"),
            query: format!("+{tag}"),
            sort: TaskSort::Smart,
            kind: ViewKind::Tag(tag.to_owned()),
        }
    }

    pub fn saved(filter: &SavedFilter) -> View {
        View {
            name: filter.name.clone(),
            query: filter.query.clone(),
            sort: TaskSort::Smart,
            kind: ViewKind::Saved(filter.id),
        }
    }
}

/// Orders tasks so each subtask follows its parent (depth 1) when the parent
/// is in the list; orphaned subtasks keep their place at depth 0.
pub fn arrange_tree(tasks: Vec<Task>) -> Vec<(Task, u8)> {
    let ids: std::collections::HashSet<TaskId> = tasks.iter().map(|t| t.id).collect();
    let (children, roots): (Vec<Task>, Vec<Task>) = tasks
        .into_iter()
        .partition(|t| t.parent_id.is_some_and(|p| ids.contains(&p)));
    let mut out = Vec::with_capacity(roots.len() + children.len());
    for root in roots {
        let id = root.id;
        out.push((root, 0));
        for child in children.iter().filter(|c| c.parent_id == Some(id)) {
            out.push((child.clone(), 1));
        }
    }
    out
}
