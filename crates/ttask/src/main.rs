mod cli;
mod commands;
mod config;
mod form;
mod keychain;
mod output;
mod password;
mod remind;

use std::process::ExitCode;

use clap::Parser;
use ttask_app::AppError;
use ttask_store::StoreError;

use cli::Cli;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            return ExitCode::from(if err.use_stderr() { 1 } else { 0 });
        }
    };
    match commands::run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(exit_code(&err))
        }
    }
}

/// Exit codes from the design doc: 1 usage/other, 2 not found, 3 auth failed,
/// 4 database locked or corrupt.
pub fn exit_code(err: &anyhow::Error) -> u8 {
    let store_err = err.chain().find_map(|cause| {
        cause
            .downcast_ref::<StoreError>()
            .or_else(|| match cause.downcast_ref::<AppError>() {
                Some(AppError::Store(inner)) => Some(inner),
                _ => None,
            })
    });
    match store_err {
        Some(StoreError::Missing(_) | StoreError::TaskNotFound(_) | StoreError::NotFound(_)) => 2,
        Some(StoreError::WrongPassword | StoreError::PasswordUnavailable(_)) => 3,
        Some(
            StoreError::Locked
            | StoreError::NotADatabase
            | StoreError::NotTtask { .. }
            | StoreError::NewerSchema { .. },
        ) => 4,
        _ => 1,
    }
}
