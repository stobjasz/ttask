//! Mouse support: click to select a view, a task or an agenda cell, click a
//! task's checkbox to toggle it, scroll lists and popups with the wheel.
//! Off with `[ui] mouse = false` (terminal text selection then works
//! without Shift).

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Rect};

use crate::keymap::Action;
use crate::state::{Focus, Mode, SidebarEntry, Tui};

/// Where the last frame drew the panes (their inner areas).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hitboxes {
    pub sidebar: Option<Rect>,
    pub list: Option<Rect>,
    pub detail: Option<Rect>,
}

/// Columns at the start of a task row that toggle it when clicked: the
/// mark column and the checkbox glyph.
const CHECKBOX_COLUMNS: u16 = 4;

/// Rows the wheel scrolls lists by.
const WHEEL_ROWS: isize = 3;

fn contains(rect: Option<Rect>, x: u16, y: u16) -> bool {
    rect.is_some_and(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
}

impl Tui {
    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        self.touch();
        let (x, y) = (ev.column, ev.row);
        let scroll = match ev.kind {
            MouseEventKind::ScrollDown => Some(1i16),
            MouseEventKind::ScrollUp => Some(-1),
            _ => None,
        };
        match &mut self.mode {
            Mode::Normal => {}
            Mode::Help { scroll: s }
            | Mode::History { scroll: s, .. }
            | Mode::Report { scroll: s, .. } => {
                if let Some(d) = scroll {
                    *s = s.saturating_add_signed(d * WHEEL_ROWS as i16);
                }
                return;
            }
            Mode::Palette(palette) => {
                if let Some(d) = scroll {
                    palette.move_selection(d as isize);
                }
                return;
            }
            // Prompts, the editor, focus mode and the lock screen are
            // keyboard-only.
            _ => return,
        }
        let hit = self.hit;
        if let Some(d) = scroll {
            if contains(hit.sidebar, x, y) {
                self.focus = Focus::Sidebar;
                self.dispatch(if d > 0 { Action::Down } else { Action::Up });
            } else if contains(hit.list, x, y) {
                self.focus = Focus::List;
                let action = if d > 0 { Action::Down } else { Action::Up };
                for _ in 0..WHEEL_ROWS {
                    self.dispatch(action);
                }
            }
            return;
        }
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        if let Some(rect) = hit.sidebar.filter(|_| contains(hit.sidebar, x, y)) {
            let height = rect.height as usize;
            let offset = self.sidebar_idx.saturating_sub(height.saturating_sub(1));
            let i = offset + (y - rect.y) as usize;
            if let Some(SidebarEntry::View { view, .. }) = self.sidebar.get(i).cloned() {
                self.focus = Focus::Sidebar;
                self.sidebar_idx = i;
                self.open_view(view);
            }
        } else if let Some(rect) = hit.list.filter(|_| contains(hit.list, x, y)) {
            self.focus = Focus::List;
            if self.agenda.is_some() {
                self.click_agenda(rect, x, y);
                return;
            }
            let i = self.list_offset + (y - rect.y) as usize;
            if i < self.tasks.len() {
                let already = i == self.task_idx;
                self.task_idx = i;
                self.detail_idx = 0;
                self.reload();
                if x < rect.x + CHECKBOX_COLUMNS {
                    self.dispatch(Action::ToggleDone);
                } else if already && !self.detail_visible() {
                    self.dispatch(Action::Open);
                }
            }
        } else if contains(hit.detail, x, y) && self.detail.is_some() {
            self.focus = Focus::Detail;
        }
    }

    fn click_agenda(&mut self, rect: Rect, x: u16, y: u16) {
        let columns = Layout::horizontal([Constraint::Ratio(1, 7); 7]).split(rect);
        let Some(day) = columns.iter().position(|c| x >= c.x && x < c.right()) else {
            return;
        };
        let body = rect.height.saturating_sub(1) as usize;
        if let Some(agenda) = self.agenda.as_mut() {
            if agenda.day != day {
                agenda.day = day;
                self.task_idx = 0;
            }
        }
        self.reload();
        if y > rect.y {
            let offset = (self.task_idx + 1).saturating_sub(body);
            let i = offset + (y - rect.y - 1) as usize;
            if i < self.tasks.len() {
                self.task_idx = i;
                self.reload();
            }
        }
    }
}
