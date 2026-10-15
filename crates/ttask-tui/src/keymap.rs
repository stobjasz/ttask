//! Actions and the keys bound to them: a vim-style default, a `simple`
//! preset, and per-action overrides from `[keys]` in the config.

use std::collections::{BTreeMap, HashMap};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeymapName {
    #[default]
    Vim,
    Simple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
    FocusNext,
    FocusPrev,
    FocusLeft,
    FocusRight,
    Open,
    Add,
    AddSubtask,
    Edit,
    EditNotes,
    Modify,
    ToggleDone,
    Cancel,
    Priority(u8),
    Due,
    Move,
    Tags,
    Filter,
    Search,
    Palette,
    Undo,
    Redo,
    Mark,
    ReorderUp,
    ReorderDown,
    Delete,
    Skip,
    ChecklistAdd,
    ToggleItem,
    Timer,
    SaveFilter,
    Wait,
    Context,
    Help,
    Quit,
    Back,
}

impl Action {
    /// Every action with its config name and help text.
    pub const ALL: &'static [(Action, &'static str, &'static str)] = &[
        (Action::Up, "up", "Move up"),
        (Action::Down, "down", "Move down"),
        (Action::Top, "top", "Go to top"),
        (Action::Bottom, "bottom", "Go to bottom"),
        (Action::PageUp, "page_up", "Page up"),
        (Action::PageDown, "page_down", "Page down"),
        (Action::FocusNext, "focus_next", "Next pane"),
        (Action::FocusPrev, "focus_prev", "Previous pane"),
        (Action::FocusLeft, "focus_left", "Pane to the left"),
        (Action::FocusRight, "focus_right", "Pane to the right"),
        (Action::Open, "open", "Open view / full-screen detail"),
        (Action::Add, "quick_add", "Quick-add a task"),
        (Action::AddSubtask, "add_subtask", "Add a subtask"),
        (Action::Edit, "edit", "Edit title"),
        (Action::EditNotes, "edit_notes", "Edit notes in $EDITOR"),
        (Action::Modify, "modify", "Modify with quick-add tokens"),
        (Action::ToggleDone, "done", "Toggle done"),
        (Action::Cancel, "cancel", "Cancel task"),
        (Action::Priority(0), "priority_1", "Priority low"),
        (Action::Priority(1), "priority_2", "Priority medium"),
        (Action::Priority(2), "priority_3", "Priority high"),
        (Action::Priority(3), "priority_4", "Priority urgent"),
        (Action::Due, "due", "Set due date"),
        (Action::Move, "move", "Move to project"),
        (Action::Tags, "tags", "Edit tags"),
        (Action::Filter, "filter", "Filter current view"),
        (Action::Search, "search", "Full-text search"),
        (Action::Palette, "palette", "Command palette"),
        (Action::Undo, "undo", "Undo"),
        (Action::Redo, "redo", "Redo"),
        (Action::Mark, "mark", "Mark for multi-select"),
        (Action::ReorderUp, "reorder_up", "Move task up"),
        (Action::ReorderDown, "reorder_down", "Move task down"),
        (Action::Delete, "delete", "Delete task"),
        (Action::Skip, "skip", "Skip to next occurrence"),
        (Action::ChecklistAdd, "checklist_add", "Add checklist item"),
        (
            Action::ToggleItem,
            "toggle_item",
            "Toggle checklist item / subtask",
        ),
        (Action::Timer, "timer", "Start/stop timer"),
        (Action::SaveFilter, "save_filter", "Save filter"),
        (Action::Wait, "wait", "Wait until / someday"),
        (Action::Context, "context", "Switch context"),
        (Action::Help, "help", "Help"),
        (Action::Quit, "quit", "Quit"),
        (Action::Back, "back", "Back / clear filter"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(a, _, _)| *a == self)
            .map(|(_, _, label)| *label)
            .unwrap_or("")
    }

    fn by_name(name: &str) -> Option<Action> {
        let name = if name == "add" { "quick_add" } else { name };
        Self::ALL
            .iter()
            .find(|(_, n, _)| *n == name)
            .map(|(a, _, _)| *a)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    fn from_event(key: &KeyEvent) -> Chord {
        let mut mods = key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
        // Shift is already part of 'J' (vs 'j') and of BackTab, which some
        // terminals report with the Shift flag and some without.
        if !matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods |= key.modifiers & KeyModifiers::SHIFT;
        }
        Chord {
            code: key.code,
            mods,
        }
    }

    /// `x`, `J`, `ctrl-f`, `alt-up`, `shift-tab`, `enter`, `space`, `f1`.
    pub fn parse(s: &str) -> Result<Chord, String> {
        let lower = s.to_ascii_lowercase();
        let mut mods = KeyModifiers::NONE;
        let mut rest = s;
        loop {
            let l = rest.to_ascii_lowercase();
            if let Some(r) = l.strip_prefix("ctrl-").or_else(|| l.strip_prefix("c-")) {
                mods |= KeyModifiers::CONTROL;
                rest = &rest[rest.len() - r.len()..];
            } else if let Some(r) = l.strip_prefix("alt-").or_else(|| l.strip_prefix("m-")) {
                mods |= KeyModifiers::ALT;
                rest = &rest[rest.len() - r.len()..];
            } else if let Some(r) = l.strip_prefix("shift-") {
                mods |= KeyModifiers::SHIFT;
                rest = &rest[rest.len() - r.len()..];
            } else {
                break;
            }
        }
        let code = match rest.to_ascii_lowercase().as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "delete" | "del" => KeyCode::Delete,
            "backspace" => KeyCode::Backspace,
            f if f.starts_with('f') && f.len() > 1 && f[1..].parse::<u8>().is_ok() => {
                KeyCode::F(f[1..].parse().expect("checked"))
            }
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(format!("unknown key `{lower}`")),
                }
            }
        };
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Ok(Chord { code, mods })
    }

    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            s.push_str("Ctrl-");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            s.push_str("Alt-");
        }
        match self.code {
            KeyCode::Char(' ') => s.push_str("Space"),
            KeyCode::Char(c) => s.push(c),
            KeyCode::Enter => s.push_str("Enter"),
            KeyCode::Esc => s.push_str("Esc"),
            KeyCode::Tab => s.push_str("Tab"),
            KeyCode::BackTab => s.push_str("Shift-Tab"),
            KeyCode::Up => s.push('↑'),
            KeyCode::Down => s.push('↓'),
            KeyCode::Left => s.push('←'),
            KeyCode::Right => s.push('→'),
            KeyCode::Home => s.push_str("Home"),
            KeyCode::End => s.push_str("End"),
            KeyCode::PageUp => s.push_str("PgUp"),
            KeyCode::PageDown => s.push_str("PgDn"),
            KeyCode::Delete => s.push_str("Del"),
            KeyCode::F(n) => s.push_str(&format!("F{n}")),
            _ => s.push('?'),
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<(Chord, Action)>,
    lookup: HashMap<Chord, Action>,
}

impl Keymap {
    pub fn new(name: KeymapName, overrides: &BTreeMap<String, String>) -> Result<Keymap, String> {
        let mut bindings: Vec<(Chord, Action)> = Vec::new();
        let mut bind = |keys: &[&str], action: Action| {
            for key in keys {
                bindings.push((Chord::parse(key).expect("built-in key"), action));
            }
        };
        // Shared by both presets.
        bind(&["up"], Action::Up);
        bind(&["down"], Action::Down);
        bind(&["home"], Action::Top);
        bind(&["end"], Action::Bottom);
        bind(&["pageup"], Action::PageUp);
        bind(&["pagedown"], Action::PageDown);
        bind(&["tab"], Action::FocusNext);
        bind(&["shift-tab"], Action::FocusPrev);
        bind(&["left"], Action::FocusLeft);
        bind(&["right"], Action::FocusRight);
        bind(&["enter"], Action::Open);
        bind(&["a"], Action::Add);
        bind(&["A"], Action::AddSubtask);
        bind(&["e"], Action::Edit);
        bind(&["E"], Action::EditNotes);
        bind(&["M"], Action::Modify);
        bind(&["x"], Action::ToggleDone);
        bind(&["X"], Action::Cancel);
        bind(&["1"], Action::Priority(0));
        bind(&["2"], Action::Priority(1));
        bind(&["3"], Action::Priority(2));
        bind(&["4"], Action::Priority(3));
        bind(&["d"], Action::Due);
        bind(&["m"], Action::Move);
        bind(&["t"], Action::Tags);
        bind(&["/"], Action::Filter);
        bind(&["ctrl-f"], Action::Search);
        bind(&[":", "ctrl-p"], Action::Palette);
        bind(&["u", "ctrl-z"], Action::Undo);
        bind(&["ctrl-r", "ctrl-y"], Action::Redo);
        bind(&["v"], Action::Mark);
        bind(&["D", "delete"], Action::Delete);
        bind(&["s"], Action::Skip);
        bind(&["c"], Action::ChecklistAdd);
        bind(&["space"], Action::ToggleItem);
        bind(&["T"], Action::Timer);
        bind(&["S"], Action::SaveFilter);
        bind(&["w"], Action::Wait);
        bind(&["C"], Action::Context);
        bind(&["?", "f1"], Action::Help);
        bind(&["q"], Action::Quit);
        bind(&["esc"], Action::Back);
        match name {
            KeymapName::Vim => {
                bind(&["k"], Action::Up);
                bind(&["j"], Action::Down);
                bind(&["g"], Action::Top);
                bind(&["G"], Action::Bottom);
                bind(&["ctrl-u"], Action::PageUp);
                bind(&["ctrl-d"], Action::PageDown);
                bind(&["h"], Action::FocusLeft);
                bind(&["l"], Action::FocusRight);
                bind(&["K"], Action::ReorderUp);
                bind(&["J"], Action::ReorderDown);
            }
            KeymapName::Simple => {
                bind(&["alt-up", "ctrl-up"], Action::ReorderUp);
                bind(&["alt-down", "ctrl-down"], Action::ReorderDown);
            }
        }
        for (action_name, key) in overrides {
            let action = Action::by_name(action_name)
                .ok_or_else(|| format!("[keys]: unknown action `{action_name}`"))?;
            let chord = Chord::parse(key).map_err(|e| format!("[keys] {action_name}: {e}"))?;
            bindings.retain(|(c, a)| *a != action && *c != chord);
            bindings.push((chord, action));
        }
        let lookup = bindings.iter().copied().collect();
        Ok(Keymap { bindings, lookup })
    }

    pub fn action(&self, key: &KeyEvent) -> Option<Action> {
        self.lookup.get(&Chord::from_event(key)).copied()
    }

    /// Keys bound to `action`, for help and hints (`↓, j`). Not joined with
    /// `/`, which is itself a key.
    pub fn keys_for(&self, action: Action) -> String {
        let keys: Vec<String> = self
            .bindings
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(c, _)| c.display())
            .collect();
        keys.join(", ")
    }

    /// The first key bound to `action`, for the footer hints.
    pub fn first_key(&self, action: Action) -> String {
        self.bindings
            .iter()
            .find(|(_, a)| *a == action)
            .map(|(c, _)| c.display())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn vim_and_simple_differ_on_navigation() {
        let vim = Keymap::new(KeymapName::Vim, &BTreeMap::new()).unwrap();
        let simple = Keymap::new(KeymapName::Simple, &BTreeMap::new()).unwrap();
        let j = ev(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(vim.action(&j), Some(Action::Down));
        assert_eq!(simple.action(&j), None);
        let down = ev(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(simple.action(&down), Some(Action::Down));
        let shift_j = ev(KeyCode::Char('J'), KeyModifiers::SHIFT);
        assert_eq!(vim.action(&shift_j), Some(Action::ReorderDown));
    }

    #[test]
    fn overrides_replace_bindings() {
        let mut overrides = BTreeMap::new();
        overrides.insert("done".to_owned(), "ctrl-x".to_owned());
        let map = Keymap::new(KeymapName::Vim, &overrides).unwrap();
        assert_eq!(
            map.action(&ev(KeyCode::Char('x'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            map.action(&ev(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Some(Action::ToggleDone)
        );
        overrides.insert("frobnicate".to_owned(), "z".to_owned());
        assert!(Keymap::new(KeymapName::Vim, &overrides).is_err());
    }

    #[test]
    fn chord_parsing() {
        assert_eq!(Chord::parse("ctrl-f").unwrap().code, KeyCode::Char('f'));
        assert_eq!(Chord::parse("shift-tab").unwrap().code, KeyCode::BackTab);
        assert_eq!(Chord::parse("F5").unwrap().code, KeyCode::F(5));
        assert!(Chord::parse("ctrl-banana").is_err());
        let back_tab = ev(KeyCode::BackTab, KeyModifiers::SHIFT);
        let map = Keymap::new(KeymapName::Vim, &BTreeMap::new()).unwrap();
        assert_eq!(map.action(&back_tab), Some(Action::FocusPrev));
    }
}
