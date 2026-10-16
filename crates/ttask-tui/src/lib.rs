//! The full-screen TUI: sidebar (views, projects, tags, saved filters), task
//! list and detail pane over the same services the CLI uses.

mod agenda;
mod editor;
mod focus;
mod input;
mod keymap;
mod markdown;
mod mouse;
mod palette;
mod review;
mod state;
mod textarea;
mod theme;
mod ui;

use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ttask_app::App;

pub use focus::FocusSettings;
pub use keymap::{Action, KeymapName};
pub use state::{Focus, LinkOpener, Mode, Notifier, Opener, Settings, Tui, Unlocker};
pub use theme::{DetailPane, ThemeName};

/// Runs the TUI until the user quits. The terminal is restored on exit and
/// on panic.
pub fn run(app: App, settings: Settings) -> anyhow::Result<()> {
    let mouse = settings.mouse;
    let mut tui = Tui::new(app, settings)?;
    let mut terminal = init(mouse);
    let result = event_loop(&mut terminal, &mut tui, mouse);
    restore(mouse);
    result
}

/// Enters the TUI screen, capturing the mouse if asked; a panic still
/// releases it.
fn init(mouse: bool) -> DefaultTerminal {
    let terminal = ratatui::init();
    if mouse {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            hook(info);
        }));
        let _ = ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    }
    terminal
}

fn restore(mouse: bool) {
    if mouse {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
}

fn event_loop(terminal: &mut DefaultTerminal, tui: &mut Tui, mouse: bool) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, tui))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                // Windows reports releases too; act on presses (and repeats) only.
                Event::Key(key) if key.kind != KeyEventKind::Release => tui.handle_key(key),
                Event::Mouse(ev) => tui.handle_mouse(ev),
                _ => {}
            }
        }
        tui.tick(Instant::now());
        if let Some(id) = tui.take_editor_request() {
            let notes = tui.app.task(id).map(|t| t.notes).unwrap_or_default();
            restore(mouse);
            let result = ttask_app::editor::edit_text(&notes, ".md");
            *terminal = init(mouse);
            tui.finish_notes(id, result);
        }
        if let Some(text) = tui.take_clipboard_request() {
            copy_to_clipboard(&text);
        }
        if tui.quit {
            return Ok(());
        }
    }
}

/// Asks the terminal to set the clipboard (OSC 52). Windows Terminal,
/// iTerm2, kitty, Alacritty, WezTerm and tmux (with `set-clipboard on`)
/// support it; other terminals ignore the sequence.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
