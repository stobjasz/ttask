//! The `:` command palette: fuzzy search over actions and views.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ttask_app::View;

use crate::input::LineInput;
use crate::keymap::Action;

#[derive(Debug, Clone)]
pub enum Target {
    Action(Action),
    View(View),
    /// Activate a saved filter as the context, or clear it with `None`.
    Context(Option<String>),
    /// Open a link from the selected task.
    Link(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub hint: String,
    pub target: Target,
}

pub struct Palette {
    pub title: &'static str,
    pub input: LineInput,
    pub items: Vec<Item>,
    /// Indices into `items`, best match first.
    pub matches: Vec<usize>,
    pub selected: usize,
    matcher: Matcher,
}

impl Palette {
    pub fn new(items: Vec<Item>) -> Palette {
        Palette::titled("Command palette", items)
    }

    /// A palette over other items (e.g. a task's links).
    pub fn titled(title: &'static str, items: Vec<Item>) -> Palette {
        let mut palette = Palette {
            title,
            input: LineInput::default(),
            matches: (0..items.len()).collect(),
            items,
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        palette.refilter();
        palette
    }

    pub fn refilter(&mut self) {
        let query = self.input.text();
        if query.trim().is_empty() {
            self.matches = (0..self.items.len()).collect();
        } else {
            let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
            let mut buf = Vec::new();
            let mut scored: Vec<(u32, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let haystack = format!("{} {}", item.label, item.hint);
                    pattern
                        .score(Utf32Str::new(&haystack, &mut buf), &mut self.matcher)
                        .map(|score| (score, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.matches = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.selected = 0;
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.matches.get(self.selected).map(|&i| &self.items[i])
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_ranks_best_match_first() {
        let items = vec![
            Item {
                label: "Undo".into(),
                hint: "u".into(),
                target: Target::Action(Action::Undo),
            },
            Item {
                label: "Toggle done".into(),
                hint: "x".into(),
                target: Target::Action(Action::ToggleDone),
            },
            Item {
                label: "View: Today".into(),
                hint: "".into(),
                target: Target::View(View::today()),
            },
        ];
        let mut p = Palette::new(items);
        assert_eq!(p.matches.len(), 3);
        for c in "tdy".chars() {
            p.input.handle(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char(c),
            ));
        }
        p.refilter();
        assert_eq!(p.selected_item().unwrap().label, "View: Today");
    }
}
