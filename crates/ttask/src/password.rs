//! Where the database password comes from.
//!
//! `--password-file` alone when given (scripts must fail fast). Otherwise,
//! in order: `password_cmd` from the config, the OS keychain (if
//! `remember = "keychain"`), `TTASK_PASSWORD`, then an interactive prompt
//! with three attempts.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use secrecy::{ExposeSecret, SecretString};
use ttask_store::{PasswordSource, StoreError};

use crate::config::{Config, Remember};
use crate::keychain;

pub const ENV_VAR: &str = "TTASK_PASSWORD";
pub const MIN_LENGTH: usize = 10;
const PROMPT_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    File,
    Command,
    Keychain,
    Env,
    Prompt,
}

struct Candidate {
    origin: Origin,
    password: SecretString,
}

/// Tries each non-interactive candidate once, then prompts.
pub struct PasswordChain {
    candidates: Vec<Candidate>,
    prompt: bool,
    db: PathBuf,
    /// Which source produced the password of the latest attempt.
    pub last: Option<(Origin, SecretString)>,
}

impl PasswordChain {
    pub fn new(
        db: &Path,
        password_file: Option<&Path>,
        config: &Config,
    ) -> anyhow::Result<PasswordChain> {
        let mut candidates = Vec::new();
        if let Some(path) = password_file {
            candidates.push(Candidate {
                origin: Origin::File,
                password: read_password_file(path)?,
            });
            return Ok(PasswordChain {
                candidates,
                prompt: false,
                db: db.to_owned(),
                last: None,
            });
        }
        if let Some(cmd) = config
            .security
            .password_cmd
            .as_deref()
            .filter(|c| !c.trim().is_empty())
        {
            candidates.push(Candidate {
                origin: Origin::Command,
                password: run_password_cmd(cmd)?,
            });
        }
        if config.security.remember == Remember::Keychain {
            if let Some(password) = keychain::get(db) {
                candidates.push(Candidate {
                    origin: Origin::Keychain,
                    password,
                });
            }
        }
        if let Some(value) = std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty()) {
            candidates.push(Candidate {
                origin: Origin::Env,
                password: SecretString::from(value),
            });
        }
        Ok(PasswordChain {
            candidates,
            prompt: true,
            db: db.to_owned(),
            last: None,
        })
    }
}

impl PasswordSource for PasswordChain {
    fn password(&mut self, attempt: u32) -> ttask_store::Result<SecretString> {
        let index = attempt as usize - 1;
        if index > 0 {
            if let Some((Origin::Keychain, _)) = &self.last {
                // A stale keychain entry (password changed elsewhere): drop it.
                let _ = keychain::delete(&self.db);
            }
        }
        if let Some(candidate) = self.candidates.get(index) {
            if candidate.origin == Origin::Env {
                eprintln!(
                    "warning: using the password from {ENV_VAR}; prefer --password-file or the prompt"
                );
            }
            self.last = Some((candidate.origin, candidate.password.clone()));
            return Ok(candidate.password.clone());
        }
        if !self.prompt {
            return Err(StoreError::WrongPassword);
        }
        if index > self.candidates.len() || !self.candidates.is_empty() {
            eprintln!("Wrong password, try again.");
        }
        let password =
            prompt("Password: ").map_err(|e| StoreError::PasswordUnavailable(e.to_string()))?;
        self.last = Some((Origin::Prompt, password.clone()));
        Ok(password)
    }

    fn max_attempts(&self) -> u32 {
        self.candidates.len() as u32 + if self.prompt { PROMPT_ATTEMPTS } else { 0 }
    }
}

/// After a successful open: remember the password if configured to.
pub fn remember_if_configured(chain: &PasswordChain, config: &Config) {
    if config.security.remember != Remember::Keychain {
        return;
    }
    if let Some((origin, password)) = &chain.last {
        if *origin != Origin::Keychain {
            if let Err(e) = keychain::set(&chain.db, password) {
                eprintln!("warning: could not save the password to the keychain: {e}");
            }
        }
    }
}

/// Password for a new database or a new password: validated, confirmed when typed.
pub fn new_password(password_file: Option<&Path>, what: &str) -> anyhow::Result<SecretString> {
    let password = match password_file {
        Some(path) => read_password_file(path)?,
        None => match std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty()) {
            Some(value) => {
                eprintln!("warning: using the password from {ENV_VAR}");
                SecretString::from(value)
            }
            None => {
                eprintln!(
                    "ttask cannot recover a forgotten password. Store it in a password manager."
                );
                let first = prompt(&format!("{what}: "))?;
                let second = prompt("Repeat password: ")?;
                if first.expose_secret() != second.expose_secret() {
                    bail!("passwords do not match");
                }
                first
            }
        },
    };
    check_strength(&password)?;
    Ok(password)
}

pub fn check_strength(password: &SecretString) -> anyhow::Result<()> {
    let p = password.expose_secret();
    if p.chars().count() < MIN_LENGTH {
        bail!("password must be at least {MIN_LENGTH} characters");
    }
    let classes = [
        p.chars().any(|c| c.is_lowercase()),
        p.chars().any(|c| c.is_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    let strength = match (p.chars().count(), classes) {
        (n, _) if n >= 20 => "strong",
        (n, c) if n >= 14 && c >= 2 => "strong",
        (_, c) if c >= 3 => "fair",
        _ => "weak",
    };
    eprintln!("Password strength: {strength}");
    Ok(())
}

pub fn read_password_file(path: &Path) -> anyhow::Result<SecretString> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("reading password file {}", path.display()))?;
    let password = contents.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        bail!("password file {} is empty", path.display());
    }
    Ok(SecretString::from(password.to_owned()))
}

/// Runs `password_cmd` through the shell; its first output line is the password.
fn run_password_cmd(cmd: &str) -> anyhow::Result<SecretString> {
    let output = if cfg!(windows) {
        Command::new("cmd").args(["/C", cmd]).output()
    } else {
        Command::new("sh").args(["-c", cmd]).output()
    }
    .with_context(|| format!("running password_cmd `{cmd}`"))?;
    if !output.status.success() {
        bail!("password_cmd `{cmd}` failed with {}", output.status);
    }
    let text = String::from_utf8(output.stdout).context("password_cmd printed invalid UTF-8")?;
    let line = text.lines().next().unwrap_or("").to_owned();
    if line.is_empty() {
        bail!("password_cmd `{cmd}` printed nothing");
    }
    Ok(SecretString::from(line))
}

/// Prompts without echo. Refuses when stdin is not a terminal: on Windows
/// rpassword reads the console directly, so a script with redirected input
/// would otherwise block forever.
pub fn prompt(label: &str) -> std::io::Result<SecretString> {
    if !std::io::stdin().is_terminal() {
        return Err(std::io::Error::other(format!(
            "no terminal to prompt on; use --password-file or {ENV_VAR}"
        )));
    }
    rpassword::prompt_password(label).map(SecretString::from)
}
