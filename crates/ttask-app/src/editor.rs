//! `$EDITOR` handoff for notes and the `ttask edit` form.
//!
//! The text sits in a temporary plaintext file while the editor runs and is
//! deleted afterwards, even for an encrypted database.

use std::fs;
use std::io::{self, Write};
use std::process::Command;
use std::time::{Duration, Instant};

/// An editor that returns faster than this without changes did not wait.
const INSTANT_RETURN: Duration = Duration::from_secs(1);

/// `$VISUAL`, then `$EDITOR`, then `notepad` on Windows or `vi` elsewhere.
/// The value may carry arguments (`code --wait`).
pub fn editor_command() -> Vec<String> {
    let configured = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    match configured {
        Some(cmd) => split_command(&cmd),
        None if cfg!(windows) => vec!["notepad".into()],
        None => vec!["vi".into()],
    }
}

/// Splits on whitespace, honouring double quotes (`"C:\Program Files\x.exe" -w`).
pub(crate) fn split_command(cmd: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in cmd.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Opens `initial` in the editor and returns the saved text.
pub fn edit_text(initial: &str, suffix: &str) -> io::Result<String> {
    let mut file = tempfile::Builder::new()
        .prefix("ttask-")
        .suffix(suffix)
        .tempfile()?;
    file.write_all(initial.as_bytes())?;
    file.flush()?;
    // Close our handle so editors that replace the file can do so; the path
    // is still deleted when `path` drops.
    let path = file.into_temp_path();
    let command = editor_command();
    let (program, args) = command
        .split_first()
        .ok_or_else(|| io::Error::other("no editor configured"))?;
    let started = Instant::now();
    let status = Command::new(program)
        .args(args)
        .arg(&path)
        .status()
        .map_err(|e| {
            io::Error::other(format!(
                "could not start editor `{program}`: {e}; set $EDITOR"
            ))
        })?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "editor `{program}` exited with {status}"
        )));
    }
    let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
    // Some editors (Windows 11 Notepad, `code` without --wait) hand off to
    // another process and return at once, so the edit would be lost.
    if text == initial.replace("\r\n", "\n") && started.elapsed() < INSTANT_RETURN {
        return Err(io::Error::other(format!(
            "the editor `{program}` returned immediately, before any edit; set EDITOR to a \
             command that waits until you close the file, e.g. `code --wait` or `notepad++ -multiInst -nosession`"
        )));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_commands() {
        assert_eq!(split_command("code --wait"), ["code", "--wait"]);
        assert_eq!(
            split_command("\"C:\\Program Files\\ed.exe\" -w"),
            ["C:\\Program Files\\ed.exe", "-w"]
        );
    }
}
