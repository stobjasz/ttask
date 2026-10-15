use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use ttask_app::App;
use ttask_core::jiff::civil::date;
use ttask_core::{FixedClock, Status};
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
        unlock: Some(Box::new(|p| {
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
