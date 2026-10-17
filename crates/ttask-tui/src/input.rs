//! A single-line text input with a cursor and the usual editing keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

/// Whether `key` is a Ctrl shortcut rather than typed text. Windows reports
/// AltGr as Ctrl+Alt, so AltGr letters (Polish ą ł ż, German € @) carry both.
pub fn is_ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    chars: Vec<char>,
    cursor: usize,
    /// Show `*` instead of the text (password prompts).
    pub masked: bool,
}

impl LineInput {
    pub fn new(initial: &str) -> LineInput {
        let chars: Vec<char> = initial.chars().collect();
        LineInput {
            cursor: chars.len(),
            chars,
            masked: false,
        }
    }

    pub fn masked() -> LineInput {
        LineInput {
            masked: true,
            ..LineInput::default()
        }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// What to draw: the text, or one `*` per character when masked.
    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.chars.len())
        } else {
            self.text()
        }
    }

    /// Display columns before the cursor.
    pub fn cursor_column(&self) -> u16 {
        let before: String = if self.masked {
            "*".repeat(self.cursor)
        } else {
            self.chars[..self.cursor].iter().collect()
        };
        before.width() as u16
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    /// Applies an editing key. Returns `false` for keys it does not handle
    /// (Enter, Esc, Tab, arrows up/down).
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        let ctrl = is_ctrl(&key);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.chars.len(),
            KeyCode::Char('u') if ctrl => {
                self.chars.drain(..self.cursor);
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.chars.truncate(self.cursor),
            KeyCode::Char('w') if ctrl => self.delete_word_back(),
            KeyCode::Backspace if ctrl || alt => self.delete_word_back(),
            KeyCode::Char('b') if alt => self.cursor = self.word_left(),
            KeyCode::Char('f') if alt => self.cursor = self.word_right(),
            KeyCode::Left if ctrl => self.cursor = self.word_left(),
            KeyCode::Right if ctrl => self.cursor = self.word_right(),
            KeyCode::Char(c) if !ctrl => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.chars.len(),
            _ => return false,
        }
        true
    }

    fn word_left(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let mut i = self.cursor;
        while i < self.chars.len() && !self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    fn delete_word_back(&mut self) {
        let start = self.word_left();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn edits_at_cursor() {
        let mut input = LineInput::new("hllo");
        input.handle(key(KeyCode::Home));
        input.handle(key(KeyCode::Right));
        input.handle(key(KeyCode::Char('e')));
        assert_eq!(input.text(), "hello");
        input.handle(key(KeyCode::End));
        input.handle(key(KeyCode::Backspace));
        assert_eq!(input.text(), "hell");
        assert_eq!(input.cursor_column(), 4);
    }

    #[test]
    fn word_editing() {
        let mut input = LineInput::new("pay the rent");
        input.handle(ctrl('w'));
        assert_eq!(input.text(), "pay the ");
        input.handle(ctrl('a'));
        input.handle(ctrl('k'));
        assert_eq!(input.text(), "");
        assert!(!input.handle(key(KeyCode::Enter)));
    }

    #[test]
    fn altgr_letters_are_typed() {
        let altgr = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let mut input = LineInput::new("");
        for c in "żółć".chars() {
            assert!(input.handle(altgr(c)));
        }
        assert_eq!(input.text(), "żółć");
    }

    #[test]
    fn wide_characters_and_masking() {
        let input = LineInput::new("日本");
        assert_eq!(input.cursor_column(), 4);
        let mut pw = LineInput::masked();
        pw.handle(key(KeyCode::Char('s')));
        assert_eq!(pw.display(), "*");
        assert_eq!(pw.text(), "s");
    }
}
