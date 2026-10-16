//! `config.toml`: one file, every key optional.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub database: DatabaseConfig,
    pub security: SecurityConfig,
    pub ui: UiConfig,
    /// Action name → key, e.g. `done = "x"`.
    pub keys: BTreeMap<String, String>,
    /// Weights for the urgency score (the Next view, `--sort urgency`).
    pub urgency: ttask_core::urgency::UrgencyWeights,
    /// Pomodoro lengths for the TUI's focus mode.
    pub focus: ttask_tui::FocusSettings,
    /// Commands run after tasks are added, changed, completed or deleted.
    pub hooks: ttask_app::HookConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: Option<String>,
    pub backups: BackupsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackupsConfig {
    pub daily: u32,
    pub weekly: u32,
    pub dir: Option<String>,
}

impl Default for BackupsConfig {
    fn default() -> Self {
        BackupsConfig {
            daily: 7,
            weekly: 4,
            dir: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Remember {
    #[default]
    Never,
    Keychain,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub remember: Remember,
    pub idle_lock_minutes: Option<u32>,
    pub password_cmd: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub theme: ttask_tui::ThemeName,
    pub keymap: ttask_tui::KeymapName,
    pub show_detail_pane: ttask_tui::DetailPane,
    /// Clicks and the wheel in the TUI (default on).
    pub mouse: Option<bool>,
}

/// `TTASK_CONFIG`, else `<config dir>/ttask/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TTASK_CONFIG") {
        return Some(PathBuf::from(path));
    }
    directories::ProjectDirs::from("", "", "ttask").map(|d| d.config_dir().join("config.toml"))
}

pub fn load() -> anyhow::Result<(Config, Option<PathBuf>)> {
    let Some(path) = config_path() else {
        return Ok((Config::default(), None));
    };
    match fs::read_to_string(&path) {
        Ok(text) => {
            let config = toml::from_str(&text)
                .with_context(|| format!("reading config {}", path.display()))?;
            Ok((config, Some(path)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok((Config::default(), Some(path)))
        }
        Err(err) => Err(anyhow!(err).context(format!("reading config {}", path.display()))),
    }
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
    directories::ProjectDirs::from("", "", "ttask")
        .map(|d| d.data_dir().to_owned())
        .ok_or_else(|| anyhow!("cannot determine a data directory; pass --db"))
}

/// Expands a leading `~` to the home directory.
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path
        .strip_prefix("~")
        .filter(|r| r.is_empty() || r.starts_with(['/', '\\']))
    {
        if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_owned()) {
            return home.join(rest.trim_start_matches(['/', '\\']));
        }
    }
    PathBuf::from(path)
}

/// `--db` / `TTASK_DB`, then `database.path`, then the platform data dir.
pub fn db_path(explicit: Option<&Path>, config: &Config) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    if let Some(path) = &config.database.path {
        return Ok(expand(path));
    }
    Ok(data_dir()?.join("ttask.db"))
}

pub const EXAMPLE: &str = r#"# ttask configuration. Every key is optional.

[database]
# path = "~/Sync/ttask.db"
# backups = { daily = 7, weekly = 4 }   # set both to 0 to disable

[security]
# remember = "keychain"        # "never" | "keychain": cache the password in the OS keychain
# idle_lock_minutes = 30       # TUI re-locks after this long without input
# password_cmd = "pass show ttask"

[ui]
# theme = "auto"               # auto | dark | light | ascii
# keymap = "vim"               # vim | simple
# show_detail_pane = "auto"    # auto | always | never
# mouse = true                 # clicks and the wheel; false keeps plain text selection

[keys]
# done = "x"
# quick_add = "a"

[urgency]                      # weights for the Next view and --sort urgency
# next = 15.0                  # tagged +next
# due = 12.0                   # scaled by how close / overdue
# blocking = 8.0
# priority_urgent = 9.0
# priority_high = 6.0
# priority_medium = 3.9
# scheduled = 5.0
# active = 4.0                 # timer running
# age = 2.0
# tags = 1.0
# project = 1.0
# notes = 1.0
# blocked = -5.0

[focus]                        # focus mode (F in the TUI)
# work_minutes = 25
# short_break_minutes = 5
# long_break_minutes = 15
# long_break_every = 4

[hooks]                        # commands run after a change, with the tasks as JSON on stdin
# on_add = "~/bin/ttask-hook"  # they see decrypted data; leave unset unless you trust them
# on_modify = ""
# on_complete = ""
# on_delete = ""
# timeout_seconds = 10
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let config: Config = toml::from_str(EXAMPLE).unwrap();
        assert_eq!(config.database.backups.daily, 7);
    }

    #[test]
    fn full_config_parses() {
        let config: Config = toml::from_str(
            r#"
            [database]
            path = "~/x.db"
            backups = { daily = 3, weekly = 0 }
            [security]
            remember = "keychain"
            idle_lock_minutes = 10
            password_cmd = "echo hi"
            [ui]
            theme = "ascii"
            keymap = "simple"
            show_detail_pane = "never"
            [keys]
            done = "d"
            "#,
        )
        .unwrap();
        assert_eq!(config.security.remember, Remember::Keychain);
        assert_eq!(config.keys["done"], "d");
        assert!(
            toml::from_str::<Config>("[ui]\ncolour = 1").is_err(),
            "unknown keys rejected"
        );
    }
}
