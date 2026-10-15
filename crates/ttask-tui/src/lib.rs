//! The full-screen TUI: sidebar (views, projects, tags, saved filters), task
//! list and detail pane over the same services the CLI uses.

mod input;
mod keymap;
mod palette;
mod state;
mod theme;
mod ui;

use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ttask_app::App;

pub use keymap::{Action, KeymapName};
pub use state::{Focus, Mode, Notifier, Settings, Tui, Unlocker};
pub use theme::{DetailPane, ThemeName};

/// Runs the TUI until the user quits. The terminal is restored on exit and
/// on panic.
pub fn run(app: App, settings: Settings) -> anyhow::Result<()> {
    let mut tui = Tui::new(app, settings)?;
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut tui);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, tui: &mut Tui) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, tui))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                // Windows reports releases too; act on presses (and repeats) only.
                if key.kind != KeyEventKind::Release {
                    tui.handle_key(key);
                }
            }
        }
        tui.tick(Instant::now());
        if let Some(id) = tui.take_editor_request() {
            let notes = tui.app.task(id).map(|t| t.notes).unwrap_or_default();
            ratatui::restore();
            let result = ttask_app::editor::edit_text(&notes, ".md");
            *terminal = ratatui::init();
            tui.finish_notes(id, result);
        }
        if tui.quit {
            return Ok(());
        }
    }
}

/// Renders one frame into a buffer, for tests and screenshots.
pub fn render_to_string(tui: &mut Tui, width: u16, height: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
    terminal.draw(|f| ui::draw(f, tui)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
