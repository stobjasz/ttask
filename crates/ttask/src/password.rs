//! Where the database password comes from, in priority order:
//! `--password-file`, then `TTASK_PASSWORD`, then an interactive no-echo prompt.

use std::fs;
use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, bail};
use secrecy::{ExposeSecret, SecretString};
use ttask_store::{PasswordSource, StoreError};

const ENV_VAR: &str = "TTASK_PASSWORD";
const MIN_LENGTH: usize = 10;
const PROMPT_ATTEMPTS: u32 = 3;

/// Password for `init --encrypt`: validated, and confirmed when typed.
pub fn for_new_database(password_file: Option<&Path>) -> anyhow::Result<SecretString> {
    let password = match fixed_source(password_file)? {
        Some(mut source) => source.password(1)?,
        None => {
            eprintln!("ttask cannot recover a forgotten password. Store it in a password manager.");
            let first = prompt("New password: ")?;
            let second = prompt("Repeat password: ")?;
            if first.expose_secret() != second.expose_secret() {
                bail!("passwords do not match");
            }
            first
        }
    };
    if password.expose_secret().chars().count() < MIN_LENGTH {
        bail!("password must be at least {MIN_LENGTH} characters");
    }
    Ok(password)
}

/// Password source for opening a database; only consulted if the file is encrypted.
pub fn for_existing_database(
    password_file: Option<&Path>,
) -> anyhow::Result<Box<dyn PasswordSource>> {
    Ok(match fixed_source(password_file)? {
        Some(source) => Box::new(source),
        None => Box::new(Prompt),
    })
}

/// A non-interactive source: one attempt only, since retrying cannot help.
enum Fixed {
    File(SecretString),
    Env(SecretString),
}

impl PasswordSource for Fixed {
    fn password(&mut self, _attempt: u32) -> ttask_store::Result<SecretString> {
        Ok(match self {
            Fixed::File(password) => password.clone(),
            Fixed::Env(password) => {
                eprintln!(
                    "warning: using the password from {ENV_VAR}; prefer --password-file or the prompt"
                );
                password.clone()
            }
        })
    }
}

fn fixed_source(password_file: Option<&Path>) -> anyhow::Result<Option<Fixed>> {
    if let Some(path) = password_file {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("reading password file {}", path.display()))?;
        let password = contents.trim_end_matches(['\r', '\n']);
        if password.is_empty() {
            bail!("password file {} is empty", path.display());
        }
        return Ok(Some(Fixed::File(SecretString::from(password.to_owned()))));
    }
    Ok(std::env::var(ENV_VAR)
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| Fixed::Env(SecretString::from(value))))
}

struct Prompt;

impl PasswordSource for Prompt {
    fn password(&mut self, attempt: u32) -> ttask_store::Result<SecretString> {
        if attempt > 1 {
            eprintln!("Wrong password, try again.");
        }
        prompt("Password: ").map_err(|err| StoreError::PasswordUnavailable(err.to_string()))
    }

    fn max_attempts(&self) -> u32 {
        PROMPT_ATTEMPTS
    }
}

/// Prompts without echo. Refuses when stdin is not a terminal: on Windows
/// rpassword reads the console directly, so a script with redirected input
/// would otherwise block forever.
fn prompt(label: &str) -> std::io::Result<SecretString> {
    if !std::io::stdin().is_terminal() {
        return Err(std::io::Error::other(format!(
            "no terminal to prompt on; use --password-file or {ENV_VAR}"
        )));
    }
    rpassword::prompt_password(label).map(SecretString::from)
}
