use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// Terminal's own colours plus a few named accents.
    #[default]
    Auto,
    Dark,
    Light,
    /// No colour, ASCII glyphs only.
    Ascii,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailPane {
    /// Shown when the terminal is at least 100 columns wide.
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub base: Style,
    pub dim: Style,
    pub accent: Style,
    pub border: Style,
    pub border_focus: Style,
    pub selected: Style,
    pub marked: Style,
    pub overdue: Style,
    pub today: Style,
    pub priority: Style,
    pub project: Style,
    pub tag: Style,
    pub done: Style,
    pub error: Style,
    pub ok: Style,
    pub glyphs: Glyphs,
}

#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub open: &'static str,
    pub done: &'static str,
    pub cancelled: &'static str,
    pub recur: &'static str,
    pub marked: &'static str,
    pub blocked: &'static str,
    pub lock: &'static str,
    pub timer: &'static str,
}

const UNICODE: Glyphs = Glyphs {
    open: "○",
    done: "●",
    cancelled: "⊘",
    recur: "↻",
    marked: "▌",
    blocked: "⧗",
    lock: "[L]",
    timer: "◷",
};

const ASCII: Glyphs = Glyphs {
    open: "( )",
    done: "(x)",
    cancelled: "(-)",
    recur: "~",
    marked: "*",
    blocked: "!",
    lock: "[L]",
    timer: "@",
};

impl Theme {
    /// `no_color` (NO_COLOR or --no-color) keeps layout and reverse-video
    /// selection but drops every colour.
    pub fn new(name: ThemeName, no_color: bool) -> Theme {
        let plain = Style::new();
        if name == ThemeName::Ascii || no_color {
            let glyphs = if name == ThemeName::Ascii {
                ASCII
            } else {
                UNICODE
            };
            return Theme {
                base: plain,
                dim: plain,
                accent: plain.add_modifier(Modifier::BOLD),
                border: plain,
                border_focus: plain.add_modifier(Modifier::BOLD),
                selected: plain.add_modifier(Modifier::REVERSED),
                marked: plain.add_modifier(Modifier::BOLD),
                overdue: plain.add_modifier(Modifier::BOLD),
                today: plain.add_modifier(Modifier::UNDERLINED),
                priority: plain.add_modifier(Modifier::BOLD),
                project: plain,
                tag: plain,
                done: plain.add_modifier(Modifier::CROSSED_OUT),
                error: plain.add_modifier(Modifier::BOLD),
                ok: plain,
                glyphs,
            };
        }
        let (fg, dim, selected_bg) = match name {
            ThemeName::Dark => (Color::Gray, Color::DarkGray, Color::Rgb(45, 55, 72)),
            ThemeName::Light => (Color::Black, Color::Gray, Color::Rgb(210, 225, 245)),
            _ => (Color::Reset, Color::DarkGray, Color::Blue),
        };
        let selected = if name == ThemeName::Auto {
            plain.bg(selected_bg).fg(Color::White)
        } else {
            plain.bg(selected_bg)
        };
        Theme {
            base: plain.fg(fg),
            dim: plain.fg(dim),
            accent: plain.fg(Color::Cyan).add_modifier(Modifier::BOLD),
            border: plain.fg(dim),
            border_focus: plain.fg(Color::Cyan),
            selected,
            marked: plain.fg(Color::Magenta).add_modifier(Modifier::BOLD),
            overdue: plain.fg(Color::Red).add_modifier(Modifier::BOLD),
            today: plain.fg(Color::Yellow),
            priority: plain.fg(Color::Red),
            project: plain.fg(Color::Cyan),
            tag: plain.fg(Color::Magenta),
            done: plain.fg(dim).add_modifier(Modifier::CROSSED_OUT),
            error: plain.fg(Color::Red).add_modifier(Modifier::BOLD),
            ok: plain.fg(Color::Green),
            glyphs: UNICODE,
        }
    }
}
