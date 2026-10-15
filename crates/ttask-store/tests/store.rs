use std::fs;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use tempfile::TempDir;
use ttask_core::{NewTask, Status};
use ttask_store::{
    DbMode, FileState, PasswordSource, SCHEMA_VERSION, Store, StoreError, TaskFilter, detect,
};

const PASSWORD: &str = "correct horse battery";
const NOW: i64 = 1_760_000_000_000;

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

/// Runs `test` once against a plain and once against an encrypted database.
fn both_modes(test: impl Fn(&Path, DbMode)) {
    for mode in [DbMode::Plain, DbMode::Encrypted] {
        let dir = TempDir::new().unwrap();
        test(&db_path(&dir), mode);
    }
}

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
        assert_eq!(reopened.schema_version().unwrap(), SCHEMA_VERSION);
    });
}

#[test]
fn plain_file_has_sqlite_header_and_encrypted_file_does_not() {
    both_modes(|path, mode| {
        drop(create(path, mode));
        let bytes = fs::read(path).unwrap();
        let has_header = bytes.starts_with(b"SQLite format 3\0");
        assert_eq!(has_header, mode == DbMode::Plain, "{mode:?}");
    });
}

#[test]
fn add_list_and_complete_round_trip() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        let first = store
            .insert_task(&NewTask::titled("Pay rent"), NOW)
            .unwrap();
        let second = store
            .insert_task(&NewTask::titled("Call plumber"), NOW + 1)
            .unwrap();
        assert_eq!(first.status, Status::Open);
        assert_eq!(first.created_at, NOW);
        assert_ne!(first.uid, second.uid);

        let done = store.complete_task(first.id, NOW + 10).unwrap();
        assert_eq!(done.status, Status::Done);
        assert_eq!(done.completed_at, Some(NOW + 10));
        drop(store);

        let store = open(path).unwrap();
        let open_titles: Vec<_> = store
            .list_tasks(TaskFilter::Open)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(open_titles, ["Call plumber"]);
        assert_eq!(store.list_tasks(TaskFilter::All).unwrap().len(), 2);
    });
}

#[test]
fn completing_twice_keeps_first_completion_time() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        let task = store.insert_task(&NewTask::titled("Once"), NOW).unwrap();
        store.complete_task(task.id, NOW + 1).unwrap();
        let again = store.complete_task(task.id, NOW + 2).unwrap();
        assert_eq!(again.completed_at, Some(NOW + 1));
    });
}

#[test]
fn completing_missing_task_reports_not_found() {
    both_modes(|path, mode| {
        let mut store = create(path, mode);
        let err = store.complete_task(999, NOW).unwrap_err();
        assert!(matches!(err, StoreError::TaskNotFound(999)), "{err:?}");
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

/// Answers with a list of passwords in order, allowing one attempt each.
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

    let err = Store::open(&path, &mut Attempts(vec!["typo", "typo2", "typo3"])).unwrap_err();
    assert!(matches!(err, StoreError::WrongPassword), "{err:?}");
}

#[test]
fn encrypted_file_contains_no_plaintext_task_data() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let marker = "SECRET-MARKER-7f3a";
    let mut store = create(&path, DbMode::Encrypted);
    let mut task = NewTask::titled(marker);
    task.notes = format!("notes {marker}");
    store.insert_task(&task, NOW).unwrap();
    drop(store); // closing the last connection checkpoints the WAL

    for entry in fs::read_dir(dir.path()).unwrap() {
        let file = entry.unwrap().path();
        let bytes = fs::read(&file).unwrap();
        let found = bytes.windows(marker.len()).any(|w| w == marker.as_bytes());
        assert!(!found, "plaintext found in {}", file.display());
    }
}

#[test]
fn newer_schema_is_refused() {
    both_modes(|path, mode| {
        drop(create(path, mode));
        {
            let password = (mode == DbMode::Encrypted).then_some(PASSWORD);
            let conn = raw_conn(path, password);
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
        }
        let err = open(path).unwrap_err();
        assert!(
            matches!(err, StoreError::NewerSchema { found, .. } if found == SCHEMA_VERSION + 1),
            "{err:?}"
        );
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

/// Raw SQLite access for tests that need to tamper with a file.
fn raw_conn(path: &Path, password: Option<&str>) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).unwrap();
    if let Some(password) = password {
        conn.pragma_update(None, "key", password).unwrap();
    }
    conn
}
