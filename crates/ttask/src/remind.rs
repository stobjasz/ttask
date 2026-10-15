//! `ttask remind --install`: run `ttask remind --run` every 5 minutes via the
//! OS scheduler (Task Scheduler, launchd, systemd user timer).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

const TASK_NAME: &str = "ttask reminders";
const LAUNCHD_LABEL: &str = "com.ttask.remind";
const SYSTEMD_UNIT: &str = "ttask-remind";

/// Sends a desktop notification; failures are reported, not fatal.
pub fn notify(title: &str, body: &str) {
    let result = notify_rust::Notification::new()
        .appname("ttask")
        .summary(title)
        .body(body)
        .show();
    if let Err(e) = result {
        eprintln!("warning: desktop notification failed: {e}");
    }
}

fn quote(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// The scheduled command line (program + arguments).
pub fn run_args(exe: &Path, db: &Path) -> Vec<String> {
    vec![
        exe.display().to_string(),
        "remind".into(),
        "--run".into(),
        "--quiet".into(),
        "--db".into(),
        db.display().to_string(),
    ]
}

pub fn launchd_plist(exe: &Path, db: &Path) -> String {
    let args: String = run_args(exe, db)
        .iter()
        .map(|a| format!("    <string>{}</string>\n", xml_escape(a)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>StartInterval</key>
  <integer>300</integer>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

pub fn systemd_units(exe: &Path, db: &Path) -> (String, String) {
    let exec = run_args(exe, db)
        .iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let service = format!(
        "[Unit]\nDescription=ttask reminders\n\n[Service]\nType=oneshot\nExecStart={exec}\n"
    );
    let timer = "[Unit]\nDescription=Run ttask reminders every 5 minutes\n\n[Timer]\nOnCalendar=*:0/5\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_owned();
    (service, timer)
}

/// `schtasks /TR` value. `conhost --headless` keeps a console window from
/// flashing every five minutes.
pub fn schtasks_command(exe: &Path, db: &Path) -> String {
    format!(
        "conhost.exe --headless {} remind --run --quiet --db {}",
        quote(exe),
        quote(db)
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn run(cmd: &mut Command) -> anyhow::Result<()> {
    let status = cmd.status().with_context(|| format!("running {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed with {status}");
    }
    Ok(())
}

fn home() -> anyhow::Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_owned())
        .context("cannot find the home directory")
}

pub fn install(db: &Path) -> anyhow::Result<String> {
    let exe = std::env::current_exe().context("locating the ttask executable")?;
    let db = fs::canonicalize(db).unwrap_or_else(|_| db.to_owned());
    if cfg!(windows) {
        run(Command::new("schtasks").args([
            "/Create",
            "/F",
            "/SC",
            "MINUTE",
            "/MO",
            "5",
            "/TN",
            TASK_NAME,
            "/TR",
            &schtasks_command(&exe, &db),
        ]))?;
        Ok(format!(
            "Registered Task Scheduler task `{TASK_NAME}` (every 5 minutes)"
        ))
    } else if cfg!(target_os = "macos") {
        let path = home()?.join(format!("Library/LaunchAgents/{LAUNCHD_LABEL}.plist"));
        fs::create_dir_all(path.parent().expect("has parent"))?;
        fs::write(&path, launchd_plist(&exe, &db))?;
        run(Command::new("launchctl").arg("load").arg("-w").arg(&path))?;
        Ok(format!("Installed launchd agent {}", path.display()))
    } else {
        let dir = directories::BaseDirs::new()
            .map(|b| b.config_dir().join("systemd/user"))
            .context("cannot find the config directory")?;
        fs::create_dir_all(&dir)?;
        let (service, timer) = systemd_units(&exe, &db);
        fs::write(dir.join(format!("{SYSTEMD_UNIT}.service")), service)?;
        fs::write(dir.join(format!("{SYSTEMD_UNIT}.timer")), timer)?;
        let systemd =
            run(Command::new("systemctl").args(["--user", "daemon-reload"])).and_then(|_| {
                run(Command::new("systemctl").args([
                    "--user",
                    "enable",
                    "--now",
                    &format!("{SYSTEMD_UNIT}.timer"),
                ]))
            });
        match systemd {
            Ok(()) => Ok(format!("Enabled systemd user timer {SYSTEMD_UNIT}.timer")),
            Err(e) => Ok(format!(
                "Wrote units to {} but systemctl failed ({e}).\nWithout systemd, add this crontab line:\n*/5 * * * * {}",
                dir.display(),
                run_args(&exe, &db).join(" ")
            )),
        }
    }
}

pub fn uninstall() -> anyhow::Result<String> {
    if cfg!(windows) {
        run(Command::new("schtasks").args(["/Delete", "/F", "/TN", TASK_NAME]))?;
        Ok(format!("Removed Task Scheduler task `{TASK_NAME}`"))
    } else if cfg!(target_os = "macos") {
        let path = home()?.join(format!("Library/LaunchAgents/{LAUNCHD_LABEL}.plist"));
        let _ = run(Command::new("launchctl").arg("unload").arg(&path));
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok("Removed the launchd agent".into())
    } else {
        let _ = run(Command::new("systemctl").args([
            "--user",
            "disable",
            "--now",
            &format!("{SYSTEMD_UNIT}.timer"),
        ]));
        if let Some(dir) = directories::BaseDirs::new().map(|b| b.config_dir().join("systemd/user"))
        {
            for ext in ["service", "timer"] {
                let _ = fs::remove_file(dir.join(format!("{SYSTEMD_UNIT}.{ext}")));
            }
        }
        Ok("Removed the systemd user timer".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_units_reference_db_and_interval() {
        let exe = Path::new("/usr/local/bin/ttask");
        let db = Path::new("/home/me/My Tasks/ttask.db");
        let plist = launchd_plist(exe, db);
        assert!(plist.contains("<string>/home/me/My Tasks/ttask.db</string>"));
        assert!(plist.contains("<integer>300</integer>"));
        let (service, timer) = systemd_units(exe, db);
        assert!(service.contains("ExecStart=/usr/local/bin/ttask remind --run --quiet --db \"/home/me/My Tasks/ttask.db\""));
        assert!(timer.contains("OnCalendar=*:0/5"));
        let tr = schtasks_command(
            Path::new(r"C:\bin\ttask.exe"),
            Path::new(r"C:\Users\me\ttask.db"),
        );
        assert_eq!(
            tr,
            r#"conhost.exe --headless "C:\bin\ttask.exe" remind --run --quiet --db "C:\Users\me\ttask.db""#
        );
    }
}
