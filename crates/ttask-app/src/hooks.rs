//! Hooks: user commands run after a change is saved, with the changed
//! tasks as JSON on stdin. Opt-in through `[hooks]` in the config; they see
//! decrypted data. They cannot change or veto anything: by the time they
//! run, the change is committed.
//!
//! One run per event per action: completing three tasks runs `on_complete`
//! once with all three. Imports, undo and redo do not run hooks.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ttask_core::{Task, TaskId};

use crate::App;

/// Receives hook problems (a failing or slow command).
pub type HookReporter = Box<dyn Fn(&str)>;

/// `[hooks]` in the config: a command per event, split like `$EDITOR`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HookConfig {
    pub on_add: Option<String>,
    pub on_modify: Option<String>,
    pub on_complete: Option<String>,
    pub on_delete: Option<String>,
    /// A hook still running after this long is killed (default 10).
    pub timeout_seconds: Option<u64>,
}

impl HookConfig {
    pub fn is_empty(&self) -> bool {
        self.on_add.is_none()
            && self.on_modify.is_none()
            && self.on_complete.is_none()
            && self.on_delete.is_none()
    }

    fn command(&self, event: &str) -> Option<&str> {
        match event {
            "add" => self.on_add.as_deref(),
            "modify" => self.on_modify.as_deref(),
            "complete" => self.on_complete.as_deref(),
            "delete" => self.on_delete.as_deref(),
            _ => None,
        }
        .filter(|c| !c.trim().is_empty())
    }
}

/// What a hook reads on stdin.
#[derive(Debug, Serialize)]
pub struct HookPayload<'a> {
    pub event: &'a str,
    /// The database file the change was made in.
    pub database: String,
    /// The tasks after the change (before it, for `delete`).
    pub tasks: &'a [Task],
}

/// The event a journal entry counts as, if any.
fn event_of(kind: &str) -> Option<&'static str> {
    match kind {
        "created" | "spawned" => Some("add"),
        "updated" | "reopened" => Some("modify"),
        "completed" => Some("complete"),
        "deleted" => Some("delete"),
        _ => None,
    }
}

impl App {
    /// Runs the configured hooks for journal batch `batch`.
    pub(crate) fn run_hooks(&mut self, batch: i64) {
        let Some(hooks) = self.hooks.clone() else {
            return;
        };
        let events = match self.store().db().batch_changes(batch) {
            Ok(events) => events,
            Err(e) => return self.hook_warning(format!("hooks: {e}")),
        };
        for event in ["add", "modify", "complete", "delete"] {
            let Some(command) = hooks.command(event) else {
                continue;
            };
            let mut ids: Vec<TaskId> = Vec::new();
            for (id, kind) in &events {
                if event_of(kind) == Some(event) && !ids.contains(id) {
                    ids.push(*id);
                }
            }
            if ids.is_empty() {
                continue;
            }
            let tasks: Vec<Task> = ids
                .iter()
                .filter_map(|id| match event {
                    "delete" => self.store().db().deleted_task(*id).ok().flatten(),
                    _ => self.task(*id).ok(),
                })
                .collect();
            let payload = HookPayload {
                event,
                database: self.store().path().display().to_string(),
                tasks: &tasks,
            };
            let timeout = Duration::from_secs(hooks.timeout_seconds.unwrap_or(10).max(1));
            if let Err(e) = run(command, event, &payload, timeout) {
                self.hook_warning(format!("hook on_{event}: {e}"));
            }
        }
    }

    fn hook_warning(&mut self, message: String) {
        match &self.hook_reporter {
            Some(report) => report(&message),
            None => self.hook_warnings.push(message),
        }
    }

    /// Enables hooks. Problems go to `report`, or, without one, are kept
    /// for [`App::take_hook_warnings`].
    pub fn set_hooks(&mut self, hooks: HookConfig, report: Option<HookReporter>) {
        self.hooks = (!hooks.is_empty()).then_some(hooks);
        self.hook_reporter = report;
    }

    pub fn take_hook_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.hook_warnings)
    }
}

fn run(
    command: &str,
    event: &str,
    payload: &HookPayload<'_>,
    timeout: Duration,
) -> Result<(), String> {
    let parts = crate::editor::split_command(command);
    let (program, args) = parts.split_first().ok_or("empty command")?;
    let json = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .env("TTASK_HOOK_EVENT", event)
        .env("TTASK_DB", &payload.database)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{program}`: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A hook that ignores its input closes the pipe early; that is fine.
        let _ = stdin.write_all(&json);
    }
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "still running after {}s; stopped it",
                    timeout.as_secs()
                ));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let stderr = reader.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let detail = stderr.trim().lines().last().unwrap_or("").to_owned();
        Err(match status.code() {
            Some(code) if detail.is_empty() => format!("exited with {code}"),
            Some(code) => format!("exited with {code}: {detail}"),
            None => "was killed".into(),
        })
    }
}
