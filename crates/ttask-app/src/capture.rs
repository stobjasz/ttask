//! The capture inbox: a plain-text file (typically in a synced folder, so a
//! phone can append to it) with one quick-add line per task. ttask turns the
//! lines into tasks and takes them out of the file. The database itself
//! never has to live in the synced folder.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::Serialize;
use ttask_core::Task;
use ttask_core::quickadd::{Mods, parse_mods};

use crate::{App, AppError, Result, insert_from_mods};

#[derive(Debug, Clone, Default, Serialize)]
pub struct CaptureReport {
    pub added: Vec<Task>,
    /// Lines kept as plain titles because their tokens did not parse.
    pub warnings: Vec<String>,
}

/// The task text of a captured line: bullets and empty checkboxes from
/// notes apps removed. `None` for blank lines.
fn clean(line: &str) -> Option<&str> {
    let mut text = line.trim().trim_start_matches('\u{feff}').trim();
    for bullet in ['-', '*', '•'] {
        match text.strip_prefix(bullet) {
            Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => {
                text = rest.trim_start();
                break;
            }
            _ => {}
        }
    }
    if let Some(rest) = text.strip_prefix("[ ]") {
        text = rest.trim_start();
    }
    (!text.is_empty()).then_some(text)
}

impl App {
    /// Adds one task per line, in one undo step. The active context does
    /// not apply: captured tasks land in the Inbox unless their tokens say
    /// otherwise. A line whose tokens do not parse becomes a task with the
    /// whole line as its title, so nothing is lost.
    pub fn capture<S: AsRef<str>>(&mut self, lines: &[S]) -> Result<CaptureReport> {
        let texts: Vec<&str> = lines.iter().filter_map(|l| clean(l.as_ref())).collect();
        let mut report = CaptureReport::default();
        if texts.is_empty() {
            // No batch: an empty one would still clear the redo stack.
            return Ok(report);
        }
        let (today, tz) = (self.today(), self.clock().time_zone());
        let mut parsed = Vec::new();
        for text in texts {
            match parse_mods(text, today) {
                Ok(mods) if mods.title.as_deref().is_some_and(|t| !t.trim().is_empty()) => {
                    parsed.push(mods)
                }
                outcome => {
                    let why = match outcome {
                        Err(e) => e.to_string(),
                        Ok(_) => "no title".into(),
                    };
                    report
                        .warnings
                        .push(format!("kept `{text}` as plain text ({why})"));
                    parsed.push(Mods {
                        title: Some(text.to_owned()),
                        ..Mods::default()
                    });
                }
            }
        }
        report.added = self.batch(|tx| {
            parsed
                .iter()
                .map(|mods| {
                    let title = mods.title.clone().unwrap_or_default();
                    insert_from_mods(tx, &title, mods, None, None, today, &tz)
                })
                .collect::<Result<Vec<_>>>()
        })?;
        Ok(report)
    }

    /// Captures the lines of the file at `path`, then removes them from it.
    /// Lines added to the file meanwhile (e.g. by a sync client) stay for
    /// next time. A missing file is an empty inbox.
    pub fn capture_file(&mut self, path: &Path) -> Result<CaptureReport> {
        let io_err = |e: std::io::Error| AppError::Invalid(format!("{}: {e}", path.display()));
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(CaptureReport::default()),
            Err(e) => return Err(io_err(e)),
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.iter().all(|l| clean(l).is_none()) {
            return Ok(CaptureReport::default());
        }
        // The tasks are saved before the file shrinks: a crash in between
        // captures a line twice rather than losing it.
        let report = self.capture(&lines)?;
        let mut remaining: Vec<String> = fs::read_to_string(path)
            .map_err(io_err)?
            .lines()
            .map(str::to_owned)
            .collect();
        for taken in &lines {
            if let Some(i) = remaining.iter().position(|l| l == taken) {
                remaining.remove(i);
            }
        }
        remaining.retain(|l| clean(l).is_some());
        let mut rest = remaining.join("\n");
        if !rest.is_empty() {
            rest.push('\n');
        }
        fs::write(path, rest).map_err(io_err)?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn cleans_bullets_and_checkboxes() {
        assert_eq!(clean("  - Buy milk +shop"), Some("Buy milk +shop"));
        assert_eq!(clean("* [ ] Call Bob due:tom"), Some("Call Bob due:tom"));
        assert_eq!(clean("\u{feff}• Pay rent"), Some("Pay rent"));
        assert_eq!(clean("+tag first"), Some("+tag first"));
        assert_eq!(clean("   "), None);
        assert_eq!(clean("- "), None);
    }
}
