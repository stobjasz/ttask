//! Automatic rotating backups: on the first open each day keep the last N
//! daily and M weekly copies.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{App, Result};

#[derive(Debug, Clone)]
pub struct BackupPolicy {
    pub dir: PathBuf,
    pub daily: u32,
    pub weekly: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RotationReport {
    pub created: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

const DAILY_PREFIX: &str = "ttask-daily-";
const WEEKLY_PREFIX: &str = "ttask-weekly-";

impl App {
    /// Creates today's daily and this week's weekly backup if missing, then
    /// prunes old ones. Cheap when nothing is due (two existence checks).
    pub fn rotate_backups(&self, policy: &BackupPolicy) -> Result<RotationReport> {
        let mut report = RotationReport::default();
        let today = self.today();
        let week = today.iso_week_date();
        let slots = [
            (DAILY_PREFIX, policy.daily, today.to_string()),
            (
                WEEKLY_PREFIX,
                policy.weekly,
                format!("{}-W{:02}", week.year(), week.week()),
            ),
        ];
        for (prefix, keep, stamp) in slots {
            if keep == 0 {
                continue;
            }
            let path = policy.dir.join(format!("{prefix}{stamp}.db"));
            if !path.exists() {
                self.store().backup_to(&path)?;
                report.created.push(path);
            }
            report
                .removed
                .extend(prune(&policy.dir, prefix, keep as usize)?);
        }
        Ok(report)
    }
}

/// Deletes all but the newest `keep` files named `<prefix>*.db`.
fn prune(dir: &Path, prefix: &str, keep: usize) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".db"))
            })
            .collect(),
        Err(_) => return Ok(Vec::new()),
    };
    // Names embed ISO dates, so lexical order is chronological.
    files.sort();
    let excess = files.len().saturating_sub(keep);
    let removed: Vec<PathBuf> = files.into_iter().take(excess).collect();
    for path in &removed {
        fs::remove_file(path).map_err(ttask_store::StoreError::from)?;
    }
    Ok(removed)
}
