mod cli;
mod password;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow};
use clap::Parser;
use ttask_app::{App, AppError, TaskFilter};
use ttask_core::{Status, SystemClock, Task};
use ttask_store::{DbMode, Store, StoreError};

use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            return ExitCode::from(if err.use_stderr() { 1 } else { 0 });
        }
    };
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(exit_code(&err))
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let db = db_path(cli.db.as_deref())?;
    match cli.command {
        Command::Init { encrypt } => {
            let password = if encrypt {
                Some(password::for_new_database(cli.password_file.as_deref())?)
            } else {
                None
            };
            let store = Store::create(&db, password.as_ref())?;
            let kind = match store.mode() {
                DbMode::Plain => "plain",
                DbMode::Encrypted => "encrypted",
            };
            println!("Created {kind} database at {}", db.display());
        }
        Command::Add { title } => {
            let mut app = open_app(&db, cli.password_file.as_deref())?;
            let task = app.add_task(&title.join(" "))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&task)?);
            } else {
                println!("Added task {}: {}", task.id, task.title);
            }
        }
        Command::List { all } => {
            let app = open_app(&db, cli.password_file.as_deref())?;
            let filter = if all {
                TaskFilter::All
            } else {
                TaskFilter::Open
            };
            let tasks = app.list_tasks(filter)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else if tasks.is_empty() {
                println!("No {}tasks.", if all { "" } else { "open " });
            } else {
                for task in &tasks {
                    println!("{}", format_task_line(task));
                }
            }
        }
        Command::Done { ids } => {
            let mut app = open_app(&db, cli.password_file.as_deref())?;
            let mut completed = Vec::new();
            let mut first_error = None;
            for id in ids {
                match app.complete_task(id) {
                    Ok(task) => {
                        if !cli.json {
                            println!("Completed task {}: {}", task.id, task.title);
                        }
                        completed.push(task);
                    }
                    Err(err) => {
                        eprintln!("error: {err}");
                        first_error.get_or_insert(err);
                    }
                }
            }
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&completed)?);
            }
            if let Some(err) = first_error {
                return Ok(ExitCode::from(exit_code(&err.into())));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn db_path(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    let dirs = directories::ProjectDirs::from("", "", "ttask")
        .ok_or_else(|| anyhow!("cannot determine a data directory; pass --db"))?;
    Ok(dirs.data_dir().join("ttask.db"))
}

fn open_app(db: &Path, password_file: Option<&Path>) -> anyhow::Result<App> {
    let mut passwords = password::for_existing_database(password_file)?;
    let store = Store::open(db, passwords.as_mut()).map_err(|err| match err {
        StoreError::Missing(_) => {
            anyhow!(err).context("run `ttask init` (or `ttask init --encrypt`) first")
        }
        other => anyhow!(other),
    });
    let store = store.with_context(|| format!("opening {}", db.display()))?;
    Ok(App::new(store, SystemClock))
}

fn format_task_line(task: &Task) -> String {
    let check = if task.status == Status::Open {
        "[ ]"
    } else {
        "[x]"
    };
    let priority = "!".repeat(task.priority.value().into());
    format!("{:>4}  {check} {priority:<3} {}", task.id, task.title)
}

/// Exit codes from the design doc: 1 usage/other, 2 not found, 3 auth failed,
/// 4 database locked or corrupt.
fn exit_code(err: &anyhow::Error) -> u8 {
    let store_err = err.chain().find_map(|cause| {
        cause
            .downcast_ref::<StoreError>()
            .or_else(|| match cause.downcast_ref::<AppError>() {
                Some(AppError::Store(inner)) => Some(inner),
                _ => None,
            })
    });
    match store_err {
        Some(StoreError::Missing(_) | StoreError::TaskNotFound(_)) => 2,
        Some(StoreError::WrongPassword | StoreError::PasswordUnavailable(_)) => 3,
        Some(StoreError::Locked | StoreError::NotADatabase | StoreError::NewerSchema { .. }) => 4,
        _ => 1,
    }
}
