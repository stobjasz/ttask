//! TUI state and input handling, independent of the terminal so it can be
//! driven by tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use secrecy::SecretString;
use ttask_app::{App, TaskDetail, View, ViewKind, arrange_tree};
use ttask_core::date::parse_date;
use ttask_core::quickadd::{Change, Mods, Wait, is_tag_name};
use ttask_core::{Priority, Task, TaskId};
use ttask_store::{DbMode, StoreError, TaskSort};

use crate::editor::{Editor, NOTES};
use crate::input::LineInput;
use crate::keymap::{Action, Keymap, KeymapName};
use crate::palette::{Item, Palette, Target};
use crate::theme::{DetailPane, Theme, ThemeName};

/// Called with a notification title and body (desktop notifications).
pub type Notifier = Box<dyn Fn(&str, &str)>;
/// Checks a password typed on the idle-lock screen for the database at the path.
pub type Unlocker = Box<dyn Fn(&Path, &SecretString) -> bool>;
/// Opens another database file. Without a password an encrypted file may
/// still open (e.g. from the keychain); on failure the TUI asks for one.
pub type Opener = Box<dyn Fn(&Path, Option<&SecretString>) -> anyhow::Result<App>>;
/// Hands a link to the browser or mail client.
pub type LinkOpener = Box<dyn Fn(&str) -> std::io::Result<()>>;

pub struct Settings {
    pub theme: ThemeName,
    pub keymap: KeymapName,
    pub detail_pane: DetailPane,
    pub keys: BTreeMap<String, String>,
    pub no_color: bool,
    pub encrypted: bool,
    /// Lock after this long without input (encrypted databases only).
    pub idle_lock: Option<Duration>,
    pub unlock: Option<Unlocker>,
    pub notify: Option<Notifier>,
    pub open_db: Option<Opener>,
    pub open_link: Option<LinkOpener>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeName::Auto,
            keymap: KeymapName::Vim,
            detail_pane: DetailPane::Auto,
            keys: BTreeMap::new(),
            no_color: false,
            encrypted: false,
            idle_lock: None,
            unlock: None,
            notify: None,
            open_db: None,
            open_link: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    List,
    Detail,
}

#[derive(Debug, Clone)]
pub enum SidebarEntry {
    Header(&'static str),
    View { view: View, count: u32, depth: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    Add { parent: Option<TaskId> },
    Modify(Vec<TaskId>),
    Due(Vec<TaskId>),
    Move(Vec<TaskId>),
    Tags(Vec<TaskId>),
    Filter,
    Search,
    ChecklistAdd(TaskId),
    SaveFilter,
    Wait(Vec<TaskId>),
    Context,
    OpenDatabase,
    Password(PathBuf),
    Block(TaskId),
    Parent(TaskId),
}

impl PromptKind {
    pub fn label(&self) -> &'static str {
        match self {
            PromptKind::Add { parent: None } => "Add",
            PromptKind::Add { parent: Some(_) } => "Add subtask",
            PromptKind::Modify(_) => "Modify",
            PromptKind::Due(_) => "Due",
            PromptKind::Move(_) => "Move to",
            PromptKind::Tags(_) => "Tags",
            PromptKind::Filter => "Filter",
            PromptKind::Search => "Search",
            PromptKind::ChecklistAdd(_) => "Checklist item",
            PromptKind::SaveFilter => "Save filter as",
            PromptKind::Wait(_) => "Wait until (date, someday, none)",
            PromptKind::Context => "Context (saved filter, none)",
            PromptKind::OpenDatabase => "Open database",
            PromptKind::Password(_) => "Password",
            PromptKind::Block(_) => "Waits on (task ids)",
            PromptKind::Parent(_) => "Subtask of (task id, none)",
        }
    }

    /// Prompts that show a second line under the input.
    pub fn has_preview(&self) -> bool {
        matches!(
            self,
            PromptKind::Add { .. } | PromptKind::Modify(_) | PromptKind::OpenDatabase
        )
    }
}

pub struct Prompt {
    pub kind: PromptKind,
    pub input: LineInput,
    pub error: Option<String>,
    /// Tab-completion candidates and similar help under the input.
    pub hint: Option<String>,
    /// Filter text and view before the prompt opened, restored on Esc
    /// (filtering and searching change them as you type).
    previous_filter: String,
    previous_view: View,
}

pub enum Mode {
    Normal,
    Prompt(Prompt),
    Palette(Palette),
    Confirm {
        message: String,
        ids: Vec<TaskId>,
    },
    Help {
        scroll: u16,
    },
    /// A task's change history: `(when, what)` rows.
    History {
        title: String,
        entries: Vec<(String, String)>,
        scroll: u16,
    },
    /// The in-app task editor.
    Editor(Box<Editor>),
    Locked {
        input: LineInput,
        error: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct StatusLine {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct Tui {
    pub app: App,
    pub settings: Settings,
    pub keymap: Keymap,
    pub theme: Theme,
    pub sidebar: Vec<SidebarEntry>,
    pub sidebar_idx: usize,
    pub current: View,
    pub filter: String,
    pub tasks: Vec<(Task, u8)>,
    pub task_idx: usize,
    pub list_offset: usize,
    pub detail: Option<TaskDetail>,
    pub detail_idx: usize,
    pub detail_full: bool,
    pub focus: Focus,
    pub mode: Mode,
    pub marked: BTreeSet<TaskId>,
    pub status: Option<StatusLine>,
    pub quit: bool,
    pub page: usize,
    pub width: u16,
    pub timer: Option<(ttask_app::TimeEntry, Task)>,
    /// Name of the active context, shown in the header.
    pub context: Option<String>,
    /// `Ctrl-S` replaces the view's sort until another view opens.
    pub sort_override: Option<TaskSort>,
    editor_request: Option<TaskId>,
    clipboard_request: Option<String>,
    data_version: i64,
    last_activity: Instant,
    last_poll: Instant,
    last_reminders: Option<Instant>,
}

const POLL_EVERY: Duration = Duration::from_secs(2);
const REMINDERS_EVERY: Duration = Duration::from_secs(30);
const STATUS_FOR: Duration = Duration::from_secs(5);

impl Tui {
    pub fn new(app: App, settings: Settings) -> anyhow::Result<Tui> {
        let keymap = Keymap::new(settings.keymap, &settings.keys).map_err(anyhow::Error::msg)?;
        let theme = Theme::new(settings.theme, settings.no_color);
        let now = Instant::now();
        let mut tui = Tui {
            app,
            settings,
            keymap,
            theme,
            sidebar: Vec::new(),
            sidebar_idx: 0,
            current: View::today(),
            filter: String::new(),
            tasks: Vec::new(),
            task_idx: 0,
            list_offset: 0,
            detail: None,
            detail_idx: 0,
            detail_full: false,
            focus: Focus::List,
            mode: Mode::Normal,
            marked: BTreeSet::new(),
            status: None,
            quit: false,
            page: 10,
            width: 120,
            timer: None,
            context: None,
            sort_override: None,
            editor_request: None,
            clipboard_request: None,
            data_version: 0,
            last_activity: now,
            last_poll: now,
            last_reminders: None,
        };
        tui.reload();
        if let Some(i) = tui.sidebar_position(&View::today()) {
            tui.sidebar_idx = i;
        }
        Ok(tui)
    }

    // ------------------------------------------------------------- data

    /// Reloads sidebar, list and detail, keeping the selection by task id.
    pub fn reload(&mut self) {
        if let Err(e) = self.try_reload() {
            self.error(e.to_string());
        }
    }

    fn try_reload(&mut self) -> anyhow::Result<()> {
        self.sidebar = build_sidebar(&self.app)?;
        // Keep the current view (its counts and name may have changed).
        if let Some(i) = self.sidebar_position(&self.current) {
            self.sidebar_idx = i;
            if let SidebarEntry::View { view, .. } = &self.sidebar[i] {
                self.current = view.clone();
            }
        }
        let selected_id = self.selected_task().map(|t| t.id);
        let view = View {
            sort: self.sort(),
            ..self.current.clone()
        };
        let tasks = self.app.view_tasks(&view, &self.filter)?;
        self.tasks = arrange_tree(tasks);
        self.task_idx = selected_id
            .and_then(|id| self.tasks.iter().position(|(t, _)| t.id == id))
            .unwrap_or(self.task_idx)
            .min(self.tasks.len().saturating_sub(1));
        let live: BTreeSet<TaskId> = self.tasks.iter().map(|(t, _)| t.id).collect();
        self.marked.retain(|id| live.contains(id));
        self.load_detail();
        self.timer = self.app.running_timer()?;
        self.context = self.app.active_context()?.map(|(name, _)| name);
        self.data_version = self.app.store().data_version()?;
        Ok(())
    }

    fn load_detail(&mut self) {
        self.detail = self
            .selected_task()
            .map(|t| t.id)
            .and_then(|id| self.app.detail(id).ok());
        let items = self.detail_items_len();
        self.detail_idx = self.detail_idx.min(items.saturating_sub(1));
    }

    fn sidebar_position(&self, view: &View) -> Option<usize> {
        self.sidebar.iter().position(|e| match e {
            SidebarEntry::View { view: v, .. } => match (&v.kind, &view.kind) {
                (ViewKind::Builtin, ViewKind::Builtin) => v.name == view.name,
                (a, b) => a == b,
            },
            SidebarEntry::Header(_) => false,
        })
    }

    /// The sort in effect: the override, else the view's.
    pub fn sort(&self) -> TaskSort {
        self.sort_override.unwrap_or(self.current.sort)
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.tasks.get(self.task_idx).map(|(t, _)| t)
    }

    /// Marked tasks, or the selected one.
    fn targets(&self) -> Vec<TaskId> {
        if self.marked.is_empty() {
            self.selected_task().map(|t| vec![t.id]).unwrap_or_default()
        } else {
            self.marked.iter().copied().collect()
        }
    }

    pub fn detail_visible(&self) -> bool {
        match self.settings.detail_pane {
            DetailPane::Always => true,
            DetailPane::Never => false,
            DetailPane::Auto => self.width >= 100,
        }
    }

    /// Checklist items, then subtasks.
    pub fn detail_items_len(&self) -> usize {
        self.detail
            .as_ref()
            .map(|d| d.checklist.len() + d.subtasks.len())
            .unwrap_or(0)
    }

    pub fn ok(&mut self, text: impl Into<String>) {
        self.status = Some(StatusLine {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Some(StatusLine {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    /// Runs a mutation, reports errors in the status line and reloads.
    fn mutate(&mut self, f: impl FnOnce(&mut App) -> Result<String, ttask_app::AppError>) {
        match f(&mut self.app) {
            Ok(message) => {
                if !message.is_empty() {
                    self.ok(message);
                }
            }
            Err(e) => self.error(e.to_string()),
        }
        self.reload();
    }

    // ------------------------------------------------------------ timers

    /// Periodic work: expire the status line, pick up external changes,
    /// fire reminders, lock when idle.
    pub fn tick(&mut self, now: Instant) {
        if self
            .status
            .as_ref()
            .is_some_and(|s| now.duration_since(s.at) > STATUS_FOR)
        {
            self.status = None;
        }
        if matches!(self.mode, Mode::Locked { .. }) {
            return;
        }
        if let Some(idle) = self.settings.idle_lock {
            if self.settings.encrypted && now.duration_since(self.last_activity) >= idle {
                self.lock();
                return;
            }
        }
        if now.duration_since(self.last_poll) >= POLL_EVERY {
            self.last_poll = now;
            if let Ok(version) = self.app.store().data_version() {
                if version != self.data_version {
                    self.reload();
                }
            }
        }
        if self
            .last_reminders
            .is_none_or(|t| now.duration_since(t) >= REMINDERS_EVERY)
        {
            self.last_reminders = Some(now);
            self.fire_reminders();
        }
    }

    fn fire_reminders(&mut self) {
        match self.app.take_due_reminders() {
            Ok(fired) if !fired.is_empty() => {
                for (_, task) in &fired {
                    if let Some(notify) = &self.settings.notify {
                        notify("ttask reminder", &task.title);
                    }
                }
                let titles: Vec<&str> = fired.iter().map(|(_, t)| t.title.as_str()).collect();
                self.ok(format!("Reminder: {}", titles.join(", ")));
                self.reload();
            }
            Ok(_) => {}
            Err(e) => self.error(e.to_string()),
        }
    }

    /// Hides all task data until the password is typed again.
    pub fn lock(&mut self) {
        self.tasks.clear();
        self.detail = None;
        self.sidebar.clear();
        self.mode = Mode::Locked {
            input: LineInput::masked(),
            error: None,
        };
    }

    pub fn take_editor_request(&mut self) -> Option<TaskId> {
        self.editor_request.take()
    }

    /// Text to put on the clipboard (the terminal does it, via OSC 52).
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.clipboard_request.take()
    }

    /// Applies notes returned from `$EDITOR`.
    pub fn finish_notes(&mut self, id: TaskId, result: std::io::Result<String>) {
        match result {
            Ok(text) => self.mutate(|app| {
                app.set_notes(id, &text)?;
                Ok("Notes saved".into())
            }),
            Err(e) => self.error(e.to_string()),
        }
    }

    // ------------------------------------------------------------- input

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.last_activity = Instant::now();
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = match mode {
            Mode::Normal => {
                if let Some(action) = self.keymap.action(&key) {
                    self.dispatch(action);
                }
                // dispatch may have switched modes.
                std::mem::replace(&mut self.mode, Mode::Normal)
            }
            Mode::Prompt(prompt) => self.prompt_key(prompt, key),
            Mode::Palette(palette) => self.palette_key(palette, key),
            Mode::Editor(editor) => self.editor_key(editor, key),
            Mode::Confirm { message, ids } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let n = ids.len();
                    self.mutate(|app| {
                        app.delete(&ids)?;
                        Ok(format!("Deleted {n} task{} (u to undo)", plural(n)))
                    });
                    Mode::Normal
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                    Mode::Normal
                }
                _ => Mode::Confirm { message, ids },
            },
            Mode::Help { scroll } => match self.scroll_key(&key, scroll) {
                Some(scroll) => Mode::Help { scroll },
                None => Mode::Normal,
            },
            Mode::History {
                title,
                entries,
                scroll,
            } => match self.scroll_key(&key, scroll) {
                Some(scroll) => Mode::History {
                    title,
                    entries,
                    scroll,
                },
                None => Mode::Normal,
            },
            Mode::Locked { mut input, error } => match key.code {
                KeyCode::Enter => {
                    let password = SecretString::from(input.text());
                    let path = self.app.store().path().to_owned();
                    let ok = self
                        .settings
                        .unlock
                        .as_ref()
                        .is_some_and(|f| f(&path, &password));
                    if ok {
                        self.mode = Mode::Normal;
                        self.reload();
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    } else {
                        Mode::Locked {
                            input: LineInput::masked(),
                            error: Some("Wrong password".into()),
                        }
                    }
                }
                _ => {
                    input.handle(key);
                    Mode::Locked { input, error }
                }
            },
        };
    }

    fn editor_key(&mut self, mut editor: Box<Editor>, key: KeyEvent) -> Mode {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('s') if ctrl => return self.save_editor(editor),
            KeyCode::Esc => {
                if editor.is_dirty() && !editor.confirm_discard {
                    editor.confirm_discard = true;
                    editor.error =
                        Some("Unsaved changes: Esc again discards them, Ctrl-S saves".into());
                    return Mode::Editor(editor);
                }
                return Mode::Normal;
            }
            KeyCode::Tab => editor.move_focus(1),
            KeyCode::BackTab => editor.move_focus(-1),
            _ if editor.focus == NOTES => {
                if !editor.notes.handle(key) && key.code == KeyCode::Up {
                    editor.move_focus(-1);
                }
            }
            KeyCode::Up => editor.move_focus(-1),
            KeyCode::Down => editor.move_focus(1),
            KeyCode::Enter => return self.save_editor(editor),
            _ => {
                let focus = editor.focus;
                editor.inputs[focus].handle(key);
            }
        }
        editor.confirm_discard = false;
        if editor
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("Unsaved"))
        {
            editor.error = None;
        }
        Mode::Editor(editor)
    }

    /// Applies the editor's changes as one undoable edit.
    fn save_editor(&mut self, mut editor: Box<Editor>) -> Mode {
        let mods = match editor.form().changes(editor.original(), self.app.today()) {
            Ok(mods) => mods,
            Err(e) => {
                editor.error = Some(e);
                editor.focus_error();
                return Mode::Editor(editor);
            }
        };
        if mods.is_empty() {
            return Mode::Normal;
        }
        match self.app.edit_with(&[editor.task_id], &mods) {
            Ok(_) => {
                self.reload();
                self.ok("Saved (u to undo)");
                Mode::Normal
            }
            Err(e) => {
                editor.error = Some(e.to_string());
                Mode::Editor(editor)
            }
        }
    }

    /// New scroll position for a key in a scrollable popup; `None` closes it.
    fn scroll_key(&self, key: &KeyEvent, scroll: u16) -> Option<u16> {
        match self.keymap.action(key) {
            Some(Action::Down) => Some(scroll + 1),
            Some(Action::Up) => Some(scroll.saturating_sub(1)),
            Some(Action::PageDown) => Some(scroll + 10),
            Some(Action::PageUp) => Some(scroll.saturating_sub(10)),
            _ => None,
        }
    }

    fn prompt_key(&mut self, mut prompt: Prompt, key: KeyEvent) -> Mode {
        match key.code {
            KeyCode::Esc => {
                match prompt.kind {
                    PromptKind::Filter => {
                        self.filter = prompt.previous_filter.clone();
                        self.reload();
                    }
                    PromptKind::Search if self.current != prompt.previous_view => {
                        self.open_view(prompt.previous_view.clone());
                        self.filter = prompt.previous_filter.clone();
                        self.reload();
                    }
                    _ => {}
                }
                Mode::Normal
            }
            KeyCode::Enter => self.submit(prompt),
            KeyCode::Tab if prompt.kind == PromptKind::OpenDatabase => {
                prompt.hint = complete_path(&mut prompt.input);
                prompt.error = None;
                Mode::Prompt(prompt)
            }
            KeyCode::Tab if matches!(prompt.kind, PromptKind::Move(_)) => {
                let paths: Vec<String> = self
                    .app
                    .projects(false)
                    .map(|ps| ps.into_iter().map(|p| p.path).collect())
                    .unwrap_or_default();
                complete_from(&paths, &mut prompt.input);
                Mode::Prompt(prompt)
            }
            KeyCode::Tab if prompt.kind == PromptKind::Context => {
                let names: Vec<String> = self
                    .app
                    .saved_filters()
                    .map(|fs| fs.into_iter().map(|f| f.name).collect())
                    .unwrap_or_default();
                complete_from(&names, &mut prompt.input);
                Mode::Prompt(prompt)
            }
            _ => {
                if prompt.input.handle(key) {
                    prompt.error = None;
                    prompt.hint = None;
                    if prompt.kind == PromptKind::Search {
                        // Results as you type; Esc goes back.
                        let text = prompt.input.text();
                        if text.trim().is_empty() {
                            if self.current != prompt.previous_view {
                                self.open_view(prompt.previous_view.clone());
                            }
                        } else {
                            self.show_search(&text);
                        }
                    }
                    if prompt.kind == PromptKind::Filter {
                        // Filter as you type, while the text parses.
                        let text = prompt.input.text();
                        match ttask_core::filter::parse_filter(&text, self.app.today()) {
                            Ok(_) => {
                                self.filter = text;
                                self.task_idx = 0;
                                self.reload();
                            }
                            Err(e) => prompt.error = Some(e.message),
                        }
                    }
                }
                Mode::Prompt(prompt)
            }
        }
    }

    fn palette_key(&mut self, mut palette: Palette, key: KeyEvent) -> Mode {
        match key.code {
            KeyCode::Esc => Mode::Normal,
            KeyCode::Up => {
                palette.move_selection(-1);
                Mode::Palette(palette)
            }
            KeyCode::Down | KeyCode::Tab => {
                palette.move_selection(1);
                Mode::Palette(palette)
            }
            KeyCode::Enter => {
                match palette.selected_item().map(|i| i.target.clone()) {
                    Some(Target::Action(action)) => {
                        self.dispatch(action);
                        return std::mem::replace(&mut self.mode, Mode::Normal);
                    }
                    Some(Target::View(view)) => self.open_view(view),
                    Some(Target::Context(name)) => self.switch_context(name.as_deref()),
                    Some(Target::Link(link)) => self.open_link(&link),
                    None => {}
                }
                Mode::Normal
            }
            _ => {
                if palette.input.handle(key) {
                    palette.refilter();
                }
                Mode::Palette(palette)
            }
        }
    }

    fn open_prompt(&mut self, kind: PromptKind, initial: &str) {
        let mut input = LineInput::new(initial);
        input.masked = matches!(kind, PromptKind::Password(_));
        self.mode = Mode::Prompt(Prompt {
            kind,
            input,
            error: None,
            hint: None,
            previous_filter: self.filter.clone(),
            previous_view: self.current.clone(),
        });
    }

    /// Shows full-text results for `text` over every task, open or not.
    fn show_search(&mut self, text: &str) {
        let query = format!("status:all wait:any text:\"{}\"", text.replace('"', ""));
        self.open_view(View {
            name: format!("Search: {}", text.trim()),
            query,
            sort: TaskSort::Smart,
            kind: ViewKind::Search,
        });
    }

    /// Default project for quick-add: the project being viewed.
    fn current_project(&self) -> Option<i64> {
        match self.current.kind {
            ViewKind::Project(id) => Some(id),
            _ => None,
        }
    }

    pub fn open_view(&mut self, view: View) {
        self.current = view;
        self.sort_override = None;
        self.filter.clear();
        self.marked.clear();
        self.task_idx = 0;
        self.list_offset = 0;
        self.detail_full = false;
        self.reload();
    }

    fn submit(&mut self, mut prompt: Prompt) -> Mode {
        let text = prompt.input.text();
        let today = self.app.today();
        let result: Result<Option<String>, String> = match &prompt.kind {
            PromptKind::Add { parent } => {
                let project = self.current_project();
                match self.app.add(&text, project, *parent) {
                    Ok(task) => {
                        let id = task.id;
                        self.reload();
                        if let Some(i) = self.tasks.iter().position(|(t, _)| t.id == id) {
                            self.task_idx = i;
                            self.load_detail();
                        }
                        // Stay open for rapid entry.
                        self.ok(format!("Added {}: {}", task.id, task.title));
                        prompt.input.clear();
                        prompt.error = None;
                        return Mode::Prompt(prompt);
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            PromptKind::Modify(ids) => self
                .app
                .edit(ids, &text)
                .map(|t| Some(format!("Updated {} task{}", t.len(), plural(t.len()))))
                .map_err(|e| e.to_string()),
            PromptKind::Due(ids) => {
                let change = if text.trim().is_empty() || text.trim().eq_ignore_ascii_case("none") {
                    Ok(Change::Clear)
                } else {
                    parse_date(&text.replace(' ', "@"), today)
                        .or_else(|_| parse_date(&text, today))
                        .map(Change::Set)
                        .map_err(|e| e.to_string())
                };
                change.and_then(|due| {
                    let mods = Mods {
                        due: Some(due),
                        ..Mods::default()
                    };
                    self.app
                        .edit_with(ids, &mods)
                        .map(|_| None)
                        .map_err(|e| e.to_string())
                })
            }
            PromptKind::Move(ids) => {
                let path = text.trim().trim_start_matches('#');
                let change = if path.is_empty() || path.eq_ignore_ascii_case("inbox") {
                    Change::Clear
                } else {
                    Change::Set(path.split('/').map(|s| s.trim().to_owned()).collect())
                };
                let mods = Mods {
                    project: Some(change),
                    ..Mods::default()
                };
                self.app
                    .edit_with(ids, &mods)
                    .map(|_| None)
                    .map_err(|e| e.to_string())
            }
            PromptKind::Tags(ids) => {
                let wanted: Vec<String> = text
                    .split([' ', ','])
                    .map(|t| t.trim().trim_start_matches(['+', '#']).to_owned())
                    .filter(|t| !t.is_empty())
                    .collect();
                if let Some(bad) = wanted.iter().find(|t| !is_tag_name(t)) {
                    Err(format!("invalid tag `{bad}`"))
                } else {
                    let ids = ids.clone();
                    let mut result = Ok(None);
                    for id in &ids {
                        let current = self.app.task(*id).map(|t| t.tags).unwrap_or_default();
                        let mods = Mods {
                            add_tags: wanted.clone(),
                            remove_tags: current
                                .into_iter()
                                .filter(|t| !wanted.iter().any(|w| w.eq_ignore_ascii_case(t)))
                                .collect(),
                            ..Mods::default()
                        };
                        if let Err(e) = self.app.edit_with(&[*id], &mods) {
                            result = Err(e.to_string());
                            break;
                        }
                    }
                    result
                }
            }
            PromptKind::Filter => match ttask_core::filter::parse_filter(&text, today) {
                Ok(_) => {
                    self.filter = text;
                    Ok(None)
                }
                Err(e) => Err(e.message),
            },
            PromptKind::Search => {
                if !text.trim().is_empty() {
                    if self.current.kind != ViewKind::Search {
                        self.show_search(&text);
                    }
                    self.focus = Focus::List;
                    self.detail_full = false;
                }
                Ok(None)
            }
            PromptKind::OpenDatabase => {
                let path = expand_home(text.trim());
                if text.trim().is_empty() {
                    Err("type the path of a ttask database (Tab completes)".into())
                } else {
                    match self.start_open(&path) {
                        Ok(Some(next)) => return next,
                        Ok(None) => Ok(None),
                        Err(e) => Err(e),
                    }
                }
            }
            PromptKind::Password(path) => {
                let password = SecretString::from(text);
                let path = path.clone();
                match self.open_with(&path, Some(&password)) {
                    Ok(()) => Ok(None),
                    Err(e) => {
                        prompt.input.clear();
                        Err(e)
                    }
                }
            }
            PromptKind::Block(id) => self.set_blockers(*id, &text),
            PromptKind::Parent(id) => {
                let trimmed = text.trim().trim_start_matches('#');
                let parent = if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                    Ok(None)
                } else {
                    trimmed
                        .parse::<TaskId>()
                        .map(Some)
                        .map_err(|_| format!("`{trimmed}` is not a task id"))
                };
                parent.and_then(|parent| {
                    self.app
                        .set_parent(*id, parent)
                        .map(|_| {
                            Some(match parent {
                                Some(p) => format!("Now a subtask of {p}"),
                                None => "Now a top-level task".into(),
                            })
                        })
                        .map_err(|e| e.to_string())
                })
            }
            PromptKind::ChecklistAdd(id) => match self.app.checklist_add(*id, &text) {
                Ok(_) => {
                    self.reload();
                    prompt.input.clear();
                    prompt.error = None;
                    return Mode::Prompt(prompt);
                }
                Err(e) => Err(e.to_string()),
            },
            PromptKind::Wait(ids) => {
                let trimmed = text.trim();
                let change = match trimmed.to_ascii_lowercase().as_str() {
                    "" | "none" => Ok(Change::Clear),
                    "someday" => Ok(Change::Set(Wait::Someday)),
                    _ => parse_date(trimmed, today)
                        .map(|spec| Change::Set(Wait::Until(spec.date)))
                        .map_err(|e| e.to_string()),
                };
                change.and_then(|wait| {
                    let mods = Mods {
                        wait: Some(wait),
                        ..Mods::default()
                    };
                    self.app
                        .edit_with(ids, &mods)
                        .map(|t| {
                            Some(format!(
                                "{} task{} {}",
                                t.len(),
                                plural(t.len()),
                                if trimmed.is_empty() || trimmed == "none" {
                                    "back in the lists"
                                } else {
                                    "parked"
                                }
                            ))
                        })
                        .map_err(|e| e.to_string())
                })
            }
            PromptKind::Context => {
                let name = text.trim();
                let wanted =
                    (!name.is_empty() && !name.eq_ignore_ascii_case("none")).then_some(name);
                match self.app.set_context(wanted) {
                    Ok(Some(name)) => Ok(Some(format!("Context: {name}"))),
                    Ok(None) => Ok(Some("Context cleared".into())),
                    Err(e) => Err(e.to_string()),
                }
            }
            PromptKind::SaveFilter => {
                let query = combine(&self.current.query, &self.filter);
                self.app
                    .save_filter(&text, &query)
                    .map(|_| Some(format!("Saved filter `{}`", text.trim())))
                    .map_err(|e| e.to_string())
            }
        };
        match result {
            Ok(message) => {
                self.reload();
                if let Some(message) = message {
                    self.ok(message);
                }
                Mode::Normal
            }
            Err(e) => {
                prompt.error = Some(e);
                Mode::Prompt(prompt)
            }
        }
    }

    // ----------------------------------------------------------- actions

    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::PageUp => self.move_cursor(-(self.page.max(1) as isize)),
            Action::PageDown => self.move_cursor(self.page.max(1) as isize),
            Action::Top => self.move_cursor(isize::MIN / 2),
            Action::Bottom => self.move_cursor(isize::MAX / 2),
            Action::FocusNext | Action::FocusRight => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::List,
                    Focus::List if self.detail_visible() && self.detail.is_some() => Focus::Detail,
                    Focus::List => Focus::List,
                    Focus::Detail if action == Action::FocusNext => Focus::Sidebar,
                    Focus::Detail => Focus::Detail,
                };
            }
            Action::FocusPrev | Action::FocusLeft => {
                self.focus = match self.focus {
                    Focus::Detail => Focus::List,
                    Focus::List => Focus::Sidebar,
                    Focus::Sidebar if action == Action::FocusPrev && self.detail_visible() => {
                        Focus::Detail
                    }
                    Focus::Sidebar => Focus::Sidebar,
                };
                if self.detail_full {
                    self.detail_full = false;
                    self.focus = Focus::List;
                }
            }
            Action::Open => match self.focus {
                Focus::Sidebar => self.focus = Focus::List,
                Focus::List | Focus::Detail => {
                    if self.detail.is_some() {
                        self.detail_full = !self.detail_full;
                    }
                }
            },
            Action::Back => {
                if self.detail_full {
                    self.detail_full = false;
                } else if !self.marked.is_empty() {
                    self.marked.clear();
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.reload();
                } else if self.current.kind == ViewKind::Search {
                    let view = self
                        .sidebar_view(self.sidebar_idx)
                        .unwrap_or_else(View::today);
                    self.open_view(view);
                } else if self.focus == Focus::Detail {
                    self.focus = Focus::List;
                }
            }
            Action::Add => self.open_prompt(PromptKind::Add { parent: None }, ""),
            Action::AddSubtask => {
                if let Some(task) = self.selected_task() {
                    let parent = task.parent_id.unwrap_or(task.id);
                    self.open_prompt(
                        PromptKind::Add {
                            parent: Some(parent),
                        },
                        "",
                    );
                }
            }
            Action::Edit => {
                if let Some(task) = self.selected_task() {
                    self.mode = Mode::Editor(Box::new(Editor::new(task)));
                }
            }
            Action::EditNotes => self.editor_request = self.selected_task().map(|t| t.id),
            Action::Modify => {
                let ids = self.targets();
                if !ids.is_empty() {
                    self.open_prompt(PromptKind::Modify(ids), "");
                }
            }
            Action::ToggleDone => self.toggle_done(),
            Action::Cancel => {
                let ids = self.targets();
                if !ids.is_empty() {
                    self.mutate(|app| {
                        app.cancel(&ids)?;
                        Ok(format!("Cancelled {} task{}", ids.len(), plural(ids.len())))
                    });
                }
            }
            Action::Priority(p) => {
                let ids = self.targets();
                if !ids.is_empty() {
                    let priority = Priority::new(p).unwrap_or_default();
                    self.mutate(|app| {
                        app.set_priority(&ids, priority)?;
                        Ok(format!("Priority {}", priority.name()))
                    });
                }
            }
            Action::Due => {
                let ids = self.targets();
                let initial = self
                    .selected_task()
                    .and_then(|t| {
                        t.due_date.map(|d| match t.due_time {
                            Some(time) => format!("{d}@{}", ttask_core::date::format_time(time)),
                            None => d.to_string(),
                        })
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    self.open_prompt(PromptKind::Due(ids), &initial);
                }
            }
            Action::Move => {
                let ids = self.targets();
                let initial = self
                    .selected_task()
                    .and_then(|t| t.project.clone())
                    .unwrap_or_default();
                if !ids.is_empty() {
                    self.open_prompt(PromptKind::Move(ids), &initial);
                }
            }
            Action::Tags => {
                let ids = self.targets();
                let initial = self
                    .selected_task()
                    .map(|t| {
                        t.tags
                            .iter()
                            .map(|t| format!("+{t}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    let initial = if initial.is_empty() {
                        initial
                    } else {
                        initial + " "
                    };
                    self.open_prompt(PromptKind::Tags(ids), &initial);
                }
            }
            Action::Filter => {
                let current = self.filter.clone();
                self.open_prompt(PromptKind::Filter, &current);
            }
            Action::Search => self.open_prompt(PromptKind::Search, ""),
            Action::Palette => self.mode = Mode::Palette(Palette::new(self.palette_items())),
            Action::Undo => self.mutate(|app| {
                Ok(match app.undo()? {
                    Some(o) => format!("Undid {}: {}", o.kinds.join(", "), o.titles.join(", ")),
                    None => "Nothing to undo".into(),
                })
            }),
            Action::Redo => self.mutate(|app| {
                Ok(match app.redo()? {
                    Some(o) => format!("Redid {}: {}", o.kinds.join(", "), o.titles.join(", ")),
                    None => "Nothing to redo".into(),
                })
            }),
            Action::Mark => {
                if let Some(id) = self.selected_task().map(|t| t.id) {
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                    self.move_cursor(1);
                }
            }
            Action::ReorderUp | Action::ReorderDown => self.reorder(action == Action::ReorderUp),
            Action::Delete => {
                let ids = self.targets();
                if let Some(task) = self.selected_task() {
                    let message = if ids.len() == 1 {
                        format!("Delete \"{}\"?", task.title)
                    } else {
                        format!("Delete {} tasks?", ids.len())
                    };
                    self.mode = Mode::Confirm { message, ids };
                }
            }
            Action::Skip => {
                if let Some(id) = self.selected_task().map(|t| t.id) {
                    self.mutate(|app| {
                        let t = app.skip(id)?;
                        Ok(format!(
                            "Skipped to {}",
                            t.due_date.map(|d| d.to_string()).unwrap_or_default()
                        ))
                    });
                }
            }
            Action::ChecklistAdd => {
                if let Some(id) = self.selected_task().map(|t| t.id) {
                    self.open_prompt(PromptKind::ChecklistAdd(id), "");
                }
            }
            Action::ToggleItem => {
                if self.focus == Focus::Detail || self.detail_full {
                    self.toggle_detail_item();
                } else {
                    self.dispatch(Action::Mark);
                }
            }
            Action::Timer => {
                let running_here = self
                    .timer
                    .as_ref()
                    .zip(self.selected_task())
                    .is_some_and(|((_, t), sel)| t.id == sel.id);
                if running_here {
                    self.mutate(|app| {
                        let entry = app.stop_timer()?;
                        Ok(entry
                            .map(|e| format!("Timer stopped: {} min", e.minutes(app.now_ms())))
                            .unwrap_or_default())
                    });
                } else if let Some(id) = self.selected_task().map(|t| t.id) {
                    self.mutate(|app| {
                        app.start_timer(id)?;
                        Ok("Timer started".into())
                    });
                }
            }
            Action::SaveFilter => self.open_prompt(PromptKind::SaveFilter, ""),
            Action::Wait => {
                let ids = self.targets();
                let initial = self
                    .selected_task()
                    .map(|t| {
                        if t.someday {
                            "someday".to_owned()
                        } else {
                            t.wait_until.map(|d| d.to_string()).unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    self.open_prompt(PromptKind::Wait(ids), &initial);
                }
            }
            Action::Context => {
                let current = self.context.clone().unwrap_or_default();
                self.open_prompt(PromptKind::Context, &current);
            }
            Action::OpenLink => {
                let Some(id) = self.selected_task().map(|t| t.id) else {
                    return;
                };
                match self.app.links(id) {
                    Ok(links) if links.is_empty() => self.error("No links in this task"),
                    Ok(links) if links.len() == 1 => self.open_link(&links[0]),
                    Ok(links) => {
                        let items = links
                            .into_iter()
                            .enumerate()
                            .map(|(i, link)| Item {
                                label: link.clone(),
                                hint: format!("link {}", i + 1),
                                target: Target::Link(link),
                            })
                            .collect();
                        self.mode = Mode::Palette(Palette::titled("Open link", items));
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }
            Action::OpenDatabase => {
                let current = self.app.store().path();
                let initial = current
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
                    .unwrap_or_default();
                self.open_prompt(PromptKind::OpenDatabase, &initial);
            }
            Action::Block => {
                if let Some(id) = self.selected_task().map(|t| t.id) {
                    let initial = self
                        .detail
                        .as_ref()
                        .filter(|d| d.task.id == id)
                        .map(|d| {
                            d.blocked_by
                                .iter()
                                .map(|b| format!("{} ", b.id))
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    self.open_prompt(PromptKind::Block(id), &initial);
                }
            }
            Action::Parent => {
                if let Some(task) = self.selected_task() {
                    let (id, parent) = (task.id, task.parent_id);
                    let initial = parent.map(|p| p.to_string()).unwrap_or_default();
                    self.open_prompt(PromptKind::Parent(id), &initial);
                }
            }
            Action::History => {
                if let Some(task) = self.selected_task().cloned() {
                    match self.app.history(task.id) {
                        Ok(history) => {
                            let tz = self.app.clock().time_zone();
                            let entries = history
                                .into_iter()
                                .map(|e| {
                                    let at = ttask_core::jiff::Timestamp::from_millisecond(e.at)
                                        .map(|ts| {
                                            ts.to_zoned(tz.clone())
                                                .strftime("%Y-%m-%d %H:%M")
                                                .to_string()
                                        })
                                        .unwrap_or_default();
                                    let what = if e.changed.is_empty() {
                                        e.kind
                                    } else {
                                        format!("{}: {}", e.kind, e.changed.join(", "))
                                    };
                                    (at, what)
                                })
                                .collect();
                            self.mode = Mode::History {
                                title: task.title,
                                entries,
                                scroll: 0,
                            };
                        }
                        Err(e) => self.error(e.to_string()),
                    }
                }
            }
            Action::CopyTitle | Action::CopyId => {
                let text = self.selected_task().map(|t| {
                    if action == Action::CopyTitle {
                        t.title.clone()
                    } else {
                        t.id.to_string()
                    }
                });
                if let Some(text) = text {
                    self.ok(format!("Copied: {text}"));
                    self.clipboard_request = Some(text);
                }
            }
            Action::Sort => {
                const CYCLE: [TaskSort; 6] = [
                    TaskSort::Smart,
                    TaskSort::Urgency,
                    TaskSort::Priority,
                    TaskSort::Title,
                    TaskSort::Created,
                    TaskSort::Manual,
                ];
                let current = self.sort();
                let next = CYCLE
                    .iter()
                    .position(|s| *s == current)
                    .map_or(CYCLE[0], |i| CYCLE[(i + 1) % CYCLE.len()]);
                self.sort_override = (next != self.current.sort).then_some(next);
                self.reload();
                self.ok(format!("Sorted by {}", sort_name(next)));
            }
            Action::Reload => {
                self.reload();
                self.ok("Reloaded");
            }
            Action::Lock => {
                if self.settings.encrypted && self.settings.unlock.is_some() {
                    self.lock();
                } else {
                    self.error("Only an encrypted database can be locked");
                }
            }
            Action::Help => self.mode = Mode::Help { scroll: 0 },
            Action::Quit => self.quit = true,
        }
    }

    fn open_link(&mut self, link: &str) {
        let result = match &self.settings.open_link {
            Some(open) => open(link).map_err(|e| e.to_string()),
            None => Err("opening links is not available here".into()),
        };
        match result {
            Ok(()) => self.ok(format!("Opened {link}")),
            Err(e) => self.error(e),
        }
    }

    /// Checks `path` and opens it, or returns the password prompt to show
    /// for an encrypted file the opener could not unlock on its own.
    fn start_open(&mut self, path: &Path) -> Result<Option<Mode>, String> {
        if self.settings.open_db.is_none() {
            return Err("opening other databases is not available here".into());
        }
        if same_file(path, self.app.store().path()) {
            return Err("that database is already open".into());
        }
        match ttask_store::inspect(path) {
            Err(StoreError::Missing(_)) => Err(format!("no file at {}", path.display())),
            Err(e) => Err(e.to_string()),
            Ok(DbMode::Plain) => self.open_with(path, None).map(|()| None),
            Ok(DbMode::Encrypted) => match self.open_with(path, None) {
                Ok(()) => Ok(None),
                Err(_) => {
                    self.open_prompt(PromptKind::Password(path.to_owned()), "");
                    Ok(Some(std::mem::replace(&mut self.mode, Mode::Normal)))
                }
            },
        }
    }

    /// Replaces the open database with the one at `path`.
    fn open_with(&mut self, path: &Path, password: Option<&SecretString>) -> Result<(), String> {
        let open = self
            .settings
            .open_db
            .as_ref()
            .ok_or("opening other databases is not available here")?;
        let app = open(path, password).map_err(|e| {
            match e.chain().find_map(|c| c.downcast_ref::<StoreError>()) {
                Some(StoreError::WrongPassword | StoreError::NotADatabase) => {
                    "Wrong password".to_owned()
                }
                _ => format!("{e:#}"),
            }
        })?;
        self.settings.encrypted = app.store().mode() == DbMode::Encrypted;
        self.app = app;
        self.marked.clear();
        self.focus = Focus::List;
        self.last_reminders = None;
        self.data_version = -1;
        self.open_view(View::today());
        if let Some(i) = self.sidebar_position(&View::today()) {
            self.sidebar_idx = i;
        }
        self.ok(format!("Opened {}", path.display()));
        Ok(())
    }

    /// Makes `id` wait on exactly the task ids in `text`.
    fn set_blockers(&mut self, id: TaskId, text: &str) -> Result<Option<String>, String> {
        let wanted = text
            .split([' ', ','])
            .map(|s| s.trim().trim_start_matches('#'))
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<TaskId>()
                    .map_err(|_| format!("`{s}` is not a task id"))
            })
            .collect::<Result<BTreeSet<TaskId>, String>>()?;
        let current: BTreeSet<TaskId> = self
            .app
            .detail(id)
            .map_err(|e| e.to_string())?
            .blocked_by
            .iter()
            .map(|t| t.id)
            .collect();
        for blocker in wanted.difference(&current) {
            self.app.block(id, *blocker).map_err(|e| e.to_string())?;
        }
        for blocker in current.difference(&wanted) {
            self.app.unblock(id, *blocker).map_err(|e| e.to_string())?;
        }
        Ok(Some(if wanted.is_empty() {
            "Waits on nothing".into()
        } else {
            let ids: Vec<String> = wanted.iter().map(|b| format!("#{b}")).collect();
            format!("Waits on {}", ids.join(" "))
        }))
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Focus::Sidebar => {
                let selectable: Vec<usize> = self
                    .sidebar
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| matches!(e, SidebarEntry::View { .. }))
                    .map(|(i, _)| i)
                    .collect();
                let Some(pos) = selectable
                    .iter()
                    .position(|&i| i == self.sidebar_idx)
                    .or(Some(0))
                else {
                    return;
                };
                let target = clamp_add(pos, delta, selectable.len());
                if let Some(&i) = selectable.get(target) {
                    self.sidebar_idx = i;
                    if let Some(view) = self.sidebar_view(i) {
                        self.open_view(view);
                    }
                }
            }
            Focus::List => {
                if !self.tasks.is_empty() {
                    self.task_idx = clamp_add(self.task_idx, delta, self.tasks.len());
                    self.detail_idx = 0;
                    self.load_detail();
                }
            }
            Focus::Detail => {
                let len = self.detail_items_len();
                if len > 0 {
                    self.detail_idx = clamp_add(self.detail_idx, delta, len);
                }
            }
        }
    }

    fn sidebar_view(&self, i: usize) -> Option<View> {
        match self.sidebar.get(i)? {
            SidebarEntry::View { view, .. } => Some(view.clone()),
            SidebarEntry::Header(_) => None,
        }
    }

    fn toggle_done(&mut self) {
        let ids = self.targets();
        let Some(selected) = self.selected_task().cloned() else {
            return;
        };
        self.mutate(|app| {
            if selected.is_open() {
                let done = app.complete(&ids)?;
                let next: Vec<String> = done
                    .iter()
                    .filter_map(|c| c.next.as_ref())
                    .map(|n| {
                        format!(
                            "next {} on {}",
                            n.title,
                            n.due_date.map(|d| d.to_string()).unwrap_or_default()
                        )
                    })
                    .collect();
                let mut msg = format!("Completed {} task{}", ids.len(), plural(ids.len()));
                if !next.is_empty() {
                    msg.push_str("; ");
                    msg.push_str(&next.join(", "));
                }
                Ok(msg)
            } else {
                app.reopen(&ids)?;
                Ok(format!("Reopened {} task{}", ids.len(), plural(ids.len())))
            }
        });
        self.marked.clear();
    }

    fn toggle_detail_item(&mut self) {
        let Some(detail) = &self.detail else {
            return;
        };
        let checklist_len = detail.checklist.len();
        if let Some(item) = detail.checklist.get(self.detail_idx) {
            let item_id = item.id;
            self.mutate(|app| {
                app.checklist_toggle(item_id)?;
                Ok(String::new())
            });
        } else if let Some(sub) = detail.subtasks.get(self.detail_idx - checklist_len) {
            let (id, open) = (sub.id, sub.is_open());
            self.mutate(|app| {
                if open {
                    app.complete(&[id])?;
                } else {
                    app.reopen(&[id])?;
                }
                Ok(String::new())
            });
        }
    }

    fn reorder(&mut self, up: bool) {
        let Some(id) = self.selected_task().map(|t| t.id) else {
            return;
        };
        if self.sort() != TaskSort::Manual {
            self.error("Manual order applies in the Inbox and project views");
            return;
        }
        let order: Vec<TaskId> = self.tasks.iter().map(|(t, _)| t.id).collect();
        self.mutate(|app| {
            app.reorder(id, &order, up)?;
            Ok(String::new())
        });
    }

    fn palette_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for entry in &self.sidebar {
            if let SidebarEntry::View { view, .. } = entry {
                let prefix = match view.kind {
                    ViewKind::Project(_) => "Project",
                    ViewKind::Tag(_) => "Tag",
                    ViewKind::Saved(_) => "Filter",
                    _ => "View",
                };
                items.push(Item {
                    label: format!("{prefix}: {}", view.name),
                    hint: view.query.clone(),
                    target: Target::View(view.clone()),
                });
            }
        }
        for (action, _, label) in Action::ALL {
            if matches!(
                action,
                Action::Up
                    | Action::Down
                    | Action::Top
                    | Action::Bottom
                    | Action::PageUp
                    | Action::PageDown
                    | Action::FocusLeft
                    | Action::FocusRight
                    | Action::Palette
                    | Action::Back
            ) {
                continue;
            }
            items.push(Item {
                label: (*label).to_owned(),
                hint: self.keymap.keys_for(*action),
                target: Target::Action(*action),
            });
        }
        if let Ok(filters) = self.app.saved_filters() {
            for filter in filters {
                items.push(Item {
                    label: format!("Context: {}", filter.name),
                    hint: filter.query,
                    target: Target::Context(Some(filter.name)),
                });
            }
        }
        if self.context.is_some() {
            items.push(Item {
                label: "Context: none".into(),
                hint: "show everything".into(),
                target: Target::Context(None),
            });
        }
        items
    }

    fn switch_context(&mut self, name: Option<&str>) {
        self.mutate(|app| {
            Ok(match app.set_context(name)? {
                Some(name) => format!("Context: {name}"),
                None => "Context cleared".into(),
            })
        });
    }
}

fn build_sidebar(app: &App) -> anyhow::Result<Vec<SidebarEntry>> {
    let mut entries = vec![SidebarEntry::Header("Views")];
    for view in View::builtins() {
        let count = if view.name == "Logbook" {
            0
        } else {
            app.view_count(&view)?
        };
        entries.push(SidebarEntry::View {
            view,
            count,
            depth: 0,
        });
    }
    let projects = app.projects(false)?;
    if !projects.is_empty() {
        entries.push(SidebarEntry::Header("Projects"));
        for project in &projects {
            entries.push(SidebarEntry::View {
                view: View::project(project),
                count: project.open_tasks,
                depth: u8::from(project.parent_id.is_some()),
            });
        }
    }
    let tags: Vec<_> = app
        .tags()?
        .into_iter()
        .filter(|t| t.open_tasks > 0)
        .collect();
    if !tags.is_empty() {
        entries.push(SidebarEntry::Header("Tags"));
        for tag in tags {
            entries.push(SidebarEntry::View {
                view: View::tag(&tag.name),
                count: tag.open_tasks,
                depth: 0,
            });
        }
    }
    let filters = app.saved_filters()?;
    if !filters.is_empty() {
        entries.push(SidebarEntry::Header("Filters"));
        for filter in &filters {
            let view = View::saved(filter);
            let count = app.view_count(&view).unwrap_or(0);
            entries.push(SidebarEntry::View {
                view,
                count,
                depth: 0,
            });
        }
    }
    Ok(entries)
}

/// Tab-completes `input` from `candidates`, cycling once it is complete.
fn complete_from(paths: &[String], input: &mut LineInput) {
    let typed = input.text().to_lowercase();
    let next = match paths.iter().position(|p| p.to_lowercase() == typed) {
        // Already complete: cycle to the next match of the same prefix.
        Some(i) => paths
            .iter()
            .cycle()
            .skip(i + 1)
            .take(paths.len())
            .next()
            .cloned(),
        None => paths
            .iter()
            .find(|p| p.to_lowercase().starts_with(&typed))
            .cloned(),
    };
    if let Some(next) = next {
        *input = LineInput::new(&next);
    }
}

pub fn sort_name(sort: TaskSort) -> &'static str {
    match sort {
        TaskSort::Smart => "due date",
        TaskSort::Manual => "manual order",
        TaskSort::Priority => "priority",
        TaskSort::Created => "newest",
        TaskSort::Completed => "completion",
        TaskSort::Title => "title",
        TaskSort::Urgency => "urgency",
    }
}

/// `~/x` → the home directory joined with `x`.
fn expand_home(text: &str) -> PathBuf {
    let home = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    };
    match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => match home() {
            Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(text),
        },
        _ => PathBuf::from(text),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Tab-completes a file system path in `input`: a single match is filled
/// in (directories get a trailing separator), several are completed to their
/// common prefix and returned as a hint.
fn complete_path(input: &mut LineInput) -> Option<String> {
    let typed = input.text();
    let (dir_text, prefix) = match typed.rfind(['/', '\\']) {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed.as_str()),
    };
    let dir = if dir_text.is_empty() {
        PathBuf::from(".")
    } else {
        expand_home(dir_text)
    };
    let entries = std::fs::read_dir(&dir).ok()?;
    let lower = prefix.to_lowercase();
    let mut matches: Vec<(String, bool)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            // Hidden files only when asked for with a leading dot.
            let visible = !name.starts_with('.') || prefix.starts_with('.');
            (visible && name.to_lowercase().starts_with(&lower)).then_some((name, is_dir))
        })
        .collect();
    matches.sort();
    match matches.as_slice() {
        [] => Some("no matches".into()),
        [(name, is_dir)] => {
            let sep = if *is_dir {
                std::path::MAIN_SEPARATOR.to_string()
            } else {
                String::new()
            };
            *input = LineInput::new(&format!("{dir_text}{name}{sep}"));
            None
        }
        many => {
            let common = many
                .iter()
                .skip(1)
                .fold(many[0].0.clone(), |acc, (name, _)| {
                    acc.chars()
                        .zip(name.chars())
                        .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        .map(|(a, _)| a)
                        .collect()
                });
            if common.chars().count() > prefix.chars().count() {
                *input = LineInput::new(&format!("{dir_text}{common}"));
            }
            let names: Vec<String> = many
                .iter()
                .map(|(name, is_dir)| {
                    if *is_dir {
                        format!("{name}{}", std::path::MAIN_SEPARATOR)
                    } else {
                        name.clone()
                    }
                })
                .collect();
            Some(names.join("  "))
        }
    }
}

fn combine(view_query: &str, filter: &str) -> String {
    match (view_query.trim().is_empty(), filter.trim().is_empty()) {
        (true, _) => filter.to_owned(),
        (false, true) => view_query.to_owned(),
        (false, false) => format!("({view_query}) and ({filter})"),
    }
}

fn clamp_add(pos: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (pos as isize)
        .saturating_add(delta)
        .clamp(0, len as isize - 1) as usize
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
