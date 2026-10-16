use jiff::civil::{Date, date};
use tempfile::TempDir;
use ttask_core::{FixedClock, Priority, Status};
use ttask_store::{Store, TaskSort};

use super::*;

const TODAY: Date = date(2026, 10, 15); // Thursday

fn app(dir: &TempDir) -> App {
    app_on(dir, TODAY)
}

fn app_on(dir: &TempDir, today: Date) -> App {
    let path = dir.path().join("t.db");
    let store = if path.exists() {
        Store::open(&path, &mut secrecy::SecretString::from("x".to_owned())).unwrap()
    } else {
        Store::create(&path, None).unwrap()
    };
    App::new(store, FixedClock::on(today))
}

fn titles(app: &App, query: &str) -> Vec<String> {
    app.list(query, "", TaskSort::Smart)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

#[test]
fn quick_add_sets_every_field() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app
        .add(
            "Renew passport #admin +gov !high due:fri@17:00 sched:mon ~30m",
            None,
            None,
        )
        .unwrap();
    assert_eq!(task.title, "Renew passport");
    assert_eq!(task.project.as_deref(), Some("admin"));
    assert_eq!(task.tags, ["gov"]);
    assert_eq!(task.priority, Priority::new(2).unwrap());
    assert_eq!(task.due_date, Some(date(2026, 10, 16)));
    assert_eq!(task.due_time.unwrap().hour(), 17);
    assert_eq!(task.scheduled_date, Some(date(2026, 10, 19)));
    assert_eq!(task.estimate_min, Some(30));
}

#[test]
fn add_rejects_blank_title_and_bad_tokens() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    assert!(matches!(
        app.add("   ", None, None),
        Err(AppError::EmptyTitle)
    ));
    assert!(matches!(
        app.add("#work !high", None, None),
        Err(AppError::EmptyTitle)
    ));
    assert!(matches!(
        app.add("x due:someday", None, None),
        Err(AppError::QuickAdd(_))
    ));
    assert!(
        titles(&app, "status:all wait:any").is_empty(),
        "failed adds leave nothing"
    );
}

#[test]
fn subtasks_inherit_parent_project() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let parent = app.add("Trip #travel", None, None).unwrap();
    let child = app.add("Book hotel", None, Some(parent.id)).unwrap();
    assert_eq!(child.parent_id, Some(parent.id));
    assert_eq!(child.project.as_deref(), Some("travel"));
}

#[test]
fn edit_applies_modifications() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app.add("Draft +later #work due:tom", None, None).unwrap();
    let edited = app
        .edit(
            &[task.id],
            "Final report -+later +q4 #none due:none !urgent",
        )
        .unwrap()
        .remove(0);
    assert_eq!(edited.title, "Final report");
    assert_eq!(edited.tags, ["q4"]);
    assert_eq!(edited.project, None);
    assert_eq!(edited.due_date, None);
    assert_eq!(edited.priority, Priority::new(3).unwrap());
    assert!(matches!(
        app.edit(&[task.id], ""),
        Err(AppError::Invalid(_))
    ));
}

#[test]
fn completing_recurring_task_spawns_next_instance() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app
        .add(
            "Weekly review every:week due:2026-10-01 +self remind:due-1h",
            None,
            None,
        )
        .unwrap();
    app.checklist_add(task.id, "inbox zero").unwrap();
    let done = app.complete(&[task.id]).unwrap().remove(0);
    assert_eq!(done.task.status, Status::Done);
    let next = done.next.expect("spawned");
    // Missed occurrences collapse: next Thursday after today.
    assert_eq!(next.due_date, Some(date(2026, 10, 22)));
    assert_eq!(next.tags, ["self"]);
    assert_eq!(next.recur_series.as_deref(), Some(task.uid.as_str()));
    let detail = app.detail(next.id).unwrap();
    assert_eq!(detail.checklist.len(), 1);
    assert!(!detail.checklist[0].checked);
    assert_eq!(detail.reminders.len(), 1);

    // Completing the spawned one keeps the same series.
    let again = app.complete(&[next.id]).unwrap().remove(0).next.unwrap();
    assert_eq!(again.recur_series, next.recur_series);
}

#[test]
fn recurrence_without_due_gets_first_occurrence() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app.add("Gym every:mon,wed", None, None).unwrap();
    assert_eq!(task.due_date, Some(date(2026, 10, 19)));
    let daily = app.add("Vitamins every:day", None, None).unwrap();
    assert_eq!(daily.due_date, Some(TODAY));
}

#[test]
fn count_limited_recurrence_stops() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app
        .add("Twice every:FREQ=DAILY;COUNT=2 due:today", None, None)
        .unwrap();
    let next = app.complete(&[task.id]).unwrap().remove(0).next.unwrap();
    assert!(app.complete(&[next.id]).unwrap().remove(0).next.is_none());
}

#[test]
fn skip_advances_without_completing() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app
        .add("Water plants every!:3d due:today", None, None)
        .unwrap();
    let skipped = app.skip(task.id).unwrap();
    assert_eq!(skipped.status, Status::Open);
    assert_eq!(skipped.due_date, Some(date(2026, 10, 18)));
    let plain = app.add("Plain", None, None).unwrap();
    assert!(matches!(app.skip(plain.id), Err(AppError::NotRecurring(_))));
}

#[test]
fn completing_parent_completes_open_subtasks() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let parent = app.add("Move house", None, None).unwrap();
    let a = app.add("Pack", None, Some(parent.id)).unwrap();
    app.add("Clean", None, Some(parent.id)).unwrap();
    app.complete(&[a.id]).unwrap();
    let done = app.complete(&[parent.id]).unwrap().remove(0);
    assert_eq!(done.subtasks.len(), 1, "only the still-open one");
    assert!(titles(&app, "").is_empty());
}

#[test]
fn undo_reverts_whole_batches() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app.add("Report every:week due:today", None, None).unwrap();
    app.complete(&[task.id]).unwrap();
    assert_eq!(titles(&app, "status:all wait:any").len(), 2);

    // One undo reverts both the completion and the spawned instance.
    let undone = app.undo().unwrap().unwrap();
    assert!(undone.kinds.contains(&"spawned".to_owned()));
    let all = app
        .list("status:all wait:any", "", TaskSort::Smart)
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, Status::Open);

    app.redo().unwrap().unwrap();
    assert_eq!(titles(&app, "status:all wait:any").len(), 2);

    app.undo().unwrap();
    app.undo().unwrap();
    assert!(
        titles(&app, "status:all wait:any").is_empty(),
        "creation undone too"
    );
    assert!(app.undo().unwrap().is_none());
}

#[test]
fn delete_takes_subtasks_and_undo_brings_them_back() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let parent = app.add("Parent", None, None).unwrap();
    app.add("Child", None, Some(parent.id)).unwrap();
    app.delete(&[parent.id]).unwrap();
    assert!(titles(&app, "status:all wait:any").is_empty());
    app.undo().unwrap();
    assert_eq!(titles(&app, "status:all wait:any"), ["Parent", "Child"]);
}

#[test]
fn reorder_swaps_manual_order() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let a = app.add("A #p", None, None).unwrap();
    let b = app.add("B #p", None, None).unwrap();
    let c = app.add("C #p", None, None).unwrap();
    let order = [a.id, b.id, c.id];
    app.reorder(c.id, &order, true).unwrap();
    let manual: Vec<_> = app
        .list("#p", "", TaskSort::Manual)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(manual, ["A", "C", "B"]);
}

#[test]
fn views_select_expected_tasks() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Inbox thing", None, None).unwrap();
    app.add("Due today #work due:today", None, None).unwrap();
    app.add("Overdue #work due:yesterday", None, None).unwrap();
    app.add("Later #work due:+5d", None, None).unwrap();
    app.add("Starts today #work sched:today", None, None)
        .unwrap();
    let done = app.add("Done one #work", None, None).unwrap();
    app.complete(&[done.id]).unwrap();

    let names = |v: View| -> Vec<String> {
        app.view_tasks(&v, "")
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    };
    assert_eq!(names(View::inbox()), ["Inbox thing"]);
    assert_eq!(
        names(View::today()),
        ["Overdue", "Due today", "Starts today"]
    );
    assert_eq!(names(View::upcoming()), ["Later"]);
    assert_eq!(names(View::overdue()), ["Overdue"]);
    assert_eq!(names(View::logbook()), ["Done one"]);
    assert_eq!(app.view_count(&View::all()).unwrap(), 5);
    // Extra filter narrows a view.
    assert_eq!(
        app.view_tasks(&View::today(), "due:today").unwrap().len(),
        1
    );
}

#[test]
fn arrange_tree_puts_children_under_parents() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let p = app.add("P due:+3d", None, None).unwrap();
    app.add("Other due:+1d", None, None).unwrap();
    app.add("Child due:today", None, Some(p.id)).unwrap();
    let tree = arrange_tree(app.list("", "", TaskSort::Smart).unwrap());
    let shape: Vec<_> = tree.iter().map(|(t, d)| (t.title.as_str(), *d)).collect();
    assert_eq!(shape, [("Other", 0), ("P", 0), ("Child", 1)]);
}

#[test]
fn dependencies_and_detail() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let a = app.add("Get quote", None, None).unwrap();
    let b = app.add("Sign contract", None, None).unwrap();
    app.block(b.id, a.id).unwrap();
    let detail = app.detail(b.id).unwrap();
    assert_eq!(detail.blocked_by.len(), 1);
    assert!(detail.task.blocked);
    app.complete(&[a.id]).unwrap();
    assert!(!app.task(b.id).unwrap().blocked);
    app.undo().unwrap();
    app.undo().unwrap();
    assert!(!app.task(b.id).unwrap().blocked, "dependency undone");
}

#[test]
fn reminders_fire_once() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Call remind:today@08:00", None, None).unwrap();
    let fired = app.take_due_reminders().unwrap();
    assert_eq!(fired.len(), 1, "08:00 is before the fixed noon clock");
    assert!(app.take_due_reminders().unwrap().is_empty());
    assert!(matches!(
        app.add("x remind:due-1h", None, None),
        Err(AppError::Store(_))
    ));
}

#[test]
fn rotate_backups_creates_and_prunes() {
    let dir = TempDir::new().unwrap();
    let backups = dir.path().join("backups");
    let policy = BackupPolicy {
        dir: backups.clone(),
        daily: 2,
        weekly: 1,
    };
    for day in [13, 14, 15] {
        let app = app_on(&dir, date(2026, 10, day));
        app.rotate_backups(&policy).unwrap();
    }
    let mut names: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ttask-daily-2026-10-14.db",
            "ttask-daily-2026-10-15.db",
            "ttask-weekly-2026-W42.db"
        ]
    );
    let report = app(&dir).rotate_backups(&policy).unwrap();
    assert!(report.created.is_empty(), "idempotent within a day");
}

fn seed(app: &mut App) {
    let parent = app
        .add(
            "Plan trip #travel/europe +fun !high due:2026-11-01@09:30 every:year ~2h remind:due-1h",
            None,
            None,
        )
        .unwrap();
    app.add("Book flights", None, Some(parent.id)).unwrap();
    app.checklist_add(parent.id, "passport").unwrap();
    let blocker = app.add("Renew passport #admin", None, None).unwrap();
    app.block(parent.id, blocker.id).unwrap();
    let done = app.add("Old chore wait:+2d", None, None).unwrap();
    app.complete(&[done.id]).unwrap();
    app.set_notes(blocker.id, "bring photos\nand form").unwrap();
    app.save_filter("Fun", "+fun").unwrap();
    app.start_timer(blocker.id).unwrap();
    app.stop_timer().unwrap();
}

#[test]
fn json_round_trip_is_lossless_and_idempotent() {
    let src_dir = TempDir::new().unwrap();
    let mut src = app(&src_dir);
    seed(&mut src);
    let doc = src.export().unwrap();
    let json = serde_json::to_string(&doc).unwrap();

    let dst_dir = TempDir::new().unwrap();
    let mut dst = app(&dst_dir);
    let report = dst
        .import(io::parse_json(&json).unwrap().into(), None, false)
        .unwrap();
    assert_eq!(report.tasks_created, 4);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let mut again = dst.export().unwrap();
    again.exported_at = doc.exported_at;
    // History differs only by the import event itself.
    again.history.clear();
    let mut original = doc.clone();
    original.history.clear();
    assert_eq!(again, original);

    let report = dst
        .import(io::parse_json(&json).unwrap().into(), None, false)
        .unwrap();
    assert_eq!(
        (report.tasks_created, report.tasks_updated),
        (0, 4),
        "matched on uid"
    );
    assert_eq!(
        dst.list("status:all wait:any", "", TaskSort::Smart)
            .unwrap()
            .len(),
        4
    );
    let detail = dst
        .detail(dst.list("+fun", "", TaskSort::Smart).unwrap()[0].id)
        .unwrap();
    assert_eq!(detail.subtasks.len(), 1);
    assert_eq!(detail.blocked_by.len(), 1);
    assert_eq!(detail.reminders.len(), 1);
    assert_eq!(detail.checklist.len(), 1);
    assert!(!dst.history(detail.task.id).unwrap().is_empty());
}

#[test]
fn import_dry_run_and_undo() {
    let src_dir = TempDir::new().unwrap();
    let mut src = app(&src_dir);
    seed(&mut src);
    let doc = src.export().unwrap();

    let dst_dir = TempDir::new().unwrap();
    let mut dst = app(&dst_dir);
    let report = dst.import(doc.clone().into(), None, true).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.tasks_created, 4);
    assert!(
        titles(&dst, "status:all wait:any").is_empty(),
        "dry run writes nothing"
    );

    dst.import(doc.into(), None, false).unwrap();
    assert_eq!(titles(&dst, "status:all wait:any").len(), 4);
    dst.undo().unwrap();
    assert!(
        titles(&dst, "status:all wait:any").is_empty(),
        "import is one undo step"
    );
}

#[test]
fn csv_and_todotxt_round_trip() {
    let src_dir = TempDir::new().unwrap();
    let mut src = app(&src_dir);
    seed(&mut src);
    let today = src.today();
    for (text, parse) in [
        (
            io::csv_export(&src).unwrap(),
            Box::new(|t: &str| io::csv_import(t, &[], today))
                as Box<dyn Fn(&str) -> Result<io::ImportBatch>>,
        ),
        (
            io::todotxt_export(&src).unwrap(),
            Box::new(move |t: &str| io::todotxt_import(t, today)),
        ),
    ] {
        let dst_dir = TempDir::new().unwrap();
        let mut dst = app(&dst_dir);
        let report = dst.import(parse(&text).unwrap(), None, false).unwrap();
        assert_eq!(report.tasks_created, 4, "{text}");
        let trip = &dst.list("+fun", "", TaskSort::Smart).unwrap()[0];
        assert_eq!(trip.title, "Plan trip");
        assert_eq!(trip.project.as_deref(), Some("travel/europe"));
        assert_eq!(trip.due_date, Some(date(2026, 11, 1)));
        assert_eq!(trip.due_time.map(|t| t.minute()), Some(30));
        assert_eq!(trip.recur_rule.as_deref(), Some("FREQ=YEARLY"));
        assert_eq!(trip.subtasks.total, 1, "parent link survives");
        // Same uids: importing again updates.
        let again = dst.import(parse(&text).unwrap(), None, false).unwrap();
        assert_eq!(again.tasks_created, 0);
    }
}

#[test]
fn markdown_export_groups_by_project() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    seed(&mut app);
    let md = io::markdown_export(&app).unwrap();
    assert!(md.starts_with("# ttask export"));
    let inbox = md.find("## Inbox").unwrap();
    let travel = md.find("## travel/europe").unwrap();
    assert!(inbox < travel);
    assert!(md.contains("- [ ] Plan trip — due 2026-11-01 09:30 · high · +fun"));
    assert!(md.contains("  - [ ] Book flights"));
    assert!(md.contains("  > bring photos"));
    assert!(md.contains("- [x] Old chore"));
}

#[test]
fn import_into_default_project() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let batch = io::todotxt_import("Loose task\nOther +home\n", TODAY).unwrap();
    app.import(batch, Some("Imported"), false).unwrap();
    assert_eq!(titles(&app, "#Imported"), ["Loose task"]);
    assert_eq!(titles(&app, "#home"), ["Other"]);
}

#[test]
fn urgency_orders_the_next_view() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Plain chore", None, None).unwrap();
    app.add("Overdue bill due:-10d", None, None).unwrap();
    app.add("Flagged +next", None, None).unwrap();
    let blocker = app.add("Get quote", None, None).unwrap();
    let blocked = app.add("Sign contract !urgent", None, None).unwrap();
    app.block(blocked.id, blocker.id).unwrap();
    app.add("Later sched:+5d !urgent", None, None).unwrap();

    let next: Vec<_> = app
        .view_tasks(&View::next(), "")
        .unwrap()
        .into_iter()
        .map(|t| (t.title, t.urgency))
        .collect();
    let titles: Vec<&str> = next.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        ["Flagged", "Overdue bill", "Get quote", "Plain chore"]
    );
    assert!(next.windows(2).all(|w| w[0].1 >= w[1].1), "{next:?}");
    // Blocked and future-scheduled tasks stay out of Next.
    assert!(!titles.contains(&"Sign contract"));
    assert!(!titles.contains(&"Later"));
    assert!(
        app.task(blocker.id).unwrap().urgency >= 8.0,
        "blocking adds urgency"
    );
}

#[test]
fn custom_urgency_weights() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let t = app.add("x !high", None, None).unwrap();
    assert_eq!(app.task(t.id).unwrap().urgency, 6.0);
    app.set_urgency_weights(ttask_core::urgency::UrgencyWeights {
        priority_high: 1.0,
        ..Default::default()
    });
    assert_eq!(app.task(t.id).unwrap().urgency, 1.0);
}

#[test]
fn waiting_and_someday_are_hidden_until_due() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Visible", None, None).unwrap();
    let waiting = app.add("Follow up wait:+3d", None, None).unwrap();
    let someday = app.add("Learn piano wait:someday", None, None).unwrap();
    app.add("Waited enough wait:yesterday", None, None).unwrap();

    assert_eq!(titles(&app, ""), ["Visible", "Waited enough"]);
    let names = |v: View| -> Vec<String> {
        app.view_tasks(&v, "")
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    };
    assert_eq!(names(View::waiting()), ["Follow up"]);
    assert_eq!(names(View::someday()), ["Learn piano"]);
    assert_eq!(titles(&app, "wait:any").len(), 4);

    // Clearing brings them back; someday and a date exclude each other.
    app.edit(&[someday.id], "wait:none").unwrap();
    app.edit(&[waiting.id], "wait:someday").unwrap();
    let w = app.task(waiting.id).unwrap();
    assert!(w.someday && w.wait_until.is_none());
    assert!(titles(&app, "").contains(&"Learn piano".to_owned()));
}

#[test]
fn recurring_wait_moves_with_the_due_date() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let t = app
        .add("Invoice every:month due:today wait:-2d", None, None)
        .unwrap();
    let next = app.complete(&[t.id]).unwrap().remove(0).next.unwrap();
    assert_eq!(next.due_date, Some(date(2026, 11, 15)));
    assert_eq!(next.wait_until, Some(date(2026, 11, 13)));
}

#[test]
fn contexts_filter_lists_and_seed_new_tasks() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Home task #home", None, None).unwrap();
    app.add("Work task #work +office", None, None).unwrap();
    app.define_context("work", "#work +office").unwrap();
    assert_eq!(app.active_context().unwrap().unwrap().0, "work");
    assert_eq!(titles(&app, ""), ["Work task"]);
    assert_eq!(app.view_count(&View::all()).unwrap(), 1);
    assert_eq!(
        app.list_with("", "", TaskSort::Smart, false).unwrap().len(),
        2,
        "context can be bypassed"
    );

    // New tasks inherit the context's project and tags unless given.
    let t = app.add("Prep slides", None, None).unwrap();
    assert_eq!(t.project.as_deref(), Some("work"));
    assert_eq!(t.tags, ["office"]);
    let other = app.add("Groceries #home", None, None).unwrap();
    assert_eq!(other.project.as_deref(), Some("home"));

    // A context that is not a plain #project/+tag mix filters but seeds nothing.
    app.define_context("urgent", "!urgent or due:<=today")
        .unwrap();
    let plain = app.add("Plain", None, None).unwrap();
    assert_eq!(plain.project, None);

    // Deleting the saved filter silently drops the context.
    app.delete_filter("urgent").unwrap();
    assert!(app.active_context().unwrap().is_none());
    app.set_context(Some("WORK")).unwrap();
    assert_eq!(app.active_context().unwrap().unwrap().0, "work");
    app.set_context(None).unwrap();
    assert!(app.active_context().unwrap().is_none());
    assert!(app.set_context(Some("missing")).is_err());
}

#[test]
fn taskwarrior_import_links_dependencies_and_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let export = r#"[
{"uuid":"tw-1","description":"Get quote","status":"pending","entry":"20261001T090000Z","project":"Work.Acme","tags":["phone"]},
{"uuid":"tw-2","description":"Sign contract","status":"pending","entry":"20261001T090000Z","depends":"tw-1","priority":"H"},
{"uuid":"tw-3","description":"Old","status":"completed","entry":"20260901T090000Z","end":"20261002T090000Z"}
]"#;
    let tz = jiff::tz::TimeZone::UTC;
    let report = app
        .import(io::taskwarrior_import(export, &tz).unwrap(), None, false)
        .unwrap();
    assert_eq!(report.tasks_created, 3);
    let contract = &app.list("Sign", "", TaskSort::Smart).unwrap()[0];
    assert!(contract.blocked);
    assert_eq!(contract.uid, "tw-2");
    assert_eq!(titles(&app, "#work/acme +phone"), ["Get quote"]);
    let again = app
        .import(io::taskwarrior_import(export, &tz).unwrap(), None, false)
        .unwrap();
    assert_eq!((again.tasks_created, again.tasks_updated), (0, 3));
}

#[test]
fn ics_export_lists_due_tasks_in_context() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add(
        "Dentist due:2026-11-03@10:00 remind:due-1h ~45m",
        None,
        None,
    )
    .unwrap();
    app.add("No date", None, None).unwrap();
    app.add("Parked due:+3d wait:someday", None, None).unwrap();
    let tasks = app.ics_tasks("").unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].1.len(), 1, "one alarm");
    let ics = io::ics_export(&tasks, app.now_ms(), &app.clock().time_zone());
    assert!(ics.contains("SUMMARY:Dentist"));
    assert!(ics.contains("DTSTART:20261103T100000Z"));
    assert!(ics.contains("DTEND:20261103T104500Z"));
}

#[test]
fn annotations_append_timestamped_lines_and_undo() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let task = app.add("Call bank", None, None).unwrap();
    app.annotate(task.id, "left a message").unwrap();
    app.annotate(task.id, " called back ").unwrap();
    assert_eq!(
        app.task(task.id).unwrap().notes,
        "- 2026-10-15 12:00 left a message\n- 2026-10-15 12:00 called back"
    );
    app.undo().unwrap();
    assert_eq!(
        app.task(task.id).unwrap().notes,
        "- 2026-10-15 12:00 left a message"
    );
    assert!(app.annotate(task.id, "  ").is_err());
}

#[test]
fn report_buckets_completions_time_and_overdue_by_week() {
    let dir = TempDir::new().unwrap();
    // Week 41 is 5–11 October, week 42 (today, the 15th) 12–18 October.
    let mut app = app_on(&dir, date(2026, 10, 5));
    let late = app.add("Late #work due:2026-10-06", None, None).unwrap();
    let fixed = app.add("Fixed later due:2026-10-08", None, None).unwrap();
    let early = app.add("Early win", None, None).unwrap();
    app.complete(&[early.id]).unwrap();
    drop(app);
    let mut app = app_on(&dir, date(2026, 10, 7));
    app.start_timer(late.id).unwrap();
    drop(app);
    let mut app = app_on(&dir, date(2026, 10, 8));
    app.stop_timer().unwrap();
    drop(app);
    let mut app = app_on(&dir, date(2026, 10, 13));
    app.complete(&[fixed.id]).unwrap();
    drop(app);
    let mut app = app_on(&dir, TODAY);
    let today = app.add("Today's", None, None).unwrap();
    app.complete(&[today.id]).unwrap();

    let report = app.report(2).unwrap();
    let counts = |rows: &[WeekCount]| -> Vec<(String, u32)> {
        rows.iter().map(|r| (r.week.clone(), r.count)).collect()
    };
    assert_eq!(
        counts(&report.completed),
        [("2026-W41".to_owned(), 1), ("2026-W42".to_owned(), 2)]
    );
    assert_eq!(
        counts(&report.overdue),
        [("2026-W41".to_owned(), 2), ("2026-W42".to_owned(), 1)]
    );
    assert_eq!(report.time_since, date(2026, 10, 5));
    assert_eq!(
        report.time,
        [ProjectTime {
            project: Some("work".into()),
            minutes: 1440
        }]
    );
    let text = report.to_lines(10).join("\n");
    assert!(text.contains("  2026-W42  ██████████ 2"), "{text}");
    assert!(text.contains("  work  ██████████ 24h"), "{text}");
}

#[test]
fn task_templates_keep_subtasks_checklists_and_relative_dates() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    let root = app
        .add("Release #work !high due:+3d ~2h +ship", None, None)
        .unwrap();
    app.checklist_add(root.id, "tag the build").unwrap();
    app.add("Changelog due:+1d", None, Some(root.id)).unwrap();
    let template = app.save_task_template("release", root.id).unwrap();
    assert_eq!(template.task_count(), 2);
    assert_eq!(template.tasks[0].due_in_days, Some(3));
    drop(app);

    // Ten days later, with a new title and due date for the main task.
    let mut app = app_on(&dir, date(2026, 10, 25));
    let created = app
        .use_template(
            "Release",
            &UseTemplate {
                text: Some("Release v2 due:+7d".into()),
                ..UseTemplate::default()
            },
        )
        .unwrap();
    let new = &created[0];
    assert_eq!(new.title, "Release v2");
    assert_eq!(new.due_date, Some(date(2026, 11, 1)));
    assert_eq!(new.project.as_deref(), Some("work"));
    assert_eq!(new.priority.name(), "high");
    assert_eq!(new.estimate_min, Some(120));
    assert_eq!(new.tags, ["ship"]);
    let detail = app.detail(new.id).unwrap();
    assert_eq!(detail.checklist[0].text, "tag the build");
    assert_eq!(detail.subtasks[0].title, "Changelog");
    assert_eq!(detail.subtasks[0].due_date, Some(date(2026, 10, 26)));

    app.undo().unwrap();
    assert!(app.task(new.id).is_err(), "one undo removes the whole tree");
    assert!(app.task(detail.subtasks[0].id).is_err());
}

#[test]
fn project_templates_copy_open_tasks_into_a_new_project() {
    let dir = TempDir::new().unwrap();
    let mut app = app(&dir);
    app.add("Laptop #onboarding", None, None).unwrap();
    app.add("Accounts #onboarding due:+2d", None, None).unwrap();
    let done = app.add("Old step #onboarding", None, None).unwrap();
    app.complete(&[done.id]).unwrap();
    app.save_project_template("onboarding", "onboarding")
        .unwrap();
    assert!(app.save_project_template("empty", "nope").is_err());

    let created = app
        .use_template(
            "onboarding",
            &UseTemplate {
                project: Some("onboarding/anna".into()),
                start: Some(date(2026, 11, 2)),
                ..UseTemplate::default()
            },
        )
        .unwrap();
    let mut titles: Vec<&str> = created.iter().map(|t| t.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["Accounts", "Laptop"]);
    assert!(
        created
            .iter()
            .all(|t| t.project.as_deref() == Some("onboarding/anna"))
    );
    let accounts = created.iter().find(|t| t.title == "Accounts").unwrap();
    assert_eq!(accounts.due_date, Some(date(2026, 11, 4)));
    assert_eq!(app.templates().unwrap().len(), 1);
    app.delete_template("onboarding").unwrap();
    assert!(
        app.use_template("onboarding", &UseTemplate::default())
            .is_err()
    );
}
