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
            .env("TTASK_CONFIG", self.dir.path().join("config.toml"))
            .env("NO_COLOR", "1")
            .env_remove("TTASK_DB")
            .env_remove("TTASK_PASSWORD")
            .env_remove("TTASK_EXPORT_PASSPHRASE");
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

fn init_plain(env: &Env) {
    env.cmd().arg("init").assert().success();
}

fn add_json(env: &Env, text: &str) -> Value {
    let out = env.cmd().args(["--json", "add", text]).assert().success();
    json(&out.get_output().stdout)
}

#[test]
fn quick_add_edit_and_show() {
    let env = Env::new();
    init_plain(&env);
    let task = add_json(
        &env,
        "Renew passport #admin +gov !high due:2026-11-30 every:year ~30m",
    );
    assert_eq!(task["project"], "admin");
    assert_eq!(task["tags"][0], "gov");
    assert_eq!(task["priority"], 2);
    assert_eq!(task["due_date"], "2026-11-30");
    assert_eq!(task["recur_rule"], "FREQ=YEARLY");
    assert_eq!(task["estimate_min"], 30);

    env.cmd()
        .args([
            "edit", "1", "-+gov", "+travel", "due:none", "New", "passport",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("New passport"));
    env.cmd()
        .args(["edit", "1", "--notes", "bring photos"])
        .assert()
        .success();

    let out = env
        .cmd()
        .args(["--json", "show", "1", "--history"])
        .assert()
        .success();
    let shown = json(&out.get_output().stdout);
    assert_eq!(shown["detail"]["task"]["title"], "New passport");
    assert_eq!(shown["detail"]["task"]["tags"][0], "travel");
    assert_eq!(shown["detail"]["task"]["notes"], "bring photos");
    assert!(shown["detail"]["task"]["due_date"].is_null());
    let kinds: Vec<&str> = shown["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["created", "updated", "updated"]);

    env.cmd()
        .args(["show", "1", "--history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bring photos"))
        .stdout(predicate::str::contains("History"));
}

#[test]
fn views_and_filters_from_the_command_line() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Inbox thing");
    add_json(&env, "Work today #work due:today");
    add_json(&env, "Work later #work due:+10d !urgent");
    let titles = |args: &[&str]| -> Vec<String> {
        let mut full = vec!["--json", "ls"];
        full.extend_from_slice(args);
        let out = env.cmd().args(&full).assert().success();
        json(&out.get_output().stdout)
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(titles(&["inbox"]), ["Inbox thing"]);
    assert_eq!(titles(&["today"]), ["Work today"]);
    assert_eq!(titles(&["#work", "and", "!>=2"]), ["Work later"]);
    assert_eq!(
        titles(&["#work", "--sort", "priority"]),
        ["Work later", "Work today"]
    );

    env.cmd()
        .args(["filter", "save", "Urgent", "!urgent"])
        .assert()
        .success();
    assert_eq!(
        titles(&["urgent"]),
        ["Work later"],
        "saved filters work as views"
    );
    env.cmd()
        .args(["ls", "due:<=banana"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("banana"));
}

#[test]
fn done_with_recurrence_undo_and_redo() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Water plants every!:3d due:today");
    env.cmd()
        .args(["done", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Completed task 1: Water plants (next: 2",
        ));
    env.cmd()
        .arg("undo")
        .assert()
        .success()
        .stdout(predicate::str::contains("Undid completed, spawned"));
    env.cmd().arg("redo").assert().success();
    env.cmd().arg("undo").assert().success();
    env.cmd().arg("undo").assert().success();
    env.cmd()
        .arg("undo")
        .assert()
        .success()
        .stdout("Nothing to undo\n");
}

#[test]
fn projects_tags_checklist_search_block_timer() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Client call #work/acme +phone");
    add_json(&env, "Prep slides #work");
    env.cmd()
        .args(["project", "ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("work/acme"));
    env.cmd()
        .args(["project", "rename", "work/acme", "Initech"])
        .assert()
        .success();
    env.cmd()
        .args(["tag", "rename", "phone", "call"])
        .assert()
        .success();
    env.cmd()
        .args(["tags"])
        .assert()
        .success()
        .stdout(predicate::str::contains("+call"));

    env.cmd()
        .args(["checklist", "2", "add", "outline"])
        .assert()
        .success();
    env.cmd()
        .args(["checklist", "2", "add", "visuals"])
        .assert()
        .success();
    env.cmd()
        .args(["checklist", "2", "toggle", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1. [x] outline"));
    env.cmd()
        .args(["checklist", "2", "toggle", "9"])
        .assert()
        .code(2);

    env.cmd()
        .args(["search", "slid"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Prep slides"));

    env.cmd()
        .args(["block", "1", "--on", "2"])
        .assert()
        .success();
    env.cmd()
        .args(["ls", "is:blocked"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Client call"));
    env.cmd().args(["block", "2", "--on", "1"]).assert().code(1);

    env.cmd().args(["timer", "start", "2"]).assert().success();
    env.cmd()
        .args(["timer", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Timing 2: Prep slides"));
    env.cmd().args(["timer", "stop"]).assert().success();
}

#[test]
fn json_export_import_round_trip_and_dry_run() {
    let src = Env::new();
    init_plain(&src);
    add_json(&src, "Exported #proj +tag due:2026-12-01");
    add_json(&src, "Second");
    let export = src.dir.path().join("out.json");
    src.cmd()
        .args(["export", "--format", "json", "--output"])
        .arg(&export)
        .assert()
        .success();

    let dst = Env::new();
    init_plain(&dst);
    dst.cmd()
        .args(["import", "--dry-run"])
        .arg(&export)
        .assert()
        .success()
        .stdout(predicate::str::contains("Would import: 2 new"));
    dst.cmd()
        .arg("import")
        .arg(&export)
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported: 2 new, 0 updated"));
    dst.cmd()
        .arg("import")
        .arg(&export)
        .assert()
        .success()
        .stdout(predicate::str::contains("0 new, 2 updated"));
    dst.cmd()
        .args(["ls", "#proj"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));
}

#[test]
fn other_formats_export_and_import() {
    let src = Env::new();
    init_plain(&src);
    add_json(&src, "Formatted #proj +tag due:2026-12-01");
    for (format, file) in [
        ("csv", "out.csv"),
        ("todotxt", "out.txt"),
        ("markdown", "out.md"),
    ] {
        let path = src.dir.path().join(file);
        src.cmd()
            .args(["export", "--format", format, "--output"])
            .arg(&path)
            .assert()
            .success();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Formatted"), "{format}: {text}");
        if format == "markdown" {
            continue;
        }
        let dst = Env::new();
        init_plain(&dst);
        dst.cmd().arg("import").arg(&path).assert().success();
        let out = dst.cmd().args(["--json", "ls"]).assert().success();
        let tasks = json(&out.get_output().stdout);
        assert_eq!(tasks[0]["title"], "Formatted", "{format}");
        assert_eq!(tasks[0]["project"], "proj", "{format}");
        assert_eq!(tasks[0]["due_date"], "2026-12-01", "{format}");
    }
}

#[test]
fn todoist_csv_is_detected() {
    let env = Env::new();
    init_plain(&env);
    let path = env.dir.path().join("Groceries.csv");
    fs::write(
        &path,
        "TYPE,CONTENT,DESCRIPTION,PRIORITY,INDENT,AUTHOR,RESPONSIBLE,DATE,DATE_LANG,TIMEZONE\n\
         task,Milk @shop,,4,1,,,,en,UTC\n",
    )
    .unwrap();
    env.cmd().arg("import").arg(&path).assert().success();
    env.cmd()
        .args(["ls", "#Groceries", "+shop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Milk"));
}

#[test]
fn encrypted_export_round_trips() {
    let src = Env::new();
    init_plain(&src);
    add_json(&src, "Top secret");
    let path = src.dir.path().join("backup.age");
    src.cmd()
        .args(["export", "--encrypt", "--output"])
        .arg(&path)
        .env("TTASK_EXPORT_PASSPHRASE", "an export passphrase")
        .assert()
        .success();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"age-encryption.org/v1"));
    assert!(!bytes.windows(10).any(|w| w == b"Top secret"));

    let dst = Env::new();
    init_plain(&dst);
    dst.cmd()
        .arg("import")
        .arg(&path)
        .env("TTASK_EXPORT_PASSPHRASE", "wrong passphrase!!")
        .assert()
        .failure();
    dst.cmd()
        .arg("import")
        .arg(&path)
        .env("TTASK_EXPORT_PASSPHRASE", "an export passphrase")
        .assert()
        .success();
    dst.cmd()
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("Top secret"));
}

#[test]
fn encrypt_change_password_and_decrypt() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Keep me");
    env.cmd()
        .args(["db", "encrypt"])
        .env("TTASK_PASSWORD", PASSWORD)
        .assert()
        .success()
        .stdout(predicate::str::contains("Encrypted the database"));
    assert!(
        !fs::read(env.db())
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    env.cmd_with_password(PASSWORD)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("Keep me"));

    let new_password = "a brand new password";
    env.cmd_with_password(PASSWORD)
        .args(["db", "passwd"])
        .env("TTASK_PASSWORD", new_password)
        .assert()
        .success();
    env.cmd_with_password(PASSWORD).arg("ls").assert().code(3);
    env.cmd_with_password(new_password)
        .args(["db", "decrypt"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--yes"));
    env.cmd_with_password(new_password)
        .args(["db", "decrypt", "--yes"])
        .assert()
        .success();
    assert!(
        fs::read(env.db())
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    env.cmd()
        .args(["db", "gc-backups"])
        .assert()
        .success()
        .stdout("Removed the .bak copy\n");
    env.cmd()
        .args(["db", "encrypt"])
        .env("TTASK_PASSWORD", "short")
        .assert()
        .code(1);
}

#[test]
fn backups_check_info_gc() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Something");
    let backups = env.dir.path().join("backups");
    let rotated = fs::read_dir(&backups).unwrap().count();
    assert_eq!(rotated, 2, "daily and weekly rotation on first open");
    env.cmd()
        .args(["db", "backup"])
        .arg(env.dir.path().join("manual.db"))
        .assert()
        .success();
    assert!(env.dir.path().join("manual.db").exists());
    env.cmd()
        .args(["db", "check"])
        .assert()
        .success()
        .stdout("Database OK\n");
    let out = env.cmd().args(["--json", "db", "info"]).assert().success();
    let info = json(&out.get_output().stdout);
    assert_eq!(info["mode"], "plain");
    assert_eq!(info["open_tasks"], 1);
    env.cmd()
        .arg("gc")
        .assert()
        .success()
        .stdout(predicate::str::contains("Purged 0 task(s)"));
}

#[test]
fn config_file_is_honoured() {
    let env = Env::new();
    let config = env.dir.path().join("config.toml");
    env.cmd().args(["config", "init"]).assert().success();
    assert!(fs::read_to_string(&config).unwrap().contains("[security]"));
    fs::write(&config, "[database]\nbackups = { daily = 0, weekly = 0 }\n").unwrap();
    init_plain(&env);
    add_json(&env, "No backups");
    assert!(!env.dir.path().join("backups").exists());
    fs::write(&config, "[ui]\ncolour = true\n").unwrap();
    env.cmd()
        .arg("ls")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("config"));
}

#[test]
fn completions_and_reminder_listing() {
    let env = Env::new();
    env.cmd()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ttask"));
    init_plain(&env);
    add_json(&env, "Dentist due:2099-01-01@10:00 remind:due-1h");
    env.cmd()
        .arg("remind")
        .assert()
        .success()
        .stdout(predicate::str::contains("09:00").and(predicate::str::contains("Dentist")));
    env.cmd().args(["remind", "--run"]).assert().success();
}

#[test]
fn tui_refuses_without_a_terminal() {
    let env = Env::new();
    init_plain(&env);
    env.cmd()
        .assert()
        .code(1)
        .stderr(predicate::str::contains("needs a terminal"));
}

fn ls_titles(env: &Env, args: &[&str]) -> Vec<String> {
    let mut full = vec!["--json", "ls"];
    full.extend_from_slice(args);
    let out = env.cmd().args(&full).assert().success();
    json(&out.get_output().stdout)
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn next_view_sorts_by_urgency() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Plain");
    add_json(&env, "Overdue due:-10d");
    add_json(&env, "Flagged +next");
    assert_eq!(ls_titles(&env, &["next"]), ["Flagged", "Overdue", "Plain"]);
    env.cmd()
        .args(["ls", "--sort", "urgency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("u15.8"));
    env.cmd()
        .args(["show", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("urgency"));
}

#[test]
fn urgency_weights_from_config() {
    let env = Env::new();
    fs::write(
        env.dir.path().join("config.toml"),
        "[urgency]\nnext = 0.0\ndue = 30.0\n",
    )
    .unwrap();
    init_plain(&env);
    add_json(&env, "Flagged +next");
    add_json(&env, "Overdue due:-10d");
    assert_eq!(ls_titles(&env, &["next"]), ["Overdue", "Flagged"]);
}

#[test]
fn wait_and_someday_from_the_command_line() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Visible");
    add_json(&env, "Follow up wait:+3d");
    add_json(&env, "Learn piano wait:someday");
    assert_eq!(ls_titles(&env, &[]), ["Visible"]);
    assert_eq!(ls_titles(&env, &["waiting"]), ["Follow up"]);
    assert_eq!(ls_titles(&env, &["someday"]), ["Learn piano"]);
    assert_eq!(ls_titles(&env, &["--all"]).len(), 3);
    env.cmd()
        .args(["ls", "is:waiting"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wait "));
    env.cmd()
        .args(["edit", "3", "wait:none"])
        .assert()
        .success();
    assert_eq!(ls_titles(&env, &[]).len(), 2);
}

#[test]
fn contexts_from_the_command_line() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Home task #home");
    add_json(&env, "Work task #work");
    env.cmd()
        .arg("context")
        .assert()
        .success()
        .stdout("No context\n");
    env.cmd()
        .args(["context", "define", "work", "#work"])
        .assert()
        .success()
        .stdout("Context: work\n");
    assert_eq!(ls_titles(&env, &[]), ["Work task"]);
    assert_eq!(ls_titles(&env, &["--no-context"]).len(), 2);
    let added = add_json(&env, "Inherits");
    assert_eq!(added["project"], "work");
    env.cmd()
        .arg("context")
        .assert()
        .success()
        .stdout(predicate::str::contains("work  #work"));
    env.cmd().args(["context", "none"]).assert().success();
    assert_eq!(ls_titles(&env, &[]).len(), 3);
    env.cmd().args(["context", "nope"]).assert().code(2);
}

#[test]
fn ics_export_with_filter() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Dentist #health due:2026-11-03@10:00 remind:due-1h");
    add_json(&env, "Pay rent #home due:2026-11-01 every:month");
    add_json(&env, "No date #home");
    let path = env.dir.path().join("tasks.ics");
    env.cmd()
        .args(["export", "--format", "ics", "--output"])
        .arg(&path)
        .assert()
        .success();
    let ics = fs::read_to_string(&path).unwrap();
    assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
    assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
    assert!(ics.contains("RRULE:FREQ=MONTHLY"));
    assert!(ics.contains("BEGIN:VALARM"));
    let out = env
        .cmd()
        .args(["export", "--format", "ics", "--filter", "#home"])
        .assert()
        .success();
    let filtered = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(filtered.matches("BEGIN:VEVENT").count(), 1);
    assert!(filtered.contains("SUMMARY:Pay rent"));
    env.cmd()
        .args(["export", "--format", "csv", "--filter", "#home"])
        .assert()
        .code(1);
}

#[test]
fn taskwarrior_export_is_detected_and_imported() {
    let env = Env::new();
    init_plain(&env);
    let path = env.dir.path().join("tw.json");
    fs::write(
        &path,
        r#"[{"uuid":"a1","description":"From taskwarrior","status":"pending","project":"Work.Acme","tags":["next"],"priority":"H","entry":"20261001T090000Z","due":"20261020T000000Z"},
{"uuid":"a2","description":"Deleted","status":"deleted","entry":"20261001T090000Z"}]"#,
    )
    .unwrap();
    env.cmd()
        .arg("import")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Imported: 1 new, 0 updated task(s), 2 new project(s)",
        ))
        .stderr(predicate::str::contains("1 deleted"));
    let out = env.cmd().args(["--json", "ls"]).assert().success();
    let tasks = json(&out.get_output().stdout);
    assert_eq!(tasks[0]["title"], "From taskwarrior");
    assert_eq!(tasks[0]["project"], "Work/Acme");
    assert_eq!(tasks[0]["uid"], "a1");
    assert_eq!(tasks[0]["priority"], 2);
}

#[test]
fn github_issues_are_detected_and_reimport_updates() {
    let env = Env::new();
    init_plain(&env);
    let path = env.dir.path().join("issues.json");
    let issues = |title: &str| {
        format!(
            r#"[{{"number":7,"title":"{title}","body":"Details","url":"https://github.com/acme/rocket/issues/7","state":"OPEN","labels":[{{"name":"bug"}}],"createdAt":"2026-10-01T10:00:00Z"}},
{{"number":8,"title":"Done already","body":"","url":"https://github.com/acme/rocket/issues/8","state":"CLOSED","labels":[],"closedAt":"2026-10-02T10:00:00Z"}}]"#
        )
    };
    fs::write(&path, issues("Crash on start")).unwrap();
    env.cmd()
        .arg("import")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported: 2 new, 0 updated"));
    fs::write(&path, issues("Crash on start (renamed)")).unwrap();
    env.cmd()
        .arg("import")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported: 0 new, 2 updated"));
    let out = env
        .cmd()
        .args(["--json", "ls", "#rocket"])
        .assert()
        .success();
    let tasks = json(&out.get_output().stdout);
    assert_eq!(tasks.as_array().unwrap().len(), 1, "{tasks}");
    assert_eq!(tasks[0]["title"], "Crash on start (renamed)");
    assert_eq!(tasks[0]["tags"][0], "bug");

    // The issue URL is the first link, so `ttask open` goes to the issue.
    let id = tasks[0]["id"].to_string();
    env.cmd()
        .args(["open", &id, "--print"])
        .assert()
        .success()
        .stdout("https://github.com/acme/rocket/issues/7\n");
}

#[test]
fn count_prints_a_number() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "One due:today +x");
    add_json(&env, "Two +x");
    add_json(&env, "Three");
    env.cmd().arg("count").assert().success().stdout("3\n");
    env.cmd()
        .args(["count", "+x"])
        .assert()
        .success()
        .stdout("2\n");
    env.cmd()
        .args(["count", "today"])
        .assert()
        .success()
        .stdout("1\n");
    let out = env.cmd().args(["--json", "count", "-a"]).assert().success();
    assert_eq!(json(&out.get_output().stdout)["count"], 3);
}

#[test]
fn open_lists_links_and_refuses_ambiguity() {
    let env = Env::new();
    init_plain(&env);
    let one = add_json(
        &env,
        "Read https://example.com/a and https://example.com/b.",
    );
    let none = add_json(&env, "No links");
    let id = one["id"].to_string();
    env.cmd()
        .args(["open", &id])
        .assert()
        .code(1)
        .stdout("  1. https://example.com/a\n  2. https://example.com/b\n")
        .stderr(predicate::str::contains("has 2 links"));
    env.cmd()
        .args(["open", &id, "2", "--print"])
        .assert()
        .success()
        .stdout("https://example.com/b\n");
    env.cmd()
        .args(["open", &id, "3", "--print"])
        .assert()
        .code(2);
    env.cmd()
        .args(["open", &none["id"].to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no http, https or mailto links"));
}

#[test]
fn bulk_actions_by_filter_need_yes_and_undo_together() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Pay rent +bills");
    add_json(&env, "Pay water +bills");
    add_json(&env, "Unrelated");
    env.cmd()
        .args(["done", "--filter", "+bills"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("pass --yes"));
    env.cmd()
        .args(["done", "--filter", "+bills", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed 2 tasks"));
    assert_eq!(ls_titles(&env, &[]), ["Unrelated"]);
    env.cmd().arg("undo").assert().success();
    assert_eq!(ls_titles(&env, &[]).len(), 3);

    // One match needs no --yes; reopen looks at closed tasks.
    env.cmd()
        .args(["cancel", "--filter", "Unrelated"])
        .assert()
        .success();
    env.cmd()
        .args(["reopen", "--filter", "Unrelated"])
        .assert()
        .success()
        .stdout("Reopened task 3: Unrelated\n");
    env.cmd()
        .args(["rm", "--filter", "+bills", "-y"])
        .assert()
        .success();
    assert_eq!(ls_titles(&env, &[]), ["Unrelated"]);
    env.cmd()
        .args(["rm", "--filter", "+nothing"])
        .assert()
        .success()
        .stdout("No tasks match.\n");
    env.cmd()
        .args(["done", "1", "--filter", "+bills"])
        .assert()
        .code(1);
}

#[test]
fn files_that_are_not_ttask_databases_are_refused() {
    let env = Env::new();
    let notes = env.dir.path().join("notes.txt");
    fs::write(&notes, "hello").unwrap();
    let mut cmd = Command::cargo_bin("ttask").unwrap();
    cmd.arg("--db")
        .arg(&notes)
        .arg("ls")
        .env("TTASK_CONFIG", env.dir.path().join("config.toml"))
        .env_remove("TTASK_DB")
        .assert()
        .code(4)
        .stderr(predicate::str::contains("not a ttask database"));
    assert_eq!(fs::read_to_string(&notes).unwrap(), "hello");
}

#[test]
fn templates_from_the_command_line_survive_export() {
    let env = Env::new();
    init_plain(&env);
    let root = add_json(&env, "Release #work due:+2d");
    let id = root["id"].to_string();
    env.cmd()
        .args(["add", "--parent", &id, "Changelog"])
        .assert()
        .success();
    env.cmd()
        .args(["template", "save", "release", "--from", &id])
        .assert()
        .success()
        .stdout("Saved template `release`: task \"Release\" with 1 subtask(s)\n");
    env.cmd()
        .args(["template", "use", "release", "Release v2", "+big"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created").and(predicate::str::contains("Release v2")));
    let titles = ls_titles(&env, &["#work"]);
    assert_eq!(
        titles.iter().filter(|t| *t == "Changelog").count(),
        2,
        "{titles:?}"
    );

    let export = env.dir.path().join("all.json");
    env.cmd()
        .args(["export", "--output"])
        .arg(&export)
        .assert()
        .success();
    let other = Env::new();
    init_plain(&other);
    other
        .cmd()
        .arg("import")
        .arg(&export)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 template(s)"));
    other
        .cmd()
        .args(["template", "ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("release"));
    env.cmd().args(["template", "use", "nope"]).assert().code(2);
}

#[test]
fn edit_by_filter_is_one_undo_step() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Old one +old");
    add_json(&env, "Old two +old");
    add_json(&env, "Keep me");
    env.cmd()
        .args(["edit", "--filter", "+old", "+archive"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes"));
    env.cmd()
        .args([
            "edit", "--filter", "+old", "--yes", "--", "-+old", "+archive", "!high",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 2 tasks"));
    let out = env
        .cmd()
        .args(["--json", "ls", "+archive"])
        .assert()
        .success();
    let tasks = json(&out.get_output().stdout);
    assert_eq!(tasks.as_array().unwrap().len(), 2);
    assert_eq!(tasks[0]["tags"][0], "archive");
    assert_eq!(tasks[0]["priority"], 2);
    env.cmd().arg("undo").assert().success();
    let out = env.cmd().args(["--json", "ls", "+old"]).assert().success();
    assert_eq!(json(&out.get_output().stdout).as_array().unwrap().len(), 2);
    env.cmd()
        .args(["edit", "--filter", "+old", "-y"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to change"));
    env.cmd()
        .args(["edit", "--filter", "+old", "--notes", "x", "+y"])
        .assert()
        .failure();
    env.cmd()
        .args(["edit", "word", "+tag"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a task id"));
}

#[test]
fn dup_copies_and_applies_changes() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Weekly report #work ~1h due:fri");
    let out = env
        .cmd()
        .args(["--json", "dup", "1", "due:+8d", "!high"])
        .assert()
        .success();
    let copies = json(&out.get_output().stdout);
    assert_eq!(copies[0]["id"], 2);
    assert_eq!(copies[0]["title"], "Weekly report");
    assert_eq!(copies[0]["project"], "work");
    assert_eq!(copies[0]["estimate_min"], 60);
    assert_eq!(copies[0]["priority"], 2);
    env.cmd()
        .args(["duplicate", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Copied as"));
}

#[test]
fn timer_log_ls_edit_rm() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Write docs ~2h");
    let out = env
        .cmd()
        .args(["--json", "timer", "log", "1", "45m"])
        .assert()
        .success();
    let entry = json(&out.get_output().stdout);
    let id = entry["id"].as_i64().unwrap().to_string();
    let ms = |e: &Value| e["ended_at"].as_i64().unwrap() - e["started_at"].as_i64().unwrap();
    assert_eq!(ms(&entry), 45 * 60_000);
    env.cmd()
        .args(["timer", "log", "1", "1h", "tom@9am"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("future"));
    env.cmd()
        .args(["timer", "ls", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(45m)"))
        .stdout(predicate::str::contains("1 entry, 45m in total"));
    env.cmd()
        .args(["timer", "ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Write docs"));
    let out = env
        .cmd()
        .args(["--json", "timer", "edit", &id, "30m"])
        .assert()
        .success();
    // A new length keeps the start.
    let edited = json(&out.get_output().stdout);
    assert_eq!(ms(&edited), 30 * 60_000);
    assert_eq!(edited["started_at"], entry["started_at"]);
    env.cmd()
        .args(["timer", "edit", &id, "1h"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("future"));
    env.cmd()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Time log"))
        .stdout(predicate::str::contains("(30m)"));
    env.cmd().args(["timer", "rm", &id]).assert().success();
    env.cmd()
        .args(["timer", "ls", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No time logged"));
    env.cmd().arg("undo").assert().success();
    env.cmd()
        .args(["timer", "ls", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(30m)"));
    env.cmd()
        .args(["timer", "edit", "999", "5m"])
        .assert()
        .code(2);
}

#[test]
fn ls_reports_planned_time_on_stderr() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "A ~1h due:today");
    add_json(&env, "B ~30m due:today");
    env.cmd()
        .args(["ls", "today"])
        .assert()
        .success()
        .stdout(predicate::str::contains("estimated").not())
        .stderr(predicate::str::contains("2 open tasks, ~1h30m estimated"));
}

#[test]
fn report_shows_estimates_vs_tracked() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Task ~1h");
    env.cmd()
        .args(["timer", "log", "1", "1h30m"])
        .assert()
        .success();
    env.cmd().args(["done", "1"]).assert().success();
    env.cmd()
        .args(["report", "--weeks", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Estimates vs tracked"))
        .stdout(predicate::str::contains("50% over"));
}

#[test]
fn remind_snooze_by_id_and_recent() {
    let env = Env::new();
    init_plain(&env);
    add_json(&env, "Call back");
    env.cmd()
        .args(["remind", "snooze", "1", "2h"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Snoozed 1: Call back until"));
    let out = env.cmd().args(["--json", "remind"]).assert().success();
    assert_eq!(json(&out.get_output().stdout).as_array().unwrap().len(), 1);
    env.cmd().arg("undo").assert().success();
    let out = env.cmd().args(["--json", "remind"]).assert().success();
    assert!(
        json(&out.get_output().stdout)
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Nothing fired recently, so a bare snooze has nothing to act on.
    env.cmd()
        .args(["remind", "snooze"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no reminder fired"));
    env.cmd()
        .args(["remind", "snooze", "1", "whenever"])
        .assert()
        .failure();
}

#[test]
fn capture_reads_and_empties_the_inbox_file() {
    let env = Env::new();
    init_plain(&env);
    let inbox = env.dir.path().join("inbox.txt");
    fs::write(
        env.dir.path().join("config.toml"),
        format!(
            "[capture]\nfile = '{}'\n",
            inbox.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    env.cmd()
        .arg("capture")
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to capture"));
    fs::write(&inbox, "- Buy milk +shop\nCall Bob due:tom\n").unwrap();
    env.cmd()
        .arg("capture")
        .assert()
        .success()
        .stdout(predicate::str::contains("Captured"))
        .stdout(predicate::str::contains("Buy milk"));
    assert_eq!(fs::read_to_string(&inbox).unwrap(), "");
    let out = env.cmd().args(["--json", "ls", "inbox"]).assert().success();
    assert_eq!(json(&out.get_output().stdout).as_array().unwrap().len(), 2);
    // The scheduled reminder run picks the inbox up too.
    fs::write(&inbox, "Third one\n").unwrap();
    env.cmd()
        .args(["remind", "--run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Third one"));
    // An explicit file overrides the config.
    let other = env.dir.path().join("other.txt");
    fs::write(&other, "From elsewhere\n").unwrap();
    env.cmd()
        .arg("capture")
        .arg(&other)
        .assert()
        .success()
        .stdout(predicate::str::contains("From elsewhere"));
}

#[test]
fn capture_without_a_file_explains_the_config() {
    let env = Env::new();
    init_plain(&env);
    env.cmd()
        .arg("capture")
        .assert()
        .failure()
        .stderr(predicate::str::contains("[capture]"));
}
