use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

const PASSWORD: &str = "correct horse battery";

struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            dir: TempDir::new().unwrap(),
        }
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("tasks.db")
    }

    fn password_file(&self, password: &str) -> PathBuf {
        let path = self.dir.path().join(format!("pw-{}.txt", password.len()));
        fs::write(&path, format!("{password}\n")).unwrap();
        path
    }

    /// A `ttask` command pointed at this env's database, isolated from the
    /// developer's real environment.
    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("ttask").unwrap();
        cmd.arg("--db")
            .arg(self.db())
            .env_remove("TTASK_DB")
            .env_remove("TTASK_PASSWORD");
        cmd
    }

    fn cmd_with_password(&self, password: &str) -> Command {
        let mut cmd = self.cmd();
        cmd.arg("--password-file").arg(self.password_file(password));
        cmd
    }
}

fn json(output: &[u8]) -> Value {
    serde_json::from_slice(output).expect("valid JSON on stdout")
}

/// Runs the add → ls → done → ls flow against an initialised database.
fn exercise_task_flow(env: &Env, cmd: impl Fn() -> Command) {
    cmd()
        .args(["add", "Pay", "rent"])
        .assert()
        .success()
        .stdout("Added task 1: Pay rent\n");
    cmd().args(["add", "Call plumber"]).assert().success();

    let listed = cmd().args(["--json", "ls"]).assert().success();
    let tasks = json(&listed.get_output().stdout);
    let titles: Vec<_> = tasks
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, ["Pay rent", "Call plumber"]);
    assert_eq!(tasks[0]["status"], "open");

    cmd()
        .args(["done", "1"])
        .assert()
        .success()
        .stdout("Completed task 1: Pay rent\n");

    cmd().arg("ls").assert().success().stdout(
        predicate::str::contains("Call plumber").and(predicate::str::contains("Pay rent").not()),
    );

    let all = cmd().args(["--json", "ls", "--all"]).assert().success();
    assert_eq!(json(&all.get_output().stdout).as_array().unwrap().len(), 2);
    assert!(env.db().exists());
}

#[test]
fn plain_database_flow() {
    let env = Env::new();
    env.cmd()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("Created plain database"));
    exercise_task_flow(&env, || env.cmd());
}

#[test]
fn encrypted_database_flow() {
    let env = Env::new();
    env.cmd_with_password(PASSWORD)
        .args(["init", "--encrypt"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("Created encrypted database"));
    assert!(
        !fs::read(env.db())
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    exercise_task_flow(&env, || env.cmd_with_password(PASSWORD));
}

#[test]
fn wrong_password_exits_with_code_3() {
    let env = Env::new();
    env.cmd_with_password(PASSWORD)
        .args(["init", "--encrypt"])
        .assert()
        .success();
    env.cmd_with_password("wrong password!!")
        .arg("ls")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("wrong password"))
        .stderr(predicate::str::contains("hmac").not());
}

#[test]
fn no_password_source_without_terminal_fails_fast_with_code_3() {
    let env = Env::new();
    env.cmd_with_password(PASSWORD)
        .args(["init", "--encrypt"])
        .assert()
        .success();
    env.cmd()
        .arg("ls")
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no terminal to prompt on"));
}

#[test]
fn env_password_works_with_a_warning() {
    let env = Env::new();
    env.cmd_with_password(PASSWORD)
        .args(["init", "--encrypt"])
        .assert()
        .success();
    env.cmd()
        .env("TTASK_PASSWORD", PASSWORD)
        .arg("ls")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: using the password from TTASK_PASSWORD",
        ));
}

#[test]
fn short_password_is_rejected_at_init() {
    let env = Env::new();
    env.cmd_with_password("short")
        .args(["init", "--encrypt"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("at least 10 characters"));
    assert!(!env.db().exists());
}

#[test]
fn init_refuses_existing_database() {
    let env = Env::new();
    env.cmd().arg("init").assert().success();
    env.cmd()
        .arg("init")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn missing_database_exits_with_code_2_and_hint() {
    let env = Env::new();
    env.cmd()
        .arg("ls")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ttask init"));
}

#[test]
fn done_with_unknown_id_exits_with_code_2() {
    let env = Env::new();
    env.cmd().arg("init").assert().success();
    env.cmd().args(["add", "Real task"]).assert().success();
    env.cmd()
        .args(["done", "1", "99"])
        .assert()
        .code(2)
        .stdout("Completed task 1: Real task\n")
        .stderr(predicate::str::contains("task 99 not found"));
}

#[test]
fn blank_title_is_rejected() {
    let env = Env::new();
    env.cmd().arg("init").assert().success();
    env.cmd()
        .args(["add", "   "])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("needs a title"));
}

#[test]
fn usage_errors_exit_with_code_1() {
    let env = Env::new();
    env.cmd().arg("frobnicate").assert().code(1);
    env.cmd().arg("--help").assert().success();
}
