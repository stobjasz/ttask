use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use ttask_app::App;
use ttask_core::jiff::civil::date;
use ttask_core::{FixedClock, Status, TaskId};
use ttask_store::Store;

use super::*;

fn tui(dir: &TempDir) -> Tui {
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    Tui::new(app, Settings::default()).unwrap()
}

fn press(tui: &mut Tui, keys: &str) {
    for c in keys.chars() {
        let mods = if c.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        tui.handle_key(KeyEvent::new(KeyCode::Char(c), mods));
    }
}

fn key(tui: &mut Tui, code: KeyCode) {
    tui.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn ctrl(tui: &mut Tui, c: char) {
    tui.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn add(tui: &mut Tui, text: &str) {
    press(tui, "a");
    press(tui, text);
    key(tui, KeyCode::Enter);
    key(tui, KeyCode::Esc);
}

fn titles(tui: &Tui) -> Vec<String> {
    tui.tasks.iter().map(|(t, _)| t.title.clone()).collect()
}

#[test]
fn starts_on_today_and_renders_empty_state() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    assert_eq!(tui.current.name, "Today");
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("ttask"));
    assert!(screen.contains("Nothing here"));
    assert!(screen.contains("Inbox"));
    assert!(
        screen.contains("/ filter"),
        "footer shows the / key: {screen}"
    );
}

#[test]
fn quick_add_with_live_preview_then_complete_and_undo() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    press(&mut tui, "a");
    press(&mut tui, "Pay rent #home !high due:today");
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(
        screen.contains("→ Pay rent · #home · high · due 2026-10-15"),
        "{screen}"
    );
    key(&mut tui, KeyCode::Enter);
    assert!(
        matches!(tui.mode, Mode::Prompt(_)),
        "stays open for the next task"
    );
    key(&mut tui, KeyCode::Esc);
    assert_eq!(titles(&tui), ["Pay rent"]);

    press(&mut tui, "x");
    assert!(titles(&tui).is_empty(), "done tasks leave Today");
    press(&mut tui, "u");
    assert_eq!(titles(&tui), ["Pay rent"]);
    assert_eq!(tui.tasks[0].0.status, Status::Open);
}

#[test]
fn bad_quick_add_shows_error_and_keeps_text() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    press(&mut tui, "a");
    press(&mut tui, "x due:someday");
    key(&mut tui, KeyCode::Enter);
    let Mode::Prompt(prompt) = &tui.mode else {
        panic!("prompt should stay open");
    };
    assert!(prompt.error.as_deref().unwrap().contains("due:someday"));
    assert_eq!(prompt.input.text(), "x due:someday");
}

#[test]
fn sidebar_navigation_switches_views() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Inbox thing");
    add(&mut tui, "Work thing #work due:+3d");
    key(&mut tui, KeyCode::Tab); // detail or wraps; go to sidebar explicitly
    tui.focus = Focus::Sidebar;
    press(&mut tui, "k");
    assert_eq!(tui.current.name, "Inbox");
    assert_eq!(titles(&tui), ["Inbox thing"]);
    press(&mut tui, "G");
    assert_eq!(tui.current.name, "work");
    assert_eq!(titles(&tui), ["Work thing"]);
    // Quick-add inside a project view lands in that project.
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.focus, Focus::List);
    add(&mut tui, "Another");
    assert_eq!(titles(&tui).len(), 2);
}

#[test]
fn filter_prompt_filters_live_and_esc_restores() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Alpha due:today +a");
    add(&mut tui, "Beta due:today +b");
    press(&mut tui, "/");
    press(&mut tui, "+a");
    assert_eq!(titles(&tui), ["Alpha"]);
    key(&mut tui, KeyCode::Esc);
    assert_eq!(titles(&tui).len(), 2);
    press(&mut tui, "/");
    press(&mut tui, "+b");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(titles(&tui), ["Beta"]);
    key(&mut tui, KeyCode::Esc);
    assert_eq!(
        titles(&tui).len(),
        2,
        "Esc in normal mode clears the filter"
    );
}

#[test]
fn priority_due_tags_move_prompts() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Task due:today");
    press(&mut tui, "3");
    assert_eq!(tui.tasks[0].0.priority.value(), 2);
    press(&mut tui, "t");
    press(&mut tui, "+x y");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.tasks[0].0.tags, ["x", "y"]);
    press(&mut tui, "m");
    press(&mut tui, "work/acme");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.tasks[0].0.project.as_deref(), Some("work/acme"));
    press(&mut tui, "d");
    ctrl(&mut tui, 'u');
    press(&mut tui, "yesterday");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.tasks[0].0.due_date, Some(date(2026, 10, 14)));
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("yesterday"));
}

#[test]
fn multi_select_completes_marked_tasks() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "One due:today");
    add(&mut tui, "Two due:today");
    add(&mut tui, "Three due:today");
    press(&mut tui, "g");
    press(&mut tui, "vv");
    assert_eq!(tui.marked.len(), 2);
    press(&mut tui, "x");
    assert_eq!(titles(&tui), ["Three"]);
}

#[test]
fn delete_needs_confirmation() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Doomed due:today");
    press(&mut tui, "D");
    assert!(matches!(tui.mode, Mode::Confirm { .. }));
    press(&mut tui, "n");
    assert_eq!(titles(&tui).len(), 1);
    press(&mut tui, "D");
    press(&mut tui, "y");
    assert!(titles(&tui).is_empty());
}

#[test]
fn checklist_and_subtasks_in_detail_pane() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Trip due:today");
    press(&mut tui, "c");
    press(&mut tui, "passport");
    key(&mut tui, KeyCode::Enter);
    press(&mut tui, "tickets");
    key(&mut tui, KeyCode::Enter);
    key(&mut tui, KeyCode::Esc);
    press(&mut tui, "A");
    press(&mut tui, "Book hotel");
    key(&mut tui, KeyCode::Enter);
    key(&mut tui, KeyCode::Esc);
    let detail = tui.detail.as_ref().unwrap();
    assert_eq!(detail.checklist.len(), 2);
    assert_eq!(detail.subtasks.len(), 1);

    tui.focus = Focus::Detail;
    key(&mut tui, KeyCode::Char(' '));
    assert!(tui.detail.as_ref().unwrap().checklist[0].checked);
    press(&mut tui, "jj");
    key(&mut tui, KeyCode::Char(' '));
    assert!(!tui.detail.as_ref().unwrap().subtasks[0].is_open());
    let screen = render_to_string(&mut tui, 130, 30);
    assert!(screen.contains("Checklist 1/2"), "{screen}");
    assert!(screen.contains("Subtasks 1/1"));
}

#[test]
fn palette_opens_views_and_runs_actions() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Later due:+5d");
    press(&mut tui, ":");
    press(&mut tui, "upcoming");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.current.name, "Upcoming");
    assert_eq!(titles(&tui), ["Later"]);
    press(&mut tui, ":");
    press(&mut tui, "help");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Help { .. }));
    let screen = render_to_string(&mut tui, 120, 40);
    assert!(screen.contains("Quick-add"));
}

#[test]
fn search_finds_closed_tasks_and_esc_returns() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Invoice ACME due:today");
    press(&mut tui, "x");
    ctrl(&mut tui, 'f');
    press(&mut tui, "invo");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(titles(&tui), ["Invoice ACME"]);
    key(&mut tui, KeyCode::Esc);
    assert_eq!(tui.current.name, "Today");
}

#[test]
fn edit_title_and_redo() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Draft due:today");
    press(&mut tui, "e");
    ctrl(&mut tui, 'u');
    press(&mut tui, "Final");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(titles(&tui), ["Final"]);
    press(&mut tui, "u");
    assert_eq!(titles(&tui), ["Draft"]);
    ctrl(&mut tui, 'r');
    assert_eq!(titles(&tui), ["Final"]);
}

#[test]
fn reorder_only_in_manual_views() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    tui.open_view(ttask_app::View::inbox());
    add(&mut tui, "A");
    add(&mut tui, "B");
    press(&mut tui, "G");
    press(&mut tui, "K");
    assert_eq!(titles(&tui), ["B", "A"]);
    tui.open_view(ttask_app::View::all());
    press(&mut tui, "K");
    assert!(tui.status.as_ref().unwrap().error);
}

#[test]
fn idle_lock_hides_data_until_password() {
    let dir = TempDir::new().unwrap();
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let settings = Settings {
        encrypted: true,
        idle_lock: Some(std::time::Duration::from_millis(0)),
        unlock: Some(Box::new(|_, p| {
            secrecy::ExposeSecret::expose_secret(p) == "open sesame"
        })),
        ..Settings::default()
    };
    let mut tui = Tui::new(app, settings).unwrap();
    tui.app.add("Secret due:today", None, None).unwrap();
    tui.tick(std::time::Instant::now());
    assert!(matches!(tui.mode, Mode::Locked { .. }));
    let screen = render_to_string(&mut tui, 100, 20);
    assert!(!screen.contains("Secret"));
    press(&mut tui, "nope");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Locked { error: Some(_), .. }));
    press(&mut tui, "open sesame");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(titles(&tui), ["Secret"]);
}

#[test]
fn narrow_terminal_hides_detail_and_enter_toggles_full_view() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Narrow task due:today");
    tui.app.set_notes(tui.tasks[0].0.id, "the notes").unwrap();
    tui.reload();
    let screen = render_to_string(&mut tui, 80, 20);
    assert!(!screen.contains("the notes"));
    key(&mut tui, KeyCode::Enter);
    let screen = render_to_string(&mut tui, 80, 20);
    assert!(screen.contains("the notes"), "{screen}");
    key(&mut tui, KeyCode::Esc);
    assert!(!tui.detail_full);
}

#[test]
fn ascii_theme_uses_ascii_glyphs() {
    let dir = TempDir::new().unwrap();
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let settings = Settings {
        theme: ThemeName::Ascii,
        ..Settings::default()
    };
    let mut tui = Tui::new(app, settings).unwrap();
    add(&mut tui, "Plain due:today every:day");
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("( )"));
    assert!(screen.contains("Plain ~"));
}

#[test]
fn wait_key_parks_tasks_and_waiting_view_shows_them() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Follow up due:today");
    add(&mut tui, "Keep due:today");
    press(&mut tui, "g");
    press(&mut tui, "w");
    press(&mut tui, "fri");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(titles(&tui), ["Keep"]);
    tui.open_view(ttask_app::View::waiting());
    assert_eq!(titles(&tui), ["Follow up"]);
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("Waiting    until 2026-10-16"), "{screen}");

    press(&mut tui, "w");
    ctrl(&mut tui, 'u');
    press(&mut tui, "someday");
    key(&mut tui, KeyCode::Enter);
    tui.open_view(ttask_app::View::someday());
    assert_eq!(titles(&tui), ["Follow up"]);
    press(&mut tui, "w");
    ctrl(&mut tui, 'u');
    key(&mut tui, KeyCode::Enter);
    tui.open_view(ttask_app::View::today());
    assert_eq!(titles(&tui).len(), 2, "empty wait brings it back");
}

#[test]
fn context_prompt_with_completion_and_palette() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Home thing due:today #home");
    add(&mut tui, "Work thing due:today #work");
    tui.app.save_filter("work", "#work").unwrap();
    tui.reload();

    press(&mut tui, "C");
    press(&mut tui, "wo");
    key(&mut tui, KeyCode::Tab);
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.context.as_deref(), Some("work"));
    assert_eq!(titles(&tui), ["Work thing"]);
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("@work"), "{screen}");

    // New tasks land in the context's project.
    add(&mut tui, "Another due:today");
    assert_eq!(titles(&tui).len(), 2);

    press(&mut tui, ":");
    press(&mut tui, "context none");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.context, None);
    assert_eq!(titles(&tui).len(), 3);
}

#[test]
fn next_view_and_urgency_in_detail() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Low");
    add(&mut tui, "Important !urgent +next");
    tui.open_view(ttask_app::View::next());
    assert_eq!(titles(&tui), ["Important", "Low"]);
    let screen = render_to_string(&mut tui, 130, 20);
    assert!(screen.contains("urgency 24.8"), "{screen}");
}

#[test]
fn search_shows_results_while_typing_and_esc_restores_the_view() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Invoice ACME due:today");
    add(&mut tui, "Call mum due:today");
    press(&mut tui, "f");
    press(&mut tui, "acme");
    assert!(matches!(tui.mode, Mode::Prompt(_)), "still typing");
    assert_eq!(tui.current.kind, ttask_app::ViewKind::Search);
    assert_eq!(titles(&tui), ["Invoice ACME"]);
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("Search: acme"), "{screen}");
    key(&mut tui, KeyCode::Esc);
    assert_eq!(tui.current.name, "Today");
    assert_eq!(titles(&tui).len(), 2);

    // Enter keeps the results and focuses the list.
    press(&mut tui, "f");
    press(&mut tui, "mum");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(titles(&tui), ["Call mum"]);
    assert_eq!(tui.focus, Focus::List);
}

#[test]
fn footer_hints_fit_the_minimum_terminal() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    let screen = render_to_string(&mut tui, 80, 24);
    let footer = screen.lines().last().unwrap();
    assert!(footer.contains("f search"), "{footer}");
    assert!(footer.contains("q quit"), "{footer}");
}

#[test]
fn clipboard_payload_is_base64() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64("Zażółć".as_bytes()), "WmHFvMOzxYLEhw==");
}

/// Settings whose opener opens plain or encrypted files with a fixed clock.
fn opener_settings() -> Settings {
    Settings {
        open_db: Some(Box::new(|path, password| {
            let mut none = secrecy::SecretString::from(String::new());
            let store = match password {
                Some(p) => Store::open(path, &mut p.clone())?,
                None => Store::open(path, &mut none)?,
            };
            Ok(App::new(store, FixedClock::on(date(2026, 10, 15))))
        })),
        unlock: Some(Box::new(|_, _| true)),
        ..Settings::default()
    }
}

fn type_path(tui: &mut Tui, path: &std::path::Path) {
    press(tui, "O");
    ctrl(tui, 'u');
    for c in path.display().to_string().chars() {
        tui.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn prompt(tui: &Tui) -> &crate::state::Prompt {
    match &tui.mode {
        Mode::Prompt(p) => p,
        _ => panic!("a prompt is open"),
    }
}

#[test]
fn open_another_database_checks_the_file_first() {
    let dir = TempDir::new().unwrap();
    let other = dir.path().join("other.db");
    {
        let store = Store::create(&other, None).unwrap();
        let mut app = App::new(store, FixedClock::on(date(2026, 10, 15)));
        app.add("From the other file due:today", None, None)
            .unwrap();
    }
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "not a database").unwrap();

    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, opener_settings()).unwrap();
    add(&mut tui, "Mine due:today");

    type_path(&mut tui, &notes);
    key(&mut tui, KeyCode::Enter);
    let error = prompt(&tui).error.clone().unwrap_or_default();
    assert!(error.contains("not a ttask database"), "{error}");
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("not a ttask database"), "{screen}");
    key(&mut tui, KeyCode::Esc);
    assert_eq!(titles(&tui), ["Mine"]);

    type_path(&mut tui, &other);
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(titles(&tui), ["From the other file"]);
    assert_eq!(tui.app.store().path(), other);
    assert!(tui.status.as_ref().unwrap().text.starts_with("Opened"));
}

#[test]
fn open_an_encrypted_database_asks_for_its_password() {
    let dir = TempDir::new().unwrap();
    let secret = dir.path().join("secret.db");
    let password = secrecy::SecretString::from("correct horse battery".to_owned());
    {
        let store = Store::create(&secret, Some(&password)).unwrap();
        let mut app = App::new(store, FixedClock::on(date(2026, 10, 15)));
        app.add("Hidden due:today", None, None).unwrap();
    }
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, opener_settings()).unwrap();

    type_path(&mut tui, &secret);
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(
        prompt(&tui).kind,
        crate::state::PromptKind::Password(_)
    ));
    press(&mut tui, "wrong");
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("Password: *****"), "{screen}");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(prompt(&tui).error.as_deref(), Some("Wrong password"));
    press(&mut tui, "correct horse battery");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(titles(&tui), ["Hidden"]);
    assert!(tui.settings.encrypted);
}

#[test]
fn tab_completes_paths_in_the_open_prompt() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("work.db"), b"").unwrap();
    std::fs::write(dir.path().join("wiki.db"), b"").unwrap();
    std::fs::create_dir(dir.path().join("archive")).unwrap();
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, opener_settings()).unwrap();

    type_path(&mut tui, &dir.path().join("wo"));
    key(&mut tui, KeyCode::Tab);
    let text = prompt(&tui).input.text();
    assert!(text.ends_with("work.db"), "{text}");

    type_path(&mut tui, &dir.path().join("w"));
    key(&mut tui, KeyCode::Tab);
    assert_eq!(prompt(&tui).hint.as_deref(), Some("wiki.db  work.db"));

    type_path(&mut tui, &dir.path().join("arc"));
    key(&mut tui, KeyCode::Tab);
    let text = prompt(&tui).input.text();
    assert!(
        text.ends_with(&format!("archive{}", std::path::MAIN_SEPARATOR)),
        "{text}"
    );
}

#[test]
fn open_link_opens_one_or_offers_a_choice() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let dir = TempDir::new().unwrap();
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = opened.clone();
    let settings = Settings {
        open_link: Some(Box::new(move |link| {
            sink.borrow_mut().push(link.to_owned());
            Ok(())
        })),
        ..Settings::default()
    };
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, settings).unwrap();
    add(&mut tui, "Read https://example.com/one due:today");
    press(&mut tui, "o");
    assert_eq!(*opened.borrow(), ["https://example.com/one"]);

    let id = tui.tasks[0].0.id;
    tui.app
        .set_notes(id, "also [two](https://example.com/two)")
        .unwrap();
    tui.reload();
    press(&mut tui, "o");
    assert!(matches!(tui.mode, Mode::Palette(_)));
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("Open link"), "{screen}");
    key(&mut tui, KeyCode::Down);
    key(&mut tui, KeyCode::Enter);
    assert_eq!(
        *opened.borrow(),
        ["https://example.com/one", "https://example.com/two"]
    );

    add(&mut tui, "No links here due:today");
    select(&mut tui, "No links here");
    press(&mut tui, "o");
    assert!(tui.status.as_ref().unwrap().error);
}

fn select(tui: &mut Tui, title: &str) -> TaskId {
    let i = tui
        .tasks
        .iter()
        .position(|(t, _)| t.title == title)
        .unwrap_or_else(|| panic!("no task `{title}` in {:?}", titles(tui)));
    tui.task_idx = i;
    tui.reload();
    tui.tasks[i].0.id
}

#[test]
fn dependencies_parent_history_copy_sort_and_lock_keys() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Buy paint due:today");
    add(&mut tui, "Paint the shed due:today");
    let paint = select(&mut tui, "Buy paint");
    let shed = select(&mut tui, "Paint the shed");

    // b: the shed waits on the paint; clearing the prompt removes it again.
    press(&mut tui, "b");
    press(&mut tui, &paint.to_string());
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.app.detail(shed).unwrap().blocked_by[0].id, paint);
    select(&mut tui, "Paint the shed");
    press(&mut tui, "b");
    ctrl(&mut tui, 'u');
    key(&mut tui, KeyCode::Enter);
    assert!(tui.app.detail(shed).unwrap().blocked_by.is_empty());

    // P: make it a subtask, then top level again.
    select(&mut tui, "Paint the shed");
    press(&mut tui, "P");
    press(&mut tui, &paint.to_string());
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.app.task(shed).unwrap().parent_id, Some(paint));
    select(&mut tui, "Paint the shed");
    press(&mut tui, "P");
    ctrl(&mut tui, 'u');
    press(&mut tui, "none");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.app.task(shed).unwrap().parent_id, None);

    // H: history popup.
    select(&mut tui, "Paint the shed");
    press(&mut tui, "H");
    let Mode::History { entries, .. } = &tui.mode else {
        panic!("history popup expected");
    };
    assert!(entries.len() >= 3, "{entries:?}");
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("History: Paint the shed"), "{screen}");
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::Normal));

    // y / Y: copy the title and the id.
    press(&mut tui, "y");
    assert_eq!(
        tui.take_clipboard_request().as_deref(),
        Some("Paint the shed")
    );
    press(&mut tui, "Y");
    assert_eq!(tui.take_clipboard_request(), Some(shed.to_string()));

    // Ctrl-S cycles the sort until another view opens.
    ctrl(&mut tui, 's');
    assert_eq!(tui.sort(), ttask_store::TaskSort::Urgency);
    let screen = render_to_string(&mut tui, 120, 20);
    assert!(screen.contains("by urgency"), "{screen}");
    tui.open_view(ttask_app::View::all());
    assert_eq!(tui.sort_override, None);

    // L only locks encrypted databases.
    press(&mut tui, "L");
    assert!(tui.status.as_ref().unwrap().error);
    assert!(matches!(tui.mode, Mode::Normal));
}

/// Types text into whatever has focus, with `\n` as Enter.
fn type_text(tui: &mut Tui, text: &str) {
    for c in text.chars() {
        let code = if c == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        };
        tui.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
}

#[test]
fn editor_changes_several_fields_in_one_undo_step() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Paint shed #home +diy due:today");
    press(&mut tui, "e");
    assert!(matches!(tui.mode, Mode::Editor(_)));
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("Edit task #1"), "{screen}");
    assert!(screen.contains("+diy"), "{screen}");

    type_text(&mut tui, " white"); // title
    key(&mut tui, KeyCode::Down); // project
    ctrl(&mut tui, 'u');
    type_text(&mut tui, "garden");
    key(&mut tui, KeyCode::Tab); // tags
    type_text(&mut tui, " +weekend");
    key(&mut tui, KeyCode::Down); // priority
    ctrl(&mut tui, 'u');
    type_text(&mut tui, "high");
    key(&mut tui, KeyCode::Down); // due
    ctrl(&mut tui, 'u');
    type_text(&mut tui, "tom 9am");
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("Fri 16 Oct · tomorrow · 09:00"), "{screen}");
    key(&mut tui, KeyCode::BackTab);
    for _ in 0..6 {
        key(&mut tui, KeyCode::Tab);
    }
    // Notes: Enter makes new lines here instead of saving.
    type_text(&mut tui, "Two coats\n- primer first");
    assert!(matches!(tui.mode, Mode::Editor(_)));
    ctrl(&mut tui, 's');
    assert!(matches!(tui.mode, Mode::Normal), "saved");

    let task = tui.app.task(1).unwrap();
    assert_eq!(task.title, "Paint shed white");
    assert_eq!(task.project.as_deref(), Some("garden"));
    assert_eq!(task.tags, ["diy", "weekend"]);
    assert_eq!(task.priority.name(), "high");
    assert_eq!(task.due_date, Some(date(2026, 10, 16)));
    assert_eq!(task.notes, "Two coats\n- primer first");

    press(&mut tui, "u");
    let task = tui.app.task(1).unwrap();
    assert_eq!(task.title, "Paint shed");
    assert_eq!(task.notes, "");
}

#[test]
fn editor_reports_bad_fields_and_guards_unsaved_changes() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Estimate me due:today");
    press(&mut tui, "e");
    for _ in 0..7 {
        key(&mut tui, KeyCode::Tab);
    }
    type_text(&mut tui, "soon");
    key(&mut tui, KeyCode::Up); // away from the bad field
    key(&mut tui, KeyCode::Enter);
    let Mode::Editor(editor) = &tui.mode else {
        panic!("stays open");
    };
    assert!(editor.error.as_deref().unwrap().starts_with("estimate:"));
    assert_eq!(editor.focus, 7, "focus jumps to the bad field");

    ctrl(&mut tui, 'u');
    type_text(&mut tui, "45m");
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::Editor(_)), "first Esc warns");
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(tui.app.task(1).unwrap().estimate_min, None, "discarded");

    // Esc without changes closes at once.
    press(&mut tui, "e");
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::Normal));
}

#[test]
fn weekly_review_walks_through_the_steps() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("t.db");
    {
        let store = Store::create(&path, None).unwrap();
        let mut app = App::new(store, FixedClock::on(date(2026, 9, 1)));
        app.add("Old idea #ideas", None, None).unwrap();
        app.add("Parked #garden wait:someday", None, None).unwrap();
    }
    let store = Store::open(&path, &mut secrecy::SecretString::from(String::new())).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, Settings::default()).unwrap();
    add(&mut tui, "Loose end");
    add(&mut tui, "Late #work due:2026-10-10");

    press(&mut tui, "W");
    assert_eq!(tui.current.name, "Review: Inbox");
    assert_eq!(titles(&tui), ["Loose end"]);
    tui.status = None;
    let screen = render_to_string(&mut tui, 140, 20);
    assert!(screen.contains("Review 1/6"), "{screen}");
    assert!(screen.contains("] next step"), "{screen}");

    press(&mut tui, "]");
    assert_eq!(titles(&tui), ["Late"]);
    press(&mut tui, "]");
    assert_eq!(tui.current.name, "Review: Stale");
    assert_eq!(titles(&tui), ["Old idea"]);
    press(&mut tui, "]");
    assert_eq!(titles(&tui), ["Parked"]);
    press(&mut tui, "]");
    let names: Vec<String> = tui
        .review
        .as_ref()
        .unwrap()
        .projects
        .iter()
        .map(|p| p.path.clone())
        .collect();
    assert_eq!(names, ["garden"], "ideas and work have open tasks");
    let screen = render_to_string(&mut tui, 140, 20);
    assert!(screen.contains("#garden"), "{screen}");

    // `a` adds the next action to the selected project; the list updates.
    press(&mut tui, "a");
    press(&mut tui, "Buy seeds");
    key(&mut tui, KeyCode::Enter);
    key(&mut tui, KeyCode::Esc);
    assert!(tui.review.as_ref().unwrap().projects.is_empty());
    assert_eq!(
        tui.app
            .list("#garden", "", ttask_store::TaskSort::Smart)
            .unwrap()[0]
            .title,
        "Buy seeds"
    );

    press(&mut tui, "]");
    assert_eq!(tui.app.last_review().unwrap(), Some(date(2026, 10, 15)));
    let screen = render_to_string(&mut tui, 140, 20);
    assert!(screen.contains("Review complete"), "{screen}");
    key(&mut tui, KeyCode::Esc);
    assert!(tui.review.is_none());
    assert_eq!(tui.current.name, "Today");

    press(&mut tui, "]");
    assert!(tui.status.as_ref().unwrap().error, "no review running");
}

#[test]
fn agenda_grid_moves_by_day_and_week_and_postpones() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Monday thing due:2026-10-12");
    add(&mut tui, "Standup due:today@09:00");
    add(&mut tui, "Gardening sched:2026-10-17");
    add(&mut tui, "Next week due:2026-10-20");
    add(&mut tui, "Undated");
    tui.open_view(ttask_app::View::agenda());
    let agenda = tui.agenda.as_ref().unwrap();
    assert_eq!(agenda.day, 3, "starts on today, a Thursday");
    assert_eq!(titles(&tui), ["Standup"]);
    tui.status = None;
    let screen = render_to_string(&mut tui, 160, 20);
    assert!(
        screen.contains("Agenda · 12 Oct – 18 Oct 2026 (this week)"),
        "{screen}"
    );
    assert!(screen.contains("Mon 12"), "{screen}");
    assert!(screen.contains("09:00 Standup"), "{screen}");
    assert!(screen.contains("Gardening"), "{screen}");
    assert!(!screen.contains("Undated"), "{screen}");

    press(&mut tui, "hhh");
    assert_eq!(titles(&tui), ["Monday thing"]);
    press(&mut tui, "h");
    assert_eq!(tui.focus, Focus::Sidebar, "left of Monday is the sidebar");
    press(&mut tui, "l");
    press(&mut tui, "lll");
    assert_eq!(titles(&tui), ["Standup"]);

    // `>` moves the task to Friday and the selection follows it.
    press(&mut tui, ">");
    assert_eq!(tui.agenda.as_ref().unwrap().day, 4);
    assert_eq!(titles(&tui), ["Standup"]);
    let task = tui.selected_task().unwrap();
    assert_eq!(task.due_date, Some(date(2026, 10, 16)));
    assert!(task.due_time.is_some(), "time kept");

    // `a` adds on the selected day.
    press(&mut tui, "a");
    match &tui.mode {
        Mode::Prompt(p) => assert_eq!(p.input.text(), "due:2026-10-16 "),
        _ => panic!("add prompt"),
    }
    key(&mut tui, KeyCode::Esc);

    press(&mut tui, "]");
    press(&mut tui, "hhh");
    assert_eq!(titles(&tui), ["Next week"]);
    let screen = render_to_string(&mut tui, 160, 20);
    assert!(screen.contains("(next week)"), "{screen}");
    tui.open_view(ttask_app::View::today());
    assert!(tui.agenda.is_none());
}

#[test]
fn focus_mode_runs_pomodoros_with_the_timer() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;
    let dir = TempDir::new().unwrap();
    let notes: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = notes.clone();
    let settings = Settings {
        notify: Some(Box::new(move |title, _| {
            sink.borrow_mut().push(title.to_owned())
        })),
        ..Settings::default()
    };
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let mut tui = Tui::new(app, settings).unwrap();
    add(&mut tui, "Write report due:today");
    let id = tui.tasks[0].0.id;
    tui.app.checklist_add(id, "outline").unwrap();
    tui.reload();

    press(&mut tui, "F");
    let running = |tui: &Tui| tui.app.running_timer().unwrap().is_some();
    assert!(running(&tui));
    let screen = render_to_string(&mut tui, 100, 30);
    assert!(screen.contains("FOCUS"), "{screen}");
    assert!(screen.contains("Write report"), "{screen}");
    assert!(screen.contains("outline"), "{screen}");

    let started = match &tui.mode {
        Mode::Focus(s) => s.phase_started,
        _ => panic!("focus mode"),
    };
    let min = Duration::from_secs(60);
    tui.tick(started + 25 * min);
    let Mode::Focus(session) = &tui.mode else {
        panic!()
    };
    assert_eq!(session.phase, crate::focus::Phase::ShortBreak);
    assert!(!running(&tui), "breaks stop the time tracker");
    assert_eq!(*notes.borrow(), ["ttask: Short break"]);

    tui.tick(started + 30 * min);
    assert!(running(&tui), "back to work");
    press(&mut tui, " ");
    assert!(!running(&tui), "paused");
    press(&mut tui, " ");
    assert!(running(&tui));

    key(&mut tui, KeyCode::Enter);
    assert!(tui.app.detail(id).unwrap().checklist[0].checked);

    press(&mut tui, "x");
    assert!(matches!(tui.mode, Mode::Normal));
    assert!(!tui.app.task(id).unwrap().is_open());
    assert!(!running(&tui));
}

#[test]
fn templates_are_saved_with_ctrl_t_and_used_from_the_palette() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Weekly report due:today +work");
    let id = tui.tasks[0].0.id;
    tui.app.checklist_add(id, "numbers").unwrap();
    ctrl(&mut tui, 't');
    match &tui.mode {
        Mode::Prompt(p) => assert_eq!(p.input.text(), "weekly report"),
        _ => panic!("name prompt"),
    }
    ctrl(&mut tui, 'u');
    press(&mut tui, "report");
    key(&mut tui, KeyCode::Enter);
    assert_eq!(tui.app.templates().unwrap()[0].name, "report");

    press(&mut tui, ":");
    press(&mut tui, "template: report");
    key(&mut tui, KeyCode::Enter);
    match &tui.mode {
        Mode::Prompt(p) => assert_eq!(p.input.text(), "Weekly report"),
        _ => panic!("use prompt"),
    }
    press(&mut tui, " (Q4)");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    let made = titles(&tui);
    assert!(made.contains(&"Weekly report (Q4)".to_owned()), "{made:?}");
    let new = tui
        .tasks
        .iter()
        .find(|(t, _)| t.title == "Weekly report (Q4)")
        .unwrap()
        .0
        .clone();
    assert_eq!(new.tags, ["work"]);
    assert_eq!(tui.app.detail(new.id).unwrap().checklist[0].text, "numbers");
}

fn click(tui: &mut Tui, x: u16, y: u16) {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    tui.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}

fn wheel(tui: &mut Tui, x: u16, y: u16, down: bool) {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    tui.handle_mouse(MouseEvent {
        kind: if down {
            MouseEventKind::ScrollDown
        } else {
            MouseEventKind::ScrollUp
        },
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}

#[test]
fn mouse_selects_views_and_tasks_and_toggles_checkboxes() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    for i in 1..=8 {
        add(&mut tui, &format!("Task {i} due:today"));
    }
    render_to_string(&mut tui, 120, 30);
    let sidebar = tui.hit.sidebar.unwrap();
    let all = tui
        .sidebar
        .iter()
        .position(
            |e| matches!(e, crate::state::SidebarEntry::View { view, .. } if view.name == "All"),
        )
        .unwrap();
    click(&mut tui, sidebar.x + 2, sidebar.y + all as u16);
    assert_eq!(tui.current.name, "All");

    render_to_string(&mut tui, 120, 30);
    let list = tui.hit.list.unwrap();
    click(&mut tui, list.x + 10, list.y + 2);
    assert_eq!(tui.focus, Focus::List);
    assert_eq!(tui.task_idx, 2);
    let third = tui.selected_task().unwrap().id;

    // The checkbox column toggles done.
    click(&mut tui, list.x + 2, list.y + 2);
    assert!(!tui.app.task(third).unwrap().is_open());

    wheel(&mut tui, list.x + 10, list.y + 1, true);
    assert_eq!(tui.task_idx, 5);
    wheel(&mut tui, list.x + 10, list.y + 1, false);
    assert_eq!(tui.task_idx, 2);

    // Popups scroll; prompts ignore the mouse.
    press(&mut tui, "?");
    wheel(&mut tui, 50, 10, true);
    assert!(matches!(tui.mode, Mode::Help { scroll: 3 }));
    key(&mut tui, KeyCode::Esc);
    press(&mut tui, "a");
    click(&mut tui, sidebar.x + 2, sidebar.y + 1);
    assert!(matches!(tui.mode, Mode::Prompt(_)));
}

#[test]
fn mouse_picks_agenda_cells() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Monday one due:2026-10-12");
    add(&mut tui, "Monday two due:2026-10-12");
    tui.open_view(ttask_app::View::agenda());
    render_to_string(&mut tui, 160, 20);
    let list = tui.hit.list.unwrap();
    // Second task of Monday (row 0 is the day header).
    click(&mut tui, list.x + 2, list.y + 2);
    assert_eq!(tui.agenda.as_ref().unwrap().day, 0);
    assert_eq!(tui.selected_task().unwrap().title, "Monday two");
}

#[test]
fn duplicate_prompt_copies_with_changes() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Report ~1h due:today");
    press(&mut tui, "p");
    assert!(matches!(
        &tui.mode,
        Mode::Prompt(p) if matches!(p.kind, crate::state::PromptKind::Duplicate(_))
    ));
    press(&mut tui, "!high");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(tui.mode, Mode::Normal));
    assert_eq!(titles(&tui), ["Report", "Report"]);
    assert!(tui.status.as_ref().unwrap().text.starts_with("Copied as"));
    assert_eq!(
        tui.selected_task().unwrap().priority.name(),
        "high",
        "the copy is selected"
    );
    press(&mut tui, "u");
    assert_eq!(titles(&tui), ["Report"]);
}

#[test]
fn time_log_popup_logs_edits_and_removes() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Write docs due:today");
    press(&mut tui, "i");
    assert!(matches!(tui.mode, Mode::TimeLog { .. }));
    let screen = render_to_string(&mut tui, 100, 30);
    assert!(screen.contains("Nothing logged yet"), "{screen}");
    press(&mut tui, "a");
    press(&mut tui, "45m");
    key(&mut tui, KeyCode::Enter);
    let Mode::TimeLog { entries, .. } = &tui.mode else {
        panic!("back in the popup");
    };
    assert_eq!(entries.len(), 1);
    let screen = render_to_string(&mut tui, 100, 30);
    assert!(screen.contains("11:15–12:00 (45m)"), "{screen}");
    // Edit: the prompt starts with the entry as it is.
    press(&mut tui, "e");
    let Mode::Prompt(prompt) = &tui.mode else {
        panic!("edit prompt");
    };
    assert_eq!(prompt.input.text(), "45m 2026-10-15@11:15");
    ctrl(&mut tui, 'u');
    press(&mut tui, "30m");
    key(&mut tui, KeyCode::Enter);
    let Mode::TimeLog { entries, .. } = &tui.mode else {
        panic!("back in the popup");
    };
    assert_eq!(entries[0].minutes(tui.app.now_ms()), 30);
    press(&mut tui, "d");
    let Mode::TimeLog { entries, .. } = &tui.mode else {
        panic!("still in the popup");
    };
    assert!(entries.is_empty());
    press(&mut tui, "u");
    let Mode::TimeLog { entries, .. } = &tui.mode else {
        panic!("still in the popup");
    };
    assert_eq!(entries.len(), 1, "undo works inside the popup");
    // A bad spec keeps the prompt open with an error; Esc returns to the popup.
    press(&mut tui, "a");
    press(&mut tui, "soon");
    key(&mut tui, KeyCode::Enter);
    assert!(matches!(&tui.mode, Mode::Prompt(p) if p.error.is_some()));
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::TimeLog { .. }));
    key(&mut tui, KeyCode::Esc);
    assert!(matches!(tui.mode, Mode::Normal));
}

#[test]
fn reminder_banner_offers_snooze() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "Stand-up due:today remind:11:00");
    add(&mut tui, "Other due:today");
    tui.tick(std::time::Instant::now());
    let status = tui.status.clone().unwrap();
    assert!(status.text.contains("z snoozes"), "{}", status.text);
    // The other task is selected, but z snoozes the reminder's task.
    press(&mut tui, "j");
    press(&mut tui, "z");
    let Mode::Prompt(prompt) = &tui.mode else {
        panic!("snooze prompt");
    };
    let crate::state::PromptKind::Snooze(ids) = &prompt.kind else {
        panic!("snooze prompt");
    };
    let stand_up = tui
        .app
        .list("Stand-up", "", ttask_store::TaskSort::Smart)
        .unwrap()[0]
        .id;
    assert_eq!(ids, &[stand_up]);
    assert_eq!(prompt.input.text(), "15m");
    key(&mut tui, KeyCode::Enter);
    assert!(
        tui.status.as_ref().unwrap().text.contains("at 12:15"),
        "{:?}",
        tui.status
    );
    let reminders = tui.app.detail(stand_up).unwrap().reminders;
    assert!(reminders.iter().any(|r| r.fired_at.is_none()));
}

#[test]
fn capture_file_is_read_on_tick() {
    let dir = TempDir::new().unwrap();
    let inbox = dir.path().join("inbox.txt");
    std::fs::write(&inbox, "- Buy milk +shop\n").unwrap();
    let store = Store::create(&dir.path().join("t.db"), None).unwrap();
    let app = App::new(store, FixedClock::on(date(2026, 10, 15)));
    let settings = Settings {
        capture_file: Some(inbox.clone()),
        ..Settings::default()
    };
    let mut tui = Tui::new(app, settings).unwrap();
    tui.tick(std::time::Instant::now());
    assert!(
        tui.status
            .as_ref()
            .unwrap()
            .text
            .contains("Captured 1 task"),
        "{:?}",
        tui.status
    );
    assert_eq!(std::fs::read_to_string(&inbox).unwrap(), "");
    assert_eq!(tui.app.view_count(&ttask_app::View::inbox()).unwrap(), 1);
}

#[test]
fn planned_time_in_header_and_agenda() {
    let dir = TempDir::new().unwrap();
    let mut tui = tui(&dir);
    add(&mut tui, "A ~1h due:today");
    add(&mut tui, "B ~30m due:today");
    let screen = render_to_string(&mut tui, 120, 30);
    assert!(screen.contains("~1h30m planned"), "{screen}");
    tui.open_view(ttask_app::View::agenda());
    let screen = render_to_string(&mut tui, 160, 30);
    assert!(screen.contains("Thu 15 ~1h30m"), "{screen}");
    assert!(!screen.contains("planned"), "{screen}");
}
