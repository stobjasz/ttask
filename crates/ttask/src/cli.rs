use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ttask_core::TaskId;

/// A keyboard-driven personal task tracker. Run without a command for the TUI.
#[derive(Debug, Parser)]
#[command(name = "ttask", version, about)]
pub struct Cli {
    /// Database file [default: from config, else the platform data directory]
    #[arg(long, global = true, env = "TTASK_DB", value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Read the database password from this file instead of prompting
    #[arg(long, global = true, value_name = "PATH")]
    pub password_file: Option<PathBuf>,

    /// Print machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable colour (also honours NO_COLOR)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Print only errors
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new database
    Init {
        /// Encrypt the database with a password (SQLCipher)
        #[arg(long)]
        encrypt: bool,
    },
    /// Add a task: `ttask add Pay rent #home +bills !high due:fri every:month`
    Add {
        /// Title with quick-add tokens (put `--` before text starting with `-`)
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// Notes (Markdown)
        #[arg(long)]
        notes: Option<String>,
        /// Make it a subtask of this task
        #[arg(long, value_name = "ID")]
        parent: Option<TaskId>,
    },
    /// List tasks: a view (inbox, today, upcoming, overdue, all, logbook),
    /// a saved filter name, or a filter query
    #[command(visible_alias = "ls")]
    List {
        /// e.g. `today`, `#work and !>=2`, `not +someday`
        #[arg(num_args = 0..)]
        query: Vec<String>,
        /// Include done and cancelled tasks
        #[arg(long, short)]
        all: bool,
        /// smart | manual | priority | created | completed | title
        #[arg(long)]
        sort: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show a task with notes, checklist, subtasks and dependencies
    Show {
        id: TaskId,
        /// Include the change history
        #[arg(long)]
        history: bool,
    },
    /// Change tasks: `ttask edit 42 due:+3d -+later !high New title`;
    /// with no changes opens $EDITOR on a form. Put --notes/--parent before
    /// the changes.
    Edit {
        #[arg(required = true, value_name = "ID")]
        id: TaskId,
        /// More ids, then modifications
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        changes: Vec<String>,
        /// Replace the notes
        #[arg(long)]
        notes: Option<String>,
        /// Make it a subtask of this task (0 = top level)
        #[arg(long, value_name = "ID")]
        parent: Option<TaskId>,
    },
    /// Complete tasks (recurring tasks spawn their next instance)
    Done {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<TaskId>,
    },
    /// Reopen done or cancelled tasks
    Reopen {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<TaskId>,
    },
    /// Cancel tasks (closed, but not done)
    Cancel {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<TaskId>,
    },
    /// Delete tasks and their subtasks (undoable; purged by `gc` after 30 days)
    #[command(visible_alias = "delete")]
    Rm {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<TaskId>,
    },
    /// Move a recurring task to its next occurrence without completing it
    Skip { id: TaskId },
    /// Revert the last action
    Undo,
    /// Re-apply the last undone action
    Redo,
    /// Full-text search over titles and notes
    Search {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// Include done and cancelled tasks
        #[arg(long, short)]
        all: bool,
    },
    /// Manage a task's checklist
    #[command(visible_alias = "cl")]
    Checklist {
        task: TaskId,
        #[command(subcommand)]
        action: Option<ChecklistAction>,
    },
    /// Make a task wait on another
    Block {
        id: TaskId,
        #[arg(long = "on", value_name = "ID")]
        on: TaskId,
    },
    /// Remove a dependency
    Unblock {
        id: TaskId,
        #[arg(long = "on", value_name = "ID")]
        on: TaskId,
    },
    /// Time tracking
    Timer {
        #[command(subcommand)]
        action: TimerAction,
    },
    /// Reminders
    Remind(RemindArgs),
    /// Manage projects
    #[command(visible_alias = "projects")]
    Project {
        #[command(subcommand)]
        action: Option<ProjectAction>,
    },
    /// Manage tags
    #[command(visible_alias = "tags")]
    Tag {
        #[command(subcommand)]
        action: Option<TagAction>,
    },
    /// Manage saved filters
    #[command(visible_alias = "filters")]
    Filter {
        #[command(subcommand)]
        action: Option<FilterAction>,
    },
    /// Import tasks from a file
    Import {
        file: PathBuf,
        #[arg(long, value_enum)]
        format: Option<ImportFormat>,
        /// Show what would change without writing
        #[arg(long)]
        dry_run: bool,
        /// Put imported tasks without a project here
        #[arg(long)]
        project: Option<String>,
        /// CSV column mapping, e.g. title=Name,due=Deadline
        #[arg(long, value_delimiter = ',')]
        map: Vec<String>,
    },
    /// Export tasks
    Export {
        #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
        format: ExportFormat,
        /// Write to a file instead of stdout
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Encrypt the output with a passphrase (age); needs --output
        #[arg(long)]
        encrypt: bool,
    },
    /// Encryption and maintenance
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Remove soft-deleted tasks older than 30 days and old history
    Gc,
    /// Forget a password cached in the OS keychain
    Lock,
    /// Show or create the config file
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Print shell completions
    Completions { shell: clap_complete::Shell },
}

#[derive(Debug, Subcommand)]
pub enum ChecklistAction {
    /// Add an item
    Add {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Toggle an item (by its position, starting at 1)
    Toggle { item: usize },
    /// Remove an item (by its position)
    Rm { item: usize },
}

#[derive(Debug, Subcommand)]
pub enum TimerAction {
    /// Start timing a task (stops any running timer)
    Start { id: TaskId },
    /// Stop the running timer
    Stop,
    /// Show the running timer
    Status,
}

#[derive(Debug, Args)]
pub struct RemindArgs {
    /// Fire due reminders as desktop notifications and exit
    #[arg(long, conflicts_with_all = ["install", "uninstall"])]
    pub run: bool,
    /// Register `ttask remind --run` with the OS scheduler (every 5 minutes)
    #[arg(long, conflicts_with = "uninstall")]
    pub install: bool,
    /// Remove the scheduler entry
    #[arg(long)]
    pub uninstall: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProjectAction {
    /// List projects with open-task counts
    Ls {
        #[arg(long)]
        archived: bool,
    },
    /// Create a project (`work` or `work/clientA`)
    Add { path: String },
    /// Rename a project
    Rename { path: String, new_name: String },
    /// Hide a project from lists
    Archive { path: String },
    /// Show an archived project again
    Unarchive { path: String },
    /// Delete a project; its tasks move to the parent project or the inbox
    Rm { path: String },
}

#[derive(Debug, Subcommand)]
pub enum TagAction {
    /// List tags with open-task counts
    Ls,
    /// Rename a tag (merges into an existing one)
    Rename { old: String, new: String },
    /// Remove a tag from every task
    Rm { name: String },
}

#[derive(Debug, Subcommand)]
pub enum FilterAction {
    /// List saved filters
    Ls,
    /// Save a filter: `ttask filter save "This week" due:<=eow`
    Save {
        name: String,
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
    },
    /// Delete a saved filter
    Rm { name: String },
}

#[derive(Debug, Subcommand)]
pub enum DbAction {
    /// Print the database path, mode and size
    Info,
    /// Change the password of an encrypted database
    Passwd,
    /// Encrypt a plain database
    Encrypt,
    /// Decrypt to a plain database
    Decrypt {
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Write a backup copy (same encryption)
    Backup {
        /// Destination [default: backups/ttask-manual-<timestamp>.db]
        path: Option<PathBuf>,
    },
    /// Check integrity
    Check,
    /// Same as `ttask gc`
    Gc,
    /// Delete the .bak copy left by encrypt/decrypt/passwd
    GcBackups,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print the config file path
    Path,
    /// Write a commented example config if none exists
    Init,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ImportFormat {
    Json,
    Csv,
    Todotxt,
    Todoist,
    Ticktick,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
    Markdown,
    Todotxt,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
