use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use clap::CommandFactory;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use ttask_app::io::{self, ImportBatch};
use ttask_app::{App, BackupPolicy, View, arrange_tree};
use ttask_core::{SystemClock, Task, TaskId};
use ttask_store::{DbMode, FileState, PasswordSource, Store, StoreError, TaskSort, detect};

use crate::cli::*;
use crate::config::{self, Config, Remember};
use crate::output::{self, Painter, Tone, task_line};
use crate::password::{self, PasswordChain};
use crate::{exit_code, form, keychain, remind};

const PURGE_AFTER_DAYS: i64 = 30;
const HISTORY_DAYS: i64 = 180;
const DAY_MS: i64 = 86_400_000;

struct Ctx {
    config: Config,
    config_path: Option<PathBuf>,
    db: PathBuf,
    password_file: Option<PathBuf>,
    json: bool,
    quiet: bool,
    painter: Painter,
    no_color: bool,
}

impl Ctx {
    /// Prints unless `--quiet`.
    fn say(&self, text: impl AsRef<str>) {
        if !self.quiet {
            println!("{}", text.as_ref());
        }
    }

    fn print_json(&self, value: &impl serde::Serialize) -> anyhow::Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }

    /// Opens the database, unlocking it if needed, and rotates backups.
    fn open(&self) -> anyhow::Result<App> {
        let mut chain = PasswordChain::new(&self.db, self.password_file.as_deref(), &self.config)?;
        let store = Store::open(&self.db, &mut chain)
            .map_err(|err| match err {
                StoreError::Missing(_) => {
                    anyhow!(err).context("run `ttask init` (or `ttask init --encrypt`) first")
                }
                other => anyhow!(other),
            })
            .with_context(|| format!("opening {}", self.db.display()))?;
        if store.mode() == DbMode::Encrypted {
            password::remember_if_configured(&chain, &self.config);
        }
        let app = new_app(store, &self.config);
        if let Some(policy) = backup_policy(&self.config, &self.db, true) {
            if let Err(e) = app.rotate_backups(&policy) {
                eprintln!("warning: automatic backup failed: {e}");
            }
        }
        Ok(app)
    }

    fn backup_dir(&self) -> PathBuf {
        backup_dir(&self.config, &self.db)
    }
}

fn new_app(store: Store, config: &Config) -> App {
    let mut app = App::new(store, SystemClock::new());
    app.set_urgency_weights(config.urgency.clone());
    app
}

fn backup_dir(config: &Config, db: &Path) -> PathBuf {
    match &config.database.backups.dir {
        Some(dir) => config::expand(dir),
        None => db
            .parent()
            .map(|p| p.join("backups"))
            .unwrap_or_else(|| PathBuf::from("backups")),
    }
}

/// Rotating backups for `db`, if configured. Backup file names do not say
/// which database they copy, so a configured `dir` serves only the main
/// database (`is_main`); others use their own `backups/` folder.
fn backup_policy(config: &Config, db: &Path, is_main: bool) -> Option<BackupPolicy> {
    let backups = &config.database.backups;
    if backups.daily == 0 && backups.weekly == 0 || backups.dir.is_some() && !is_main {
        return None;
    }
    Some(BackupPolicy {
        dir: backup_dir(config, db),
        daily: backups.daily,
        weekly: backups.weekly,
    })
}

/// The keychain, and nothing interactive: the TUI asks for a password itself.
struct KeychainOnly<'a>(&'a Path);

impl PasswordSource for KeychainOnly<'_> {
    fn password(&mut self, _attempt: u32) -> Result<SecretString, StoreError> {
        keychain::get(self.0)
            .ok_or_else(|| StoreError::PasswordUnavailable("not in the keychain".into()))
    }
}

pub fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let (config, config_path) = config::load()?;
    let db = config::db_path(cli.db.as_deref(), &config)?;
    let ctx = Ctx {
        painter: Painter::new(cli.no_color),
        no_color: cli.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        config,
        config_path,
        db,
        password_file: cli.password_file,
        json: cli.json,
        quiet: cli.quiet,
    };
    let Some(command) = cli.command else {
        return tui(&ctx).map(|_| ExitCode::SUCCESS);
    };
    match command {
        Command::Init { encrypt } => init(&ctx, encrypt)?,
        Command::Add {
            text,
            notes,
            parent,
        } => add(&ctx, &text.join(" "), notes, parent)?,
        Command::List {
            query,
            all,
            no_context,
            sort,
            limit,
        } => list(&ctx, &query, all, no_context, sort, limit)?,
        Command::Context { args } => context(&ctx, &args)?,
        Command::Show { id, history } => show(&ctx, id, history)?,
        Command::Edit {
            id,
            changes,
            notes,
            parent,
        } => edit(&ctx, id, &changes, notes, parent)?,
        Command::Done(Targets {
            filter: Some(query),
            yes,
            ..
        }) => bulk(&ctx, &query, Bulk::Done, yes)?,
        Command::Reopen(Targets {
            filter: Some(query),
            yes,
            ..
        }) => bulk(&ctx, &query, Bulk::Reopen, yes)?,
        Command::Cancel(Targets {
            filter: Some(query),
            yes,
            ..
        }) => bulk(&ctx, &query, Bulk::Cancel, yes)?,
        Command::Rm(Targets {
            filter: Some(query),
            yes,
            ..
        }) => bulk(&ctx, &query, Bulk::Delete, yes)?,
        Command::Done(Targets { ids, .. }) => {
            return per_task(&ctx, &ids, "Completed", |app, id| {
                let done = app.complete(&[id])?.remove(0);
                let mut line = format!("{}: {}", done.task.id, done.task.title);
                if let Some(next) = &done.next {
                    line.push_str(&format!(
                        " (next: {} on {})",
                        next.id,
                        next.due_date.map(|d| d.to_string()).unwrap_or_default()
                    ));
                }
                Ok((serde_json::to_value(&done)?, line))
            });
        }
        Command::Reopen(Targets { ids, .. }) => {
            return per_task(&ctx, &ids, "Reopened", |app, id| {
                let t = app.reopen(&[id])?.remove(0);
                Ok((serde_json::to_value(&t)?, format!("{}: {}", t.id, t.title)))
            });
        }
        Command::Cancel(Targets { ids, .. }) => {
            return per_task(&ctx, &ids, "Cancelled", |app, id| {
                let t = app.cancel(&[id])?.remove(0);
                Ok((serde_json::to_value(&t)?, format!("{}: {}", t.id, t.title)))
            });
        }
        Command::Rm(Targets { ids, .. }) => {
            return per_task(&ctx, &ids, "Deleted", |app, id| {
                let t = app.delete(&[id])?.remove(0);
                Ok((serde_json::to_value(&t)?, format!("{}: {}", t.id, t.title)))
            });
        }
        Command::Skip { id } => {
            let mut app = ctx.open()?;
            let t = app.skip(id)?;
            if ctx.json {
                ctx.print_json(&t)?;
            } else {
                ctx.say(format!(
                    "Skipped {}: {} is now due {}",
                    t.id,
                    t.title,
                    t.due_date.map(|d| d.to_string()).unwrap_or_default()
                ));
            }
        }
        Command::Undo | Command::Redo => {
            let redo = matches!(command, Command::Redo);
            let mut app = ctx.open()?;
            let outcome = if redo { app.redo()? } else { app.undo()? };
            match (&outcome, ctx.json) {
                (_, true) => ctx.print_json(&outcome)?,
                (Some(o), false) => ctx.say(format!(
                    "{} {}: {}",
                    if redo { "Redid" } else { "Undid" },
                    o.kinds.join(", "),
                    o.titles.join(", ")
                )),
                (None, false) => ctx.say(if redo {
                    "Nothing to redo"
                } else {
                    "Nothing to undo"
                }),
            }
        }
        Command::Count {
            query,
            all,
            no_context,
        } => count(&ctx, &query, all, no_context)?,
        Command::Open { id, n, all, print } => open_links(&ctx, id, n, all, print)?,
        Command::Search { text, all } => search(&ctx, &text.join(" "), all)?,
        Command::Checklist { task, action } => checklist(&ctx, task, action)?,
        Command::Block { id, on } => {
            ctx.open()?.block(id, on)?;
            ctx.say(format!("Task {id} now waits on task {on}"));
        }
        Command::Unblock { id, on } => {
            ctx.open()?.unblock(id, on)?;
            ctx.say(format!("Task {id} no longer waits on task {on}"));
        }
        Command::Timer { action } => timer(&ctx, action)?,
        Command::Remind(args) => reminders(&ctx, args)?,
        Command::Project { action } => projects(&ctx, action)?,
        Command::Tag { action } => tags(&ctx, action)?,
        Command::Filter { action } => filters(&ctx, action)?,
        Command::Import {
            file,
            format,
            dry_run,
            project,
            map,
        } => import(&ctx, &file, format, dry_run, project.as_deref(), &map)?,
        Command::Export {
            format,
            output,
            encrypt,
            filter,
        } => export(&ctx, format, output, encrypt, filter)?,
        Command::Db { action } => return db_cmd(&ctx, action),
        Command::Gc => gc(&ctx)?,
        Command::Lock => match keychain::delete(&ctx.db) {
            Ok(true) => ctx.say("Removed the cached password from the keychain"),
            Ok(false) => ctx.say("No cached password"),
            Err(e) => bail!("keychain: {e}"),
        },
        Command::Config { action } => config_cmd(&ctx, action)?,
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "ttask", &mut std::io::stdout());
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- TUI

fn tui(ctx: &Ctx) -> anyhow::Result<()> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        bail!("the TUI needs a terminal; try `ttask ls` or `ttask --help`");
    }
    let app = ctx.open()?;
    let encrypted = app.store().mode() == DbMode::Encrypted;
    let ui = &ctx.config.ui;
    let config = ctx.config.clone();
    let main_db = ctx.db.clone();
    let open_db: ttask_tui::Opener = Box::new(move |path, password| {
        let store = match password {
            Some(password) => {
                let store = Store::open(path, &mut password.clone())?;
                if config.security.remember == Remember::Keychain {
                    // Best effort: there is no terminal to warn on.
                    let _ = keychain::set(path, password);
                }
                store
            }
            None => Store::open(path, &mut KeychainOnly(path))?,
        };
        let app = new_app(store, &config);
        if let Some(policy) = backup_policy(&config, path, path == main_db) {
            let _ = app.rotate_backups(&policy);
        }
        Ok(app)
    });
    let settings = ttask_tui::Settings {
        theme: ui.theme,
        keymap: ui.keymap,
        detail_pane: ui.show_detail_pane,
        keys: ctx.config.keys.clone(),
        no_color: ctx.no_color,
        encrypted,
        idle_lock: ctx
            .config
            .security
            .idle_lock_minutes
            .filter(|m| *m > 0)
            .map(|m| std::time::Duration::from_secs(u64::from(m) * 60)),
        unlock: Some(Box::new(|db: &Path, password: &SecretString| {
            Store::open(db, &mut password.clone()).is_ok()
        })),
        notify: Some(Box::new(remind::notify)),
        open_db: Some(open_db),
        open_link: Some(Box::new(ttask_app::launch::open_link)),
    };
    ttask_tui::run(app, settings)
}

// ------------------------------------------------------------- basics

fn init(ctx: &Ctx, encrypt: bool) -> anyhow::Result<()> {
    if detect(&ctx.db)? != FileState::Missing {
        bail!(StoreError::AlreadyExists(ctx.db.clone()));
    }
    let password = if encrypt {
        Some(password::new_password(
            ctx.password_file.as_deref(),
            "New password",
        )?)
    } else {
        None
    };
    let store = Store::create(&ctx.db, password.as_ref())?;
    if let (Some(password), Remember::Keychain) = (&password, ctx.config.security.remember) {
        if let Err(e) = keychain::set(&ctx.db, password) {
            eprintln!("warning: could not save the password to the keychain: {e}");
        }
    }
    let kind = match store.mode() {
        DbMode::Plain => "plain",
        DbMode::Encrypted => "encrypted",
    };
    ctx.say(format!("Created {kind} database at {}", ctx.db.display()));
    Ok(())
}

fn add(ctx: &Ctx, text: &str, notes: Option<String>, parent: Option<TaskId>) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    let mut task = app.add(text, None, parent)?;
    if let Some(notes) = notes {
        task = app.set_notes(task.id, &notes)?;
    }
    if ctx.json {
        ctx.print_json(&task)?;
    } else {
        let mut line = format!("Added task {}: {}", task.id, task.title);
        if let Some(due) = task.due_date {
            line.push_str(&format!(" (due {due})"));
        }
        ctx.say(line);
    }
    Ok(())
}

fn list(
    ctx: &Ctx,
    query: &[String],
    all: bool,
    no_context: bool,
    sort: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<()> {
    let app = ctx.open()?;
    let (query, mut sort_order) = resolve_query(&app, query);
    if let Some(sort) = sort {
        sort_order = sort.parse().map_err(|e: String| anyhow!(e))?;
    }
    let extra = if all { "status:all wait:any" } else { "" };
    let mut tasks = app.list_with(&query, extra, sort_order, !no_context)?;
    if let Some(limit) = limit {
        tasks.truncate(limit);
    }
    if ctx.json {
        return ctx.print_json(&tasks);
    }
    let context = (!no_context)
        .then(|| app.active_context())
        .transpose()?
        .flatten()
        .map(|(name, _)| name);
    if tasks.is_empty() {
        let mut msg = if all { "No tasks." } else { "No open tasks." }.to_owned();
        if let Some(name) = &context {
            msg.push_str(&format!(
                " (context `{name}`; --no-context shows everything)"
            ));
        }
        ctx.say(msg);
        return Ok(());
    }
    let today = app.today();
    let show_urgency = sort_order == TaskSort::Urgency;
    for (task, depth) in arrange_tree(tasks) {
        let mut line = task_line(&task, depth, today, &ctx.painter);
        if show_urgency {
            line.push_str(
                &ctx.painter
                    .paint(Tone::Dim, &format!("  u{:.1}", task.urgency)),
            );
        }
        println!("{line}");
    }
    if let Some(name) = context {
        eprintln!(
            "{}",
            ctx.painter.paint(Tone::Dim, &format!("context: {name}"))
        );
    }
    Ok(())
}

/// A single word naming a view or saved filter becomes its query and sort;
/// anything else is a filter query.
fn resolve_query(app: &App, query: &[String]) -> (String, TaskSort) {
    let view = match query {
        [name] => View::builtin_named(name).or_else(|| {
            app.saved_filters()
                .ok()?
                .into_iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
                .map(|f| View::saved(&f))
        }),
        _ => None,
    };
    match view {
        Some(v) => (v.query, v.sort),
        None => (query.join(" "), TaskSort::Smart),
    }
}

fn count(ctx: &Ctx, query: &[String], all: bool, no_context: bool) -> anyhow::Result<()> {
    let app = ctx.open()?;
    let (query, _) = resolve_query(&app, query);
    let extra = if all { "status:all wait:any" } else { "" };
    let n = app.count(&query, extra, !no_context)?;
    if ctx.json {
        return ctx.print_json(&json!({ "count": n }));
    }
    println!("{n}");
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Bulk {
    Done,
    Reopen,
    Cancel,
    Delete,
}

impl Bulk {
    /// (`complete`, `Completed`)
    fn verbs(self) -> (&'static str, &'static str) {
        match self {
            Bulk::Done => ("complete", "Completed"),
            Bulk::Reopen => ("reopen", "Reopened"),
            Bulk::Cancel => ("cancel", "Cancelled"),
            Bulk::Delete => ("delete", "Deleted"),
        }
    }
}

/// `done/reopen/cancel/rm --filter QUERY`: one batch, so one undo.
fn bulk(ctx: &Ctx, query: &str, action: Bulk, yes: bool) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    // Without a status term `ls` shows open tasks, which cannot be reopened.
    let extra = match action {
        Bulk::Reopen => "not status:open",
        _ => "",
    };
    let tasks = app.list(query, extra, TaskSort::Smart)?;
    let (verb, past) = action.verbs();
    if tasks.is_empty() {
        ctx.say("No tasks match.");
        return Ok(());
    }
    if tasks.len() > 1 && !yes {
        confirm_bulk(ctx, &app, &tasks, verb)?;
    }
    let ids: Vec<TaskId> = tasks.iter().map(|t| t.id).collect();
    let changed: Vec<Task> = match action {
        Bulk::Done => app.complete(&ids)?.into_iter().map(|c| c.task).collect(),
        Bulk::Reopen => app.reopen(&ids)?,
        Bulk::Cancel => app.cancel(&ids)?,
        Bulk::Delete => app.delete(&ids)?,
    };
    if ctx.json {
        return ctx.print_json(&changed);
    }
    for t in &changed {
        ctx.say(format!("{past} task {}: {}", t.id, t.title));
    }
    if changed.len() > 1 {
        ctx.say(format!(
            "{past} {} tasks; `ttask undo` reverts them together",
            changed.len()
        ));
    }
    Ok(())
}

/// Lists the tasks and asks on a terminal; elsewhere `--yes` is required.
fn confirm_bulk(ctx: &Ctx, app: &App, tasks: &[Task], verb: &str) -> anyhow::Result<()> {
    let n = tasks.len();
    if !std::io::stdin().is_terminal() {
        bail!("the filter matches {n} tasks; pass --yes to {verb} them all");
    }
    let today = app.today();
    for task in tasks {
        eprintln!("{}", task_line(task, 0, today, &ctx.painter));
    }
    if let Some((name, _)) = app.active_context()? {
        eprintln!("(within context `{name}`)");
    }
    eprint!("{} these {n} tasks? [y/N] ", capitalise(verb));
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        bail!("nothing changed");
    }
    Ok(())
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn open_links(
    ctx: &Ctx,
    id: TaskId,
    n: Option<usize>,
    all: bool,
    print: bool,
) -> anyhow::Result<()> {
    let app = ctx.open()?;
    let links = app.links(id)?;
    if links.is_empty() {
        bail!("task {id} has no http, https or mailto links");
    }
    let chosen: Vec<&String> = match n {
        Some(n) => vec![
            links
                .get(n.wrapping_sub(1))
                .ok_or_else(|| StoreError::NotFound(format!("link {n} of task {id}")))?,
        ],
        None if all || print || links.len() == 1 => links.iter().collect(),
        None => {
            for (i, link) in links.iter().enumerate() {
                println!("{:>3}. {link}", i + 1);
            }
            bail!(
                "task {id} has {} links; open one with `ttask open {id} <N>` or all with --all",
                links.len()
            );
        }
    };
    if print {
        if ctx.json {
            return ctx.print_json(&chosen);
        }
        for link in chosen {
            println!("{link}");
        }
        return Ok(());
    }
    for link in chosen {
        ttask_app::launch::open_link(link).with_context(|| format!("opening {link}"))?;
        ctx.say(format!("Opened {link}"));
    }
    Ok(())
}

fn context(ctx: &Ctx, args: &[String]) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    match args {
        [] => match app.active_context()? {
            Some((name, _)) => {
                let query = app
                    .saved_filters()?
                    .into_iter()
                    .find(|f| f.name == name)
                    .map(|f| f.query)
                    .unwrap_or_default();
                ctx.say(format!("{name}  {}", ctx.painter.paint(Tone::Dim, &query)));
            }
            None => ctx.say("No context"),
        },
        [none] if none.eq_ignore_ascii_case("none") => {
            app.set_context(None)?;
            ctx.say("Context cleared");
        }
        [define, name, query @ ..] if define == "define" => {
            if query.is_empty() {
                bail!("usage: ttask context define <name> <query>");
            }
            app.define_context(name, &query.join(" "))?;
            ctx.say(format!("Context: {name}"));
        }
        [name] => {
            let name = app.set_context(Some(name))?.unwrap_or_default();
            ctx.say(format!("Context: {name}"));
        }
        _ => bail!("usage: ttask context [NAME | none | define NAME QUERY]"),
    }
    Ok(())
}

fn show(ctx: &Ctx, id: TaskId, with_history: bool) -> anyhow::Result<()> {
    let app = ctx.open()?;
    let detail = app.detail(id)?;
    let history = if with_history {
        app.history(id)?
    } else {
        Vec::new()
    };
    if ctx.json {
        return ctx.print_json(&json!({ "detail": detail, "history": history }));
    }
    let p = &ctx.painter;
    let t = &detail.task;
    let today = app.today();
    let tz = app.clock().time_zone();
    let row = |label: &str, value: String| {
        println!("  {}{value}", p.paint(Tone::Dim, &format!("{label:<10}")))
    };
    let recur = if t.recur_rule.is_some() { " ↻" } else { "" };
    println!(
        "{} {}{recur}  [{}]",
        p.paint(Tone::Dim, &format!("#{}", t.id)),
        p.paint(Tone::Bold, &t.title),
        t.status
    );
    if let Some(parent) = &detail.parent {
        row("subtask of", format!("#{} {}", parent.id, parent.title));
    }
    if let Some(project) = &t.project {
        row("project", p.paint(Tone::Cyan, project));
    }
    if !t.tags.is_empty() {
        let tags: Vec<String> = t.tags.iter().map(|g| format!("+{g}")).collect();
        row("tags", p.paint(Tone::Magenta, &tags.join(" ")));
    }
    row("priority", t.priority.name().to_owned());
    if t.is_open() {
        row("urgency", format!("{:.1}", t.urgency));
    }
    if t.someday {
        row("waiting", "someday".to_owned());
    } else if let Some(wait) = t.wait_until {
        row("waiting", format!("until {wait}"));
    }
    if let Some(due) = output::due_label(t, today) {
        let full = format!(
            "{} ({due})",
            t.due_date.map(|d| d.to_string()).unwrap_or_default()
        );
        row(
            "due",
            if t.is_overdue(today) {
                p.paint(Tone::Red, &full)
            } else {
                full
            },
        );
    }
    if let Some(sched) = t.scheduled_date {
        row("scheduled", sched.to_string());
    }
    if let Some(recur) = output::describe_recurrence(t) {
        row("repeats", recur);
    }
    if let Some(est) = output::estimate(t) {
        row("estimate", est);
    }
    if detail.tracked_minutes > 0 {
        row(
            "tracked",
            ttask_core::duration::format_minutes(detail.tracked_minutes as u32),
        );
    }
    for b in &detail.blocked_by {
        row("waits on", format!("#{} {} [{}]", b.id, b.title, b.status));
    }
    for b in &detail.blocking {
        row("blocks", format!("#{} {}", b.id, b.title));
    }
    for r in &detail.reminders {
        let state = if r.fired_at.is_some() { " (fired)" } else { "" };
        row(
            "reminder",
            format!("{}{state}", output::instant(r.fire_at, &tz)),
        );
    }
    row("created", output::instant(t.created_at, &tz));
    if let Some(done) = t.completed_at {
        row("closed", output::instant(done, &tz));
    }
    row("uid", p.paint(Tone::Dim, &t.uid));
    if !t.notes.is_empty() {
        println!();
        for line in t.notes.lines() {
            println!("  {line}");
        }
    }
    if !detail.checklist.is_empty() {
        let done = detail.checklist.iter().filter(|c| c.checked).count();
        println!("\n  Checklist {done}/{}", detail.checklist.len());
        for (i, item) in detail.checklist.iter().enumerate() {
            println!(
                "  {:>3}. [{}] {}",
                i + 1,
                if item.checked { "x" } else { " " },
                item.text
            );
        }
    }
    if !detail.subtasks.is_empty() {
        println!("\n  Subtasks");
        for sub in &detail.subtasks {
            println!("  {}", task_line(sub, 0, today, p));
        }
    }
    if with_history {
        println!("\n  History");
        for entry in &history {
            let changed = if entry.changed.is_empty() {
                String::new()
            } else {
                format!(": {}", entry.changed.join(", "))
            };
            println!(
                "  {}  {}{changed}",
                output::instant(entry.at, &tz),
                entry.kind
            );
        }
    }
    Ok(())
}

fn edit(
    ctx: &Ctx,
    first: TaskId,
    changes: &[String],
    notes: Option<String>,
    parent: Option<TaskId>,
) -> anyhow::Result<()> {
    let mut ids = vec![first];
    let mut rest = changes;
    while let Some((head, tail)) = rest.split_first() {
        match head.parse::<TaskId>() {
            Ok(id) => {
                ids.push(id);
                rest = tail;
            }
            Err(_) => break,
        }
    }
    let text = rest.join(" ");
    let mut app = ctx.open()?;
    if text.is_empty() && notes.is_none() && parent.is_none() {
        if ids.len() > 1 {
            bail!("the editor form edits one task at a time");
        }
        return edit_in_editor(ctx, &mut app, first);
    }
    let mut updated: Vec<Task> = Vec::new();
    if !text.is_empty() {
        updated = app.edit(&ids, &text)?;
    }
    for id in &ids {
        if let Some(notes) = &notes {
            app.set_notes(*id, notes)?;
        }
        if let Some(parent) = parent {
            app.set_parent(*id, (parent != 0).then_some(parent))?;
        }
    }
    if updated.is_empty() || notes.is_some() || parent.is_some() {
        updated = ids
            .iter()
            .map(|id| app.task(*id))
            .collect::<Result<_, _>>()?;
    }
    if ctx.json {
        return ctx.print_json(&updated);
    }
    let today = app.today();
    for task in &updated {
        ctx.say(format!(
            "Updated {}",
            task_line(task, 0, today, &ctx.painter).trim_start()
        ));
    }
    Ok(())
}

fn edit_in_editor(ctx: &Ctx, app: &mut App, id: TaskId) -> anyhow::Result<()> {
    let task = app.task(id)?;
    let original = form::render(&task);
    let edited = ttask_app::editor::edit_text(&original, ".toml")?;
    if edited == original {
        ctx.say("No changes");
        return Ok(());
    }
    let (mut mods, notes) = form::parse(&edited, app.today())?;
    mods.remove_tags = task
        .tags
        .iter()
        .filter(|t| !mods.add_tags.iter().any(|a| a.eq_ignore_ascii_case(t)))
        .cloned()
        .collect();
    app.edit_with(&[id], &mods)?;
    if notes != task.notes {
        app.set_notes(id, &notes)?;
    }
    let task = app.task(id)?;
    ctx.say(format!(
        "Updated {}",
        task_line(&task, 0, app.today(), &ctx.painter).trim_start()
    ));
    Ok(())
}

/// Runs `f` for each id separately so one bad id does not block the rest;
/// exits with the first error's code.
fn per_task(
    ctx: &Ctx,
    ids: &[TaskId],
    verb: &str,
    mut f: impl FnMut(&mut App, TaskId) -> anyhow::Result<(serde_json::Value, String)>,
) -> anyhow::Result<ExitCode> {
    let mut app = ctx.open()?;
    let mut json_out = Vec::new();
    let mut first_error: Option<anyhow::Error> = None;
    for &id in ids {
        match f(&mut app, id) {
            Ok((value, line)) => {
                json_out.push(value);
                if !ctx.json {
                    ctx.say(format!("{verb} task {line}"));
                }
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                first_error.get_or_insert(e);
            }
        }
    }
    if ctx.json {
        ctx.print_json(&json_out)?;
    }
    Ok(match first_error {
        Some(e) => ExitCode::from(exit_code(&e)),
        None => ExitCode::SUCCESS,
    })
}

fn search(ctx: &Ctx, text: &str, all: bool) -> anyhow::Result<()> {
    let app = ctx.open()?;
    let hits = app.search(text, all)?;
    if ctx.json {
        return ctx.print_json(&hits);
    }
    if hits.is_empty() {
        ctx.say("No matches.");
    }
    let today = app.today();
    for hit in hits {
        println!("{}", task_line(&hit.task, 0, today, &ctx.painter));
        if hit.snippet != hit.task.title {
            println!("        {}", ctx.painter.paint(Tone::Dim, &hit.snippet));
        }
    }
    Ok(())
}

fn checklist(ctx: &Ctx, task: TaskId, action: Option<ChecklistAction>) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    let item_at = |app: &App, pos: usize| -> anyhow::Result<i64> {
        let items = app.detail(task)?.checklist;
        items
            .get(pos.wrapping_sub(1))
            .map(|i| i.id)
            .ok_or_else(|| anyhow!(StoreError::NotFound(format!("checklist item {pos}"))))
    };
    match action {
        Some(ChecklistAction::Add { text }) => {
            app.checklist_add(task, &text.join(" "))?;
        }
        Some(ChecklistAction::Toggle { item }) => {
            let id = item_at(&app, item)?;
            app.checklist_toggle(id)?;
        }
        Some(ChecklistAction::Rm { item }) => {
            let id = item_at(&app, item)?;
            app.checklist_remove(id)?;
        }
        None => {}
    }
    let items = app.detail(task)?.checklist;
    if ctx.json {
        return ctx.print_json(&items);
    }
    for (i, item) in items.iter().enumerate() {
        ctx.say(format!(
            "{:>3}. [{}] {}",
            i + 1,
            if item.checked { "x" } else { " " },
            item.text
        ));
    }
    Ok(())
}

fn timer(ctx: &Ctx, action: TimerAction) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    match action {
        TimerAction::Start { id } => {
            let (stopped, _) = app.start_timer(id)?;
            if let Some(entry) = stopped {
                ctx.say(format!(
                    "Stopped task {} after {} min",
                    entry.task_id,
                    entry.minutes(app.now_ms())
                ));
            }
            ctx.say(format!("Timing task {id}: {}", app.task(id)?.title));
        }
        TimerAction::Stop => match app.stop_timer()? {
            Some(entry) => ctx.say(format!(
                "Stopped task {} after {} min",
                entry.task_id,
                entry.minutes(app.now_ms())
            )),
            None => ctx.say("No timer running"),
        },
        TimerAction::Status => match app.running_timer()? {
            Some((entry, task)) if ctx.json => {
                ctx.print_json(&json!({"entry": entry, "task": task}))?
            }
            Some((entry, task)) => ctx.say(format!(
                "Timing {}: {} for {} min",
                task.id,
                task.title,
                entry.minutes(app.now_ms())
            )),
            None => ctx.say("No timer running"),
        },
    }
    Ok(())
}

fn reminders(ctx: &Ctx, args: RemindArgs) -> anyhow::Result<()> {
    if args.install {
        ctx.say(remind::install(&ctx.db)?);
        return Ok(());
    }
    if args.uninstall {
        ctx.say(remind::uninstall()?);
        return Ok(());
    }
    let mut app = ctx.open()?;
    let tz = app.clock().time_zone();
    if args.run {
        let fired = app.take_due_reminders()?;
        for (_, task) in &fired {
            let mut body = task.title.clone();
            if let Some(due) = output::due_label(task, app.today()) {
                body.push_str(&format!(" (due {due})"));
            }
            remind::notify("ttask reminder", &body);
            ctx.say(format!("Reminded: {}", task.title));
        }
        return Ok(());
    }
    let upcoming = app.upcoming_reminders(50)?;
    if ctx.json {
        let rows: Vec<_> = upcoming
            .iter()
            .map(|(r, t)| json!({"reminder": r, "task": t}))
            .collect();
        return ctx.print_json(&rows);
    }
    if upcoming.is_empty() {
        ctx.say("No upcoming reminders.");
    }
    for (r, t) in upcoming {
        ctx.say(format!(
            "{}  {:>4}  {}",
            output::instant(r.fire_at, &tz),
            t.id,
            t.title
        ));
    }
    Ok(())
}

// ------------------------------------------------- projects, tags, filters

fn projects(ctx: &Ctx, action: Option<ProjectAction>) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    match action.unwrap_or(ProjectAction::Ls { archived: false }) {
        ProjectAction::Ls { archived } => {
            let projects = app.projects(archived)?;
            if ctx.json {
                return ctx.print_json(&projects);
            }
            if projects.is_empty() {
                ctx.say(
                    "No projects. Add one with `ttask project add <name>` or `#name` in quick-add.",
                );
            }
            for p in projects {
                let indent = if p.parent_id.is_some() { "  " } else { "" };
                let archived = if p.archived { " (archived)" } else { "" };
                println!(
                    "{indent}{}{archived}  {}",
                    p.path,
                    ctx.painter.paint(Tone::Dim, &p.open_tasks.to_string())
                );
            }
        }
        ProjectAction::Add { path } => {
            app.ensure_project(&path)?;
            ctx.say(format!("Project {path} ready"));
        }
        ProjectAction::Rename { path, new_name } => {
            app.rename_project(&path, &new_name)?;
            ctx.say(format!("Renamed {path} to {new_name}"));
        }
        ProjectAction::Archive { path } => {
            app.archive_project(&path, true)?;
            ctx.say(format!("Archived {path}"));
        }
        ProjectAction::Unarchive { path } => {
            app.archive_project(&path, false)?;
            ctx.say(format!("Unarchived {path}"));
        }
        ProjectAction::Rm { path } => {
            let moved = app.delete_project(&path)?;
            ctx.say(format!("Deleted {path}; moved {moved} task(s) up a level"));
        }
    }
    Ok(())
}

fn tags(ctx: &Ctx, action: Option<TagAction>) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    match action.unwrap_or(TagAction::Ls) {
        TagAction::Ls => {
            let tags = app.tags()?;
            if ctx.json {
                return ctx.print_json(&tags);
            }
            for t in tags {
                println!(
                    "+{}  {}",
                    t.name,
                    ctx.painter.paint(Tone::Dim, &t.open_tasks.to_string())
                );
            }
        }
        TagAction::Rename { old, new } => {
            app.rename_tag(old.trim_start_matches('+'), new.trim_start_matches('+'))?;
            ctx.say(format!("Renamed +{old} to +{new}"));
        }
        TagAction::Rm { name } => {
            app.delete_tag(name.trim_start_matches('+'))?;
            ctx.say(format!("Removed +{name}"));
        }
    }
    Ok(())
}

fn filters(ctx: &Ctx, action: Option<FilterAction>) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    match action.unwrap_or(FilterAction::Ls) {
        FilterAction::Ls => {
            let filters = app.saved_filters()?;
            if ctx.json {
                return ctx.print_json(&filters);
            }
            for f in filters {
                println!("{}  {}", f.name, ctx.painter.paint(Tone::Dim, &f.query));
            }
        }
        FilterAction::Save { name, query } => {
            app.save_filter(&name, &query.join(" "))?;
            ctx.say(format!("Saved filter `{name}`"));
        }
        FilterAction::Rm { name } => {
            app.delete_filter(&name)?;
            ctx.say(format!("Deleted filter `{name}`"));
        }
    }
    Ok(())
}

// ------------------------------------------------------ import / export

const AGE_MAGIC: &[u8] = b"age-encryption.org/v1";

fn import(
    ctx: &Ctx,
    file: &Path,
    format: Option<ImportFormat>,
    dry_run: bool,
    project: Option<&str>,
    map: &[String],
) -> anyhow::Result<()> {
    let mut bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    if bytes.starts_with(AGE_MAGIC) {
        let passphrase = export_passphrase(false)?;
        bytes = age_decrypt(&bytes, &passphrase)?;
    }
    let text = String::from_utf8(bytes).context("the file is not UTF-8 text")?;
    let text = text.trim_start_matches('\u{feff}');
    let format = format.unwrap_or_else(|| guess_format(file, text));
    let mapping: Vec<(String, String)> = map
        .iter()
        .map(|m| {
            m.split_once('=')
                .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
                .ok_or_else(|| anyhow!("--map entries look like field=Column, got `{m}`"))
        })
        .collect::<anyhow::Result<_>>()?;
    let mut app = ctx.open()?;
    let today = app.today();
    let batch: ImportBatch = match format {
        ImportFormat::Json => io::parse_json(text)?.into(),
        ImportFormat::Csv => io::csv_import(text, &mapping, today)?,
        ImportFormat::Todotxt => io::todotxt_import(text, today)?,
        ImportFormat::Todoist => {
            let default = project
                .map(str::to_owned)
                .or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()));
            io::todoist_import(text, default.as_deref(), today)?
        }
        ImportFormat::Ticktick => io::ticktick_import(text, today)?,
        ImportFormat::Taskwarrior => io::taskwarrior_import(text, &app.clock().time_zone())?,
        ImportFormat::Github => io::github_import(text, project, &app.clock().time_zone())?,
    };
    let default_project = if matches!(format, ImportFormat::Todoist | ImportFormat::Github) {
        None
    } else {
        project
    };
    let report = app.import(batch, default_project, dry_run)?;
    if ctx.json {
        return ctx.print_json(&report);
    }
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    let prefix = if dry_run { "Would import" } else { "Imported" };
    ctx.say(format!(
        "{prefix}: {} new, {} updated task(s), {} new project(s), {} filter(s){}",
        report.tasks_created,
        report.tasks_updated,
        report.projects_created,
        report.filters,
        if dry_run {
            ""
        } else {
            ". `ttask undo` reverts it."
        }
    ));
    Ok(())
}

fn guess_format(file: &Path, text: &str) -> ImportFormat {
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let first_line = text.lines().next().unwrap_or("").to_ascii_uppercase();
    match ext.as_str() {
        "json" if is_taskwarrior(text) => ImportFormat::Taskwarrior,
        "json" if is_github(text) => ImportFormat::Github,
        "json" => ImportFormat::Json,
        "csv" if first_line.starts_with("TYPE,CONTENT") => ImportFormat::Todoist,
        "csv" if text.contains("\"List Name\"") || text.contains("List Name,") => {
            ImportFormat::Ticktick
        }
        "csv" => ImportFormat::Csv,
        _ if is_taskwarrior(text) => ImportFormat::Taskwarrior,
        _ if is_github(text) => ImportFormat::Github,
        _ if text.trim_start().starts_with('{') => ImportFormat::Json,
        _ => ImportFormat::Todotxt,
    }
}

/// `task export` output: a JSON array (or lines) of objects with `uuid`
/// and `description`, unlike ttask's own `{"format": "ttask", …}`.
fn is_taskwarrior(text: &str) -> bool {
    let head: String = text.chars().take(4096).collect();
    let start = head.trim_start();
    (start.starts_with('[') || start.starts_with("{\""))
        && head.contains("\"uuid\"")
        && head.contains("\"description\"")
        && !head.contains("\"format\"")
}

/// A GitHub issue list: objects with `number`, `title` and a github.com URL.
fn is_github(text: &str) -> bool {
    let head: String = text.chars().take(4096).collect();
    let start = head.trim_start();
    (start.starts_with('[') || start.starts_with('{'))
        && head.contains("\"number\"")
        && head.contains("\"title\"")
        && head.contains("github.com/")
        && !head.contains("\"format\"")
}

fn export(
    ctx: &Ctx,
    format: ExportFormat,
    output: Option<PathBuf>,
    encrypt: bool,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let app = ctx.open()?;
    if filter.is_some() && format != ExportFormat::Ics {
        bail!("--filter applies to --format ics; the other formats export everything");
    }
    let text = match format {
        ExportFormat::Ics => {
            let tasks = app.ics_tasks(filter.as_deref().unwrap_or(""))?;
            io::ics_export(&tasks, app.now_ms(), &app.clock().time_zone())
        }
        ExportFormat::Json => serde_json::to_string_pretty(&app.export()?)?,
        ExportFormat::Csv => io::csv_export(&app)?,
        ExportFormat::Markdown => io::markdown_export(&app)?,
        ExportFormat::Todotxt => io::todotxt_export(&app)?,
    };
    if encrypt {
        let path = output.ok_or_else(|| anyhow!("--encrypt needs --output <file>"))?;
        let passphrase = export_passphrase(true)?;
        let bytes = age_encrypt(text.as_bytes(), &passphrase)?;
        write_new(&path, &bytes)?;
        ctx.say(format!("Wrote encrypted export to {}", path.display()));
        return Ok(());
    }
    if app.store().mode() == DbMode::Encrypted {
        eprintln!(
            "warning: this export is not encrypted; use --encrypt --output <file> to protect it"
        );
    }
    match output {
        Some(path) => {
            write_new(&path, text.as_bytes())?;
            ctx.say(format!("Wrote {}", path.display()));
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(text.as_bytes())?;
            if !text.ends_with('\n') {
                stdout.write_all(b"\n")?;
            }
        }
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// `TTASK_EXPORT_PASSPHRASE`, else a prompt (twice when encrypting).
fn export_passphrase(confirm: bool) -> anyhow::Result<SecretString> {
    if let Some(value) = std::env::var("TTASK_EXPORT_PASSPHRASE")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return Ok(SecretString::from(value));
    }
    let first = password::prompt("Export passphrase: ")?;
    if confirm {
        let second = password::prompt("Repeat passphrase: ")?;
        if first.expose_secret() != second.expose_secret() {
            bail!("passphrases do not match");
        }
        password::check_strength(&first)?;
    }
    Ok(first)
}

fn age_encrypt(plain: &[u8], passphrase: &SecretString) -> anyhow::Result<Vec<u8>> {
    let encryptor = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(
        passphrase.expose_secret().to_owned(),
    ));
    let mut out = Vec::new();
    let mut writer = encryptor.wrap_output(&mut out)?;
    writer.write_all(plain)?;
    writer.finish()?;
    Ok(out)
}

fn age_decrypt(cipher: &[u8], passphrase: &SecretString) -> anyhow::Result<Vec<u8>> {
    let decryptor = age::Decryptor::new(cipher)?;
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(
        passphrase.expose_secret().to_owned(),
    ));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| anyhow!("cannot decrypt the export: {e}"))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

// ----------------------------------------------------------- database

fn db_cmd(ctx: &Ctx, action: DbAction) -> anyhow::Result<ExitCode> {
    match action {
        DbAction::Info => {
            let app = ctx.open()?;
            let store = app.store();
            let size = fs::metadata(&ctx.db).map(|m| m.len()).unwrap_or(0);
            let open = app.list("", "", TaskSort::Smart)?.len();
            let all = app.list("status:all", "", TaskSort::Smart)?.len();
            let info = json!({
                "path": ctx.db,
                "mode": if store.mode() == DbMode::Encrypted { "encrypted" } else { "plain" },
                "size_bytes": size,
                "schema_version": store.schema_version()?,
                "open_tasks": open,
                "all_tasks": all,
                "backups": ctx.backup_dir(),
                "config": ctx.config_path,
            });
            if ctx.json {
                ctx.print_json(&info)?;
            } else {
                for (k, v) in info.as_object().expect("object") {
                    println!(
                        "{k:<15} {}",
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    );
                }
            }
        }
        DbAction::Passwd | DbAction::Encrypt => {
            let encrypting = matches!(action, DbAction::Encrypt);
            let app = ctx.open()?;
            let mode = app.store().mode();
            match (encrypting, mode) {
                (true, DbMode::Encrypted) => {
                    bail!("the database is already encrypted; use `ttask db passwd`")
                }
                (false, DbMode::Plain) => {
                    bail!("the database is not encrypted; use `ttask db encrypt`")
                }
                _ => {}
            }
            // A --password-file unlocked the database; the new password is prompted.
            let new = password::new_password(None, "New password")?;
            let bak = app.into_store().convert(Some(&new))?;
            if ctx.config.security.remember == Remember::Keychain {
                if let Err(e) = keychain::set(&ctx.db, &new) {
                    eprintln!("warning: could not update the keychain: {e}");
                }
            }
            ctx.say(format!(
                "{}. The previous file is kept as {} until `ttask db gc-backups`.",
                if encrypting {
                    "Encrypted the database"
                } else {
                    "Changed the password"
                },
                bak.display()
            ));
        }
        DbAction::Decrypt { yes } => {
            let app = ctx.open()?;
            if app.store().mode() != DbMode::Encrypted {
                bail!("the database is not encrypted");
            }
            if !yes {
                if !std::io::stdin().is_terminal() {
                    bail!("decrypting needs confirmation; pass --yes");
                }
                eprint!("This writes your tasks to disk unencrypted. Type 'decrypt' to continue: ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if answer.trim() != "decrypt" {
                    bail!("cancelled");
                }
            }
            let bak = app.into_store().convert(None)?;
            let _ = keychain::delete(&ctx.db);
            ctx.say(format!(
                "Decrypted the database. The encrypted original is kept as {}.",
                bak.display()
            ));
        }
        DbAction::Backup { path } => {
            let app = ctx.open()?;
            let path = path.unwrap_or_else(|| {
                let stamp = app
                    .clock()
                    .now_zoned()
                    .strftime("%Y%m%d-%H%M%S")
                    .to_string();
                ctx.backup_dir().join(format!("ttask-manual-{stamp}.db"))
            });
            app.store().backup_to(&path)?;
            ctx.say(format!("Backed up to {}", path.display()));
        }
        DbAction::Check => {
            let app = ctx.open()?;
            let report = app.store().check()?;
            if ctx.json {
                ctx.print_json(&report)?;
            } else if report.is_ok() {
                ctx.say("Database OK");
            } else {
                for problem in &report.problems {
                    eprintln!("problem: {problem}");
                }
                eprintln!(
                    "Restore from the newest backup in {}",
                    ctx.backup_dir().display()
                );
            }
            if !report.is_ok() {
                return Ok(ExitCode::from(4));
            }
        }
        DbAction::Gc => gc(ctx)?,
        DbAction::GcBackups => {
            if ttask_store::remove_backup_copy(&ctx.db)? {
                ctx.say("Removed the .bak copy");
            } else {
                ctx.say("No .bak copy to remove");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn gc(ctx: &Ctx) -> anyhow::Result<()> {
    let mut app = ctx.open()?;
    let now = app.now_ms();
    let report = app
        .store_mut()
        .gc(now - PURGE_AFTER_DAYS * DAY_MS, now - HISTORY_DAYS * DAY_MS)?;
    if ctx.json {
        return ctx.print_json(&report);
    }
    ctx.say(format!(
        "Purged {} task(s) and {} project(s); compacted {} history entries",
        report.purged_tasks, report.purged_projects, report.compacted_events
    ));
    Ok(())
}

fn config_cmd(ctx: &Ctx, action: Option<ConfigAction>) -> anyhow::Result<()> {
    let path = ctx
        .config_path
        .clone()
        .ok_or_else(|| anyhow!("cannot determine the config directory; set TTASK_CONFIG"))?;
    match action {
        Some(ConfigAction::Path) => println!("{}", path.display()),
        Some(ConfigAction::Init) => {
            if path.exists() {
                bail!("{} already exists", path.display());
            }
            write_new(&path, config::EXAMPLE.as_bytes())?;
            ctx.say(format!("Wrote {}", path.display()));
        }
        None => {
            println!(
                "config:   {}{}",
                path.display(),
                if path.exists() {
                    ""
                } else {
                    " (not created; `ttask config init`)"
                }
            );
            println!("database: {}", ctx.db.display());
            println!("backups:  {}", ctx.backup_dir().display());
        }
    }
    Ok(())
}
