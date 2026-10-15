use std::fs;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use tempfile::TempDir;
use ttask_core::filter::parse_filter;
use ttask_core::jiff::civil::{Date, date};
use ttask_core::jiff::tz::TimeZone;
use ttask_core::{NewTask, Priority, Status};
use ttask_store::{
    DbMode, EventKind, FileState, PasswordSource, QueryCtx, SCHEMA_VERSION, Store, StoreError,
    TaskFields, TaskQuery, detect,
};

const PASSWORD: &str = "correct horse battery";
const NOW: i64 = 1_760_000_000_000;
const TODAY: Date = date(2026, 10, 15);

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("ttask.db")
}

fn create(path: &Path, mode: DbMode) -> Store {
    let password = (mode == DbMode::Encrypted).then(|| secret(PASSWORD));
    Store::create(path, password.as_ref()).expect("create store")
}

fn open(path: &Path) -> Result<Store, StoreError> {
    Store::open(path, &mut secret(PASSWORD))
}

fn ctx() -> QueryCtx {
    QueryCtx {
        today: TODAY,
        tz: TimeZone::UTC,
    }
}

/// Runs `test` once against a plain and once against an encrypted database.
fn both_modes(test: impl Fn(&Path, DbMode)) {
    for mode in [DbMode::Plain, DbMode::Encrypted] {
        let dir = TempDir::new().unwrap();
        test(&db_path(&dir), mode);
    }
}

fn add(store: &mut Store, title: &str) -> ttask_core::Task {
    store
        .write(|db| db.insert_task(&NewTask::titled(title), NOW))
        .unwrap()
}

fn titles(store: &Store, filter: &str) -> Vec<String> {
    let expr = parse_filter(filter, TODAY).unwrap();
    store
        .db()
        .query_tasks(&TaskQuery::new(expr), &ctx())
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

// --- opening, encryption ---

#[test]
fn create_detects_and_reopens_in_both_modes() {
    both_modes(|path, mode| {
        let store = create(path, mode);
        assert_eq!(store.mode(), mode);
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        drop(store);

        assert_eq!(detect(path).unwrap(), FileState::Exists(mode));
        let reopened = open(path).unwrap();
        assert_eq!(reopened.mode(), mode);
    });
}

#[test]
fn plain_file_has_sqlite_header_and_encrypted_file_does_not() {
    both_modes(|path, mode| {
        drop(create(path, mode));
        let has_header = fs::read(path).unwrap().starts_with(b"SQLite format 3\0");
        assert_eq!(has_header, mode == DbMode::Plain, "{mode:?}");
    });
}

#[test]
fn create_refuses_to_overwrite_existing_file() {
    both_modes(|path, mode| {
        drop(create(path, mode));
        let err = Store::create(path, None).unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists(_)), "{err:?}");
    });
}

#[test]
fn open_missing_file_reports_missing() {
    let dir = TempDir::new().unwrap();
    let err = open(&db_path(&dir)).unwrap_err();
    assert!(matches!(err, StoreError::Missing(_)), "{err:?}");
}

#[test]
fn wrong_password_is_rejected() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    drop(create(&path, DbMode::Encrypted));
    let err = Store::open(&path, &mut secret("not the password")).unwrap_err();
    assert!(matches!(err, StoreError::WrongPassword), "{err:?}");
}

struct Attempts(Vec<&'static str>);

impl PasswordSource for Attempts {
    fn password(&mut self, attempt: u32) -> ttask_store::Result<SecretString> {
        Ok(secret(self.0[attempt as usize - 1]))
    }

    fn max_attempts(&self) -> u32 {
        self.0.len() as u32
    }
}

#[test]
fn later_attempt_with_right_password_unlocks() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    drop(create(&path, DbMode::Encrypted));
    let store = Store::open(&path, &mut Attempts(vec!["typo", PASSWORD])).unwrap();
    assert_eq!(store.mode(), DbMode::Encrypted);
    let err = Store::open(&path, &mut Attempts(vec!["a", "b", "c"])).unwrap_err();
    assert!(matches!(err, StoreError::WrongPassword), "{err:?}");
}

#[test]
fn encrypted_file_contains_no_plaintext_task_data() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let marker = "SECRET-MARKER-7f3a";
    let mut store = create(&path, DbMode::Encrypted);
    store
        .write(|db| {
            let mut task = NewTask::titled(marker);
            task.notes = format!("notes {marker}");
            task.tags = vec!["secrettag".into()];
            db.insert_task(&task, NOW)
        })
        .unwrap();
    drop(store);
    for entry in fs::read_dir(dir.path()).unwrap() {
        let file = entry.unwrap().path();
        let bytes = fs::read(&file).unwrap();
        for needle in [marker, "secrettag"] {
            let found = bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
            assert!(!found, "plaintext {needle} found in {}", file.display());
        }
    }
}

#[test]
fn newer_schema_is_refused() {
    both_modes(|path, mode| {
        drop(create(path, mode));
        {
            let conn = raw_conn(path, (mode == DbMode::Encrypted).then_some(PASSWORD));
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
        }
        let err = open(path).unwrap_err();
        assert!(matches!(err, StoreError::NewerSchema { .. }), "{err:?}");
    });
}

#[test]
fn garbage_file_is_not_a_database() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    fs::write(&path, b"SQLite format 3\0 but not really a database at all").unwrap();
    let err = open(&path).unwrap_err();
    assert!(matches!(err, StoreError::NotADatabase), "{err:?}");
}

#[test]
fn upgrading_from_v1_migrates_and_keeps_a_backup() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO tasks (uid, title, created_at, updated_at) VALUES ('u1', 'Old task', 1, 1)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let store = open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(titles(&store, ""), ["Old task"]);
    let backup = dir
        .path()
        .join(format!("ttask.db.pre-v{SCHEMA_VERSION}.bak"));
    assert!(backup.exists(), "pre-migration backup missing");
    let old = rusqlite::Connection::open(&backup).unwrap();
    let version: u32 = old
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

// --- tasks, queries ---

#[test]
fn insert_with_all_fields_round_trips() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        let task = store
            .write(|db| {
                let project = db.ensure_project_path(&["work".into(), "clientA".into()], NOW)?;
                let new = NewTask {
                    title: "Ship it".into(),
                    notes: "details".into(),
                    priority: Priority::new(2).unwrap(),
                    project_id: Some(project),
                    due_date: Some(date(2026, 10, 20)),
                    due_time: Some(ttask_core::jiff::civil::time(17, 0, 0, 0)),
                    scheduled_date: Some(date(2026, 10, 18)),
                    estimate_min: Some(90),
                    recur_rule: Some("FREQ=WEEKLY".into()),
                    recur_mode: Some(ttask_core::recur::RecurMode::Due),
                    tags: vec!["b".into(), "A".into()],
                    ..NewTask::default()
                };
                db.insert_task(&new, NOW)
            })
            .unwrap();
        drop(store);
        let store = open(path).unwrap();
        let loaded = store.db().task(task.id).unwrap();
        assert_eq!(loaded, task);
        assert_eq!(loaded.project.as_deref(), Some("work/clientA"));
        assert_eq!(loaded.tags, ["A", "b"]);
        assert_eq!(loaded.due_time.unwrap().hour(), 17);
    });
}

#[test]
fn filters_compile_to_sql() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    store
        .write(|db| {
            let work = db.ensure_project_path(&["work".into()], NOW)?;
            let client = db.ensure_project_path(&["work".into(), "clientA".into()], NOW)?;
            let home = db.ensure_project_path(&["home".into()], NOW)?;
            let mk = |title: &str, project, pri: u8, due: Option<Date>, tags: &[&str]| NewTask {
                title: title.into(),
                project_id: project,
                priority: Priority::new(pri).unwrap(),
                due_date: due,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                ..NewTask::default()
            };
            db.insert_task(
                &mk(
                    "Overdue report",
                    Some(work),
                    1,
                    Some(date(2026, 10, 10)),
                    &[],
                ),
                NOW,
            )?;
            db.insert_task(
                &mk("Client call", Some(client), 3, Some(TODAY), &["phone"]),
                NOW,
            )?;
            db.insert_task(&mk("Fix sink", Some(home), 0, None, &["someday"]), NOW)?;
            db.insert_task(
                &mk("Inbox idea", None, 0, Some(date(2026, 10, 30)), &[]),
                NOW,
            )?;
            Ok(())
        })
        .unwrap();

    assert_eq!(titles(&store, "#work"), ["Overdue report", "Client call"]);
    assert_eq!(titles(&store, "project:work/clienta"), ["Client call"]);
    assert_eq!(titles(&store, "#inbox"), ["Inbox idea"]);
    assert_eq!(titles(&store, "+phone"), ["Client call"]);
    assert_eq!(
        titles(&store, "not +someday and due:none"),
        Vec::<String>::new()
    );
    assert_eq!(
        titles(&store, "due:<=today"),
        ["Overdue report", "Client call"]
    );
    assert_eq!(titles(&store, "is:overdue"), ["Overdue report"]);
    assert_eq!(titles(&store, "!>=2 or #home"), ["Client call", "Fix sink"]);
    assert_eq!(titles(&store, "report"), ["Overdue report"]);
    assert_eq!(titles(&store, "repo"), ["Overdue report"], "prefix match");
    assert_eq!(titles(&store, "created:today"), Vec::<String>::new());
    assert_eq!(
        titles(&store, "\"it's\" or sink"),
        ["Fix sink"],
        "quotes are safe"
    );
}

#[test]
fn full_text_search_ranks_and_snippets() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Encrypted);
    store
        .write(|db| {
            let mut t = NewTask::titled("Pay invoice");
            t.notes = "invoice 42 for ACME".into();
            db.insert_task(&t, NOW)?;
            db.insert_task(&NewTask::titled("Unrelated"), NOW)?;
            Ok(())
        })
        .unwrap();
    let hits = store.db().search("invo", false, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains('['), "{}", hits[0].snippet);
}

#[test]
fn save_task_updates_fields_and_tags() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        let task = add(&mut store, "Draft");
        let saved = store
            .write(|db| {
                let mut f = TaskFields::from(&task);
                f.title = "Final".into();
                f.status = Status::Done;
                f.completed_at = Some(NOW + 5);
                f.tags = vec!["x".into()];
                db.save_task(task.id, &f, NOW + 5)
            })
            .unwrap();
        assert_eq!(saved.title, "Final");
        assert_eq!(saved.status, Status::Done);
        assert_eq!(saved.tags, ["x"]);
        assert_eq!(saved.updated_at, NOW + 5);
    });
}

#[test]
fn subtasks_nest_one_level() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let parent = add(&mut store, "Parent");
    let child = store
        .write(|db| {
            let mut t = NewTask::titled("Child");
            t.parent_id = Some(parent.id);
            db.insert_task(&t, NOW)
        })
        .unwrap();
    assert_eq!(store.db().task(parent.id).unwrap().subtasks.total, 1);
    let err = store
        .write(|db| {
            let mut t = NewTask::titled("Grandchild");
            t.parent_id = Some(child.id);
            db.insert_task(&t, NOW)
        })
        .unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err:?}");

    store
        .write(|db| db.soft_delete_task(parent.id, NOW))
        .unwrap();
    assert!(
        store.db().try_task(child.id).unwrap().is_none(),
        "subtasks go with their parent"
    );
}

#[test]
fn write_rolls_back_on_error() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let result: ttask_store::Result<()> = store.write(|db| {
        db.insert_task(&NewTask::titled("Will vanish"), NOW)?;
        Err(StoreError::Invalid("boom".into()))
    });
    assert!(result.is_err());
    assert!(titles(&store, "status:all").is_empty());
}

// --- projects, tags, checklist, filters ---

#[test]
fn projects_nest_rename_archive_delete() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let (work, client) = store
        .write(|db| {
            let work = db.ensure_project_path(&["Work".into()], NOW)?;
            let client = db.ensure_project_path(&["work".into(), "ClientA".into()], NOW)?;
            assert_eq!(
                db.ensure_project_path(&["WORK".into()], NOW)?,
                work,
                "case-insensitive"
            );
            let mut t = NewTask::titled("Billable");
            t.project_id = Some(client);
            db.insert_task(&t, NOW)?;
            Ok((work, client))
        })
        .unwrap();
    let projects = store.db().projects(false).unwrap();
    let paths: Vec<_> = projects.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(paths, ["Work", "Work/ClientA"]);
    assert_eq!(projects[1].open_tasks, 1);
    assert_eq!(
        store.db().find_project("work/clienta").unwrap().unwrap().id,
        client
    );

    store
        .write(|db| db.rename_project(client, "Acme", NOW))
        .unwrap();
    assert_eq!(titles(&store, "#work/acme"), ["Billable"]);

    store
        .write(|db| db.set_project_archived(work, true, NOW))
        .unwrap();
    assert!(store.db().projects(false).unwrap().is_empty());
    assert_eq!(store.db().projects(true).unwrap().len(), 2);

    let moved = store.write(|db| db.delete_project(work, NOW)).unwrap();
    assert_eq!(moved, 1);
    assert_eq!(titles(&store, "#inbox"), ["Billable"]);
}

#[test]
fn tags_rename_merges_and_delete_detaches() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    store
        .write(|db| {
            let mut a = NewTask::titled("A");
            a.tags = vec!["phone".into()];
            db.insert_task(&a, NOW)?;
            let mut b = NewTask::titled("B");
            b.tags = vec!["call".into(), "phone".into()];
            db.insert_task(&b, NOW)?;
            Ok(())
        })
        .unwrap();
    store.write(|db| db.rename_tag("call", "phone")).unwrap();
    let tags = store.db().tags().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].open_tasks, 2);
    assert!(store.write(|db| db.ensure_tag("9lives")).is_err());
    store.write(|db| db.delete_tag("phone")).unwrap();
    assert!(titles(&store, "+phone").is_empty());
}

#[test]
fn checklist_items_count_toward_progress() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let task = add(&mut store, "Trip");
    store
        .write(|db| {
            let a = db.add_checklist_item(task.id, "passport")?;
            db.add_checklist_item(task.id, "tickets")?;
            db.set_checklist_checked(a.id, true)
        })
        .unwrap();
    let loaded = store.db().task(task.id).unwrap();
    assert_eq!((loaded.checklist.done, loaded.checklist.total), (1, 2));
    let items = store.db().checklist(task.id).unwrap();
    assert_eq!(items[0].text, "passport");
}

#[test]
fn saved_filters_upsert() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    store
        .write(|db| db.save_filter("This week", "due:<=eow"))
        .unwrap();
    store
        .write(|db| db.save_filter("This week", "due:<=+7d"))
        .unwrap();
    let filters = store.db().saved_filters().unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].query, "due:<=+7d");
    store.write(|db| db.delete_filter("This week")).unwrap();
    assert!(store.write(|db| db.delete_filter("This week")).is_err());
}

// --- journal ---

#[test]
fn undo_and_redo_restore_snapshots() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        // Batch 1: create.
        let task = store
            .write(|db| {
                let batch = db.begin_batch()?;
                let mut new = NewTask::titled("Draft");
                new.tags = vec!["t1".into()];
                let task = db.insert_task(&new, NOW)?;
                db.add_checklist_item(task.id, "step")?;
                let after = db.snapshot(task.id)?;
                db.record_event(
                    batch,
                    task.id,
                    EventKind::Created,
                    None,
                    after.as_ref(),
                    NOW,
                )?;
                Ok(task)
            })
            .unwrap();
        // Batch 2: rename + retag.
        store
            .write(|db| {
                let batch = db.begin_batch()?;
                let before = db.snapshot(task.id)?;
                let mut f = TaskFields::from(&db.task(task.id)?);
                f.title = "Final".into();
                f.tags = vec!["t2".into()];
                db.save_task(task.id, &f, NOW + 1)?;
                let after = db.snapshot(task.id)?;
                db.record_event(
                    batch,
                    task.id,
                    EventKind::Updated,
                    before.as_ref(),
                    after.as_ref(),
                    NOW + 1,
                )
            })
            .unwrap();

        let undone = store.write(|db| db.undo()).unwrap().unwrap();
        assert_eq!(undone.titles, ["Final"]);
        let t = store.db().task(task.id).unwrap();
        assert_eq!(
            (t.title.as_str(), t.tags.clone()),
            ("Draft", vec!["t1".to_owned()])
        );
        assert_eq!(t.checklist.total, 1);

        store.write(|db| db.undo()).unwrap().unwrap();
        assert!(
            store.db().try_task(task.id).unwrap().is_none(),
            "creation undone"
        );
        assert!(store.write(|db| db.undo()).unwrap().is_none());

        store.write(|db| db.redo()).unwrap().unwrap();
        let t = store.db().task(task.id).unwrap();
        assert_eq!(t.title, "Draft");
        assert_eq!(t.checklist.total, 1);
        store.write(|db| db.redo()).unwrap().unwrap();
        assert_eq!(store.db().task(task.id).unwrap().title, "Final");
        assert!(store.write(|db| db.redo()).unwrap().is_none());

        let history = store.db().history(task.id).unwrap();
        let kinds: Vec<_> = history.iter().map(|h| h.kind.as_str()).collect();
        assert_eq!(kinds, ["created", "updated"]);
        assert!(history[1].changed.contains(&"title".to_owned()));
        assert!(history[1].changed.contains(&"tags".to_owned()));
    });
}

#[test]
fn new_action_discards_redo_stack() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let task = add(&mut store, "x");
    store
        .write(|db| {
            let batch = db.begin_batch()?;
            let snap = db.snapshot(task.id)?;
            db.record_event(batch, task.id, EventKind::Created, None, snap.as_ref(), NOW)
        })
        .unwrap();
    store.write(|db| db.undo()).unwrap();
    assert_eq!(store.db().undo_state().unwrap(), (false, true));
    store.write(|db| db.begin_batch().map(|_| ())).unwrap();
    assert_eq!(store.db().undo_state().unwrap(), (false, false));
}

// --- extras ---

#[test]
fn dependencies_block_and_refuse_cycles() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let a = add(&mut store, "A");
    let b = add(&mut store, "B");
    let c = add(&mut store, "C");
    store.write(|db| db.add_dependency(a.id, b.id)).unwrap();
    store.write(|db| db.add_dependency(b.id, c.id)).unwrap();
    assert!(store.db().task(b.id).unwrap().blocked);
    assert_eq!(titles(&store, "is:blocked"), ["B", "C"]);
    let err = store.write(|db| db.add_dependency(c.id, a.id)).unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err:?}");
    assert!(store.write(|db| db.add_dependency(a.id, a.id)).is_err());
    store.write(|db| db.remove_dependency(a.id, b.id)).unwrap();
    assert!(!store.db().task(b.id).unwrap().blocked);
}

#[test]
fn reminders_fire_once_for_open_tasks() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let task = add(&mut store, "Call mom");
    let r = store
        .write(|db| db.add_reminder(task.id, NOW + 1000))
        .unwrap();
    assert!(store.db().due_reminders(NOW).unwrap().is_empty());
    assert_eq!(store.db().upcoming_reminders(NOW, 5).unwrap().len(), 1);
    assert_eq!(store.db().due_reminders(NOW + 1000).unwrap().len(), 1);
    store
        .write(|db| db.mark_reminder_fired(r.id, NOW + 1000))
        .unwrap();
    assert!(store.db().due_reminders(NOW + 5000).unwrap().is_empty());
}

#[test]
fn timer_runs_one_entry_at_a_time() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let a = add(&mut store, "A");
    let b = add(&mut store, "B");
    store.write(|db| db.start_timer(a.id, NOW)).unwrap();
    let (stopped, _) = store
        .write(|db| db.start_timer(b.id, NOW + 120_000))
        .unwrap();
    let stopped = stopped.unwrap();
    assert_eq!(stopped.task_id, a.id);
    assert_eq!(stopped.minutes(0), 2);
    assert_eq!(store.db().running_timer().unwrap().unwrap().task_id, b.id);
    store.write(|db| db.stop_timer(NOW + 180_000)).unwrap();
    assert!(store.db().running_timer().unwrap().is_none());
    assert_eq!(store.db().time_entries(b.id).unwrap().len(), 1);
}

// --- maintenance ---

#[test]
fn backup_keeps_encryption_and_contents() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        add(&mut store, "Backed up");
        let backup = path.with_file_name("copy.db");
        store.backup_to(&backup).unwrap();
        assert_eq!(detect(&backup).unwrap(), FileState::Exists(mode));
        let copy = open(&backup).unwrap();
        assert_eq!(copy.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(titles(&copy, ""), ["Backed up"]);
        assert!(store.backup_to(&backup).is_err(), "never overwrites");
    });
}

#[test]
fn convert_encrypts_changes_password_and_decrypts() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let mut store = create(&path, DbMode::Plain);
    add(&mut store, "Survives");

    let bak = store.convert(Some(&secret(PASSWORD))).unwrap();
    assert!(bak.exists());
    assert_eq!(detect(&path).unwrap(), FileState::Exists(DbMode::Encrypted));
    let store = open(&path).unwrap();
    assert_eq!(titles(&store, ""), ["Survives"]);

    let new_password = "another long password";
    store.convert(Some(&secret(new_password))).unwrap();
    assert!(matches!(
        open(&path).unwrap_err(),
        StoreError::WrongPassword
    ));
    let store = Store::open(&path, &mut secret(new_password)).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);

    store.convert(None).unwrap();
    assert_eq!(detect(&path).unwrap(), FileState::Exists(DbMode::Plain));
    let store = Store::open(&path, &mut secret("unused")).unwrap();
    assert_eq!(titles(&store, ""), ["Survives"]);
    assert!(ttask_store::remove_backup_copy(&path).unwrap());
}

#[test]
fn check_reports_healthy_database() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        add(&mut store, "x");
        let report = store.check().unwrap();
        assert!(report.is_ok(), "{:?}", report.problems);
    });
}

#[test]
fn gc_purges_old_soft_deletes() {
    let dir = TempDir::new().unwrap();
    let mut store = create(&db_path(&dir), DbMode::Plain);
    let old = add(&mut store, "Old");
    let recent = add(&mut store, "Recent");
    store.write(|db| db.soft_delete_task(old.id, NOW)).unwrap();
    store
        .write(|db| db.soft_delete_task(recent.id, NOW + 10))
        .unwrap();
    let report = store.gc(NOW + 5, 0).unwrap();
    assert_eq!(report.purged_tasks, 1);
    assert!(store.db().snapshot(old.id).unwrap().is_none());
    assert!(store.db().snapshot(recent.id).unwrap().is_some());
}

#[test]
fn data_version_is_readable() {
    let dir = TempDir::new().unwrap();
    let store = create(&db_path(&dir), DbMode::Plain);
    assert!(store.data_version().unwrap() >= 0);
}

/// Raw SQLite access for tests that need to tamper with a file.
fn raw_conn(path: &Path, password: Option<&str>) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).unwrap();
    if let Some(password) = password {
        conn.pragma_update(None, "key", password).unwrap();
    }
    conn
}
