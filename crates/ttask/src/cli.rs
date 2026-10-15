use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ttask_core::TaskId;

#[derive(Debug, Parser)]
#[command(name = "ttask", version, about, arg_required_else_help = true)]
pub struct Cli {
    /// Database file [default: platform data directory]
    #[arg(long, global = true, env = "TTASK_DB", value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Read the database password from this file instead of prompting
    #[arg(long, global = true, value_name = "PATH")]
    pub password_file: Option<PathBuf>,

    /// Print machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new database
    Init {
        /// Encrypt the database with a password (SQLCipher)
        #[arg(long)]
        encrypt: bool,
    },
    /// Add a task
    Add {
        /// Task title; several words are joined with spaces
        #[arg(required = true, num_args = 1..)]
        title: Vec<String>,
    },
    /// List tasks (open ones by default)
    #[command(visible_alias = "ls")]
    List {
        /// Include done and cancelled tasks
        #[arg(long, short)]
        all: bool,
    },
    /// Mark one or more tasks done
    Done {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<TaskId>,
    },
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
