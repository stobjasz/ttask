//! A small multi-line text area for notes in the task editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextArea {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// First line shown; kept so the cursor stays visible.
    pub scroll: usize,
}

impl TextArea {
    pub fn new(text: &str) -> TextArea {
        let mut lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        TextArea {
            lines,
            row: 0,
            col: 0,
            scroll: 0,
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.lines.iter().map(|l| l.iter().collect())
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn at_first_line(&self) -> bool {
        self.row == 0
    }

    /// Display columns before the cursor on its line.
    pub fn cursor_column(&self) -> usize {
        self.lines[self.row][..self.col]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum()
    }

    /// Scrolls so the cursor line is among `height` visible lines.
    pub fn follow_cursor(&mut self, height: usize) {
        let height = height.max(1);
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if self.row >= self.scroll + height {
            self.scroll = self.row + 1 - height;
        }
    }

    /// Applies an editing key; returns `false` for keys it leaves to the
    /// caller (Tab, Esc, Up on the first line, Ctrl+letters it doesn't use).
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        let ctrl = crate::input::is_ctrl(&key);
        let len = self.lines[self.row].len();
        match key.code {
            KeyCode::Char('k') if ctrl => self.lines[self.row].truncate(self.col),
            KeyCode::Char('u') if ctrl => {
                self.lines[self.row].drain(..self.col);
                self.col = 0;
            }
            KeyCode::Char('a') if ctrl => self.col = 0,
            KeyCode::Char('e') if ctrl => self.col = len,
            KeyCode::Char(_) if ctrl => return false,
            KeyCode::Char(c) => {
                self.lines[self.row].insert(self.col, c);
                self.col += 1;
            }
            KeyCode::Enter => {
                let rest = self.lines[self.row].split_off(self.col);
                self.lines.insert(self.row + 1, rest);
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Backspace => {
                if self.col > 0 {
                    self.col -= 1;
                    self.lines[self.row].remove(self.col);
                } else if self.row > 0 {
                    let line = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                    self.lines[self.row].extend(line);
                }
            }
            KeyCode::Delete => {
                if self.col < len {
                    self.lines[self.row].remove(self.col);
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].extend(next);
                }
            }
            KeyCode::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                }
            }
            KeyCode::Right => {
                if self.col < len {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            KeyCode::Up if self.row == 0 => return false,
            KeyCode::Up => self.move_row(-1),
            KeyCode::Down => self.move_row(1),
            KeyCode::PageUp => self.move_row(-10),
            KeyCode::PageDown => self.move_row(10),
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = len,
            _ => return false,
        }
        true
    }

    fn move_row(&mut self, delta: isize) {
        let last = self.lines.len() as isize - 1;
        self.row = (self.row as isize + delta).clamp(0, last) as usize;
        self.col = self.col.min(self.lines[self.row].len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(area: &mut TextArea, text: &str) {
        for c in text.chars() {
            area.handle(key(if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            }));
        }
    }

    #[test]
    fn typing_splitting_and_joining_lines() {
        let mut area = TextArea::new("");
        typed(&mut area, "one\ntwo");
        assert_eq!(area.text(), "one\ntwo");
        area.handle(key(KeyCode::Home));
        area.handle(key(KeyCode::Backspace));
        assert_eq!(area.text(), "onetwo");
        assert_eq!(area.cursor_column(), 3);
        area.handle(key(KeyCode::Enter));
        area.handle(key(KeyCode::Up));
        area.handle(key(KeyCode::End));
        area.handle(key(KeyCode::Delete));
        assert_eq!(area.text(), "onetwo");
    }

    #[test]
    fn altgr_letters_are_typed() {
        let mut area = TextArea::new("");
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert!(area.handle(KeyEvent::new(KeyCode::Char('ą'), altgr)));
        assert!(!area.handle(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)));
        assert_eq!(area.text(), "ą");
    }

    #[test]
    fn up_on_the_first_line_is_left_to_the_caller() {
        let mut area = TextArea::new("a\nb");
        assert!(!area.handle(key(KeyCode::Up)));
        assert!(area.handle(key(KeyCode::Down)));
        assert_eq!(area.row(), 1);
        assert!(!area.handle(key(KeyCode::Tab)));
        area.follow_cursor(1);
        assert_eq!(area.scroll, 1);
    }
}
