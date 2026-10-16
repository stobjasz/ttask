//! Focus mode (`F`): one task full-screen with a Pomodoro timer. Work
//! phases run the task's time tracker; breaks pause it.

use std::time::{Duration, Instant};

use serde::Deserialize;
use ttask_core::TaskId;

/// `[focus]` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FocusSettings {
    pub work_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    /// A long break after this many work phases.
    pub long_break_every: u32,
}

impl Default for FocusSettings {
    fn default() -> Self {
        FocusSettings {
            work_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            long_break_every: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Work,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Work => "Focus",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FocusSession {
    pub task_id: TaskId,
    pub phase: Phase,
    pub phase_started: Instant,
    /// Set while paused.
    pub paused_at: Option<Instant>,
    /// Time spent paused in this phase.
    pub paused: Duration,
    /// Work phases finished in this session.
    pub completed: u32,
    /// Selected checklist item.
    pub item: usize,
    pub settings: FocusSettings,
}

impl FocusSession {
    pub fn new(task_id: TaskId, settings: FocusSettings, now: Instant) -> FocusSession {
        FocusSession {
            task_id,
            phase: Phase::Work,
            phase_started: now,
            paused_at: None,
            paused: Duration::ZERO,
            completed: 0,
            item: 0,
            settings,
        }
    }

    pub fn length(&self, phase: Phase) -> Duration {
        let minutes = match phase {
            Phase::Work => self.settings.work_minutes,
            Phase::ShortBreak => self.settings.short_break_minutes,
            Phase::LongBreak => self.settings.long_break_minutes,
        };
        Duration::from_secs(u64::from(minutes.max(1)) * 60)
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        let end = self.paused_at.unwrap_or(now);
        end.saturating_duration_since(self.phase_started)
            .saturating_sub(self.paused)
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.length(self.phase).saturating_sub(self.elapsed(now))
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn toggle_pause(&mut self, now: Instant) {
        match self.paused_at.take() {
            Some(at) => self.paused += now.saturating_duration_since(at),
            None => self.paused_at = Some(now),
        }
    }

    /// The phase after the current one.
    pub fn next_phase(&self) -> Phase {
        match self.phase {
            Phase::Work => {
                let every = self.settings.long_break_every.max(1);
                if (self.completed + 1) % every == 0 {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::ShortBreak | Phase::LongBreak => Phase::Work,
        }
    }

    /// Moves to the next phase (when time is up, or when skipped).
    pub fn advance(&mut self, now: Instant) {
        if self.phase == Phase::Work {
            self.phase = self.next_phase();
            self.completed += 1;
        } else {
            self.phase = Phase::Work;
        }
        self.phase_started = now;
        self.paused_at = None;
        self.paused = Duration::ZERO;
    }
}

/// `MM:SS`.
pub fn clock(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Large digits for the countdown: five rows per character.
pub fn big_text(text: &str, ascii: bool) -> [String; 5] {
    const FONT: [(char, [&str; 5]); 11] = [
        ('0', ["███", "█ █", "█ █", "█ █", "███"]),
        ('1', [" ██", "  █", "  █", "  █", "  █"]),
        ('2', ["███", "  █", "███", "█  ", "███"]),
        ('3', ["███", "  █", "███", "  █", "███"]),
        ('4', ["█ █", "█ █", "███", "  █", "  █"]),
        ('5', ["███", "█  ", "███", "  █", "███"]),
        ('6', ["███", "█  ", "███", "█ █", "███"]),
        ('7', ["███", "  █", "  █", "  █", "  █"]),
        ('8', ["███", "█ █", "███", "█ █", "███"]),
        ('9', ["███", "█ █", "███", "  █", "███"]),
        (':', ["   ", " █ ", "   ", " █ ", "   "]),
    ];
    let mut rows: [String; 5] = Default::default();
    for (n, c) in text.chars().enumerate() {
        let glyph = FONT
            .iter()
            .find(|(f, _)| *f == c)
            .map_or(["   "; 5], |(_, g)| *g);
        for (row, part) in rows.iter_mut().zip(glyph) {
            if n > 0 {
                row.push(' ');
            }
            row.push_str(&if ascii {
                part.replace('█', "#")
            } else {
                part.to_owned()
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_pauses_and_long_breaks() {
        let start = Instant::now();
        let settings = FocusSettings {
            long_break_every: 2,
            ..FocusSettings::default()
        };
        let mut s = FocusSession::new(1, settings, start);
        let min = Duration::from_secs(60);
        assert_eq!(s.remaining(start + 10 * min), 15 * min);
        s.toggle_pause(start + 10 * min);
        assert_eq!(
            s.remaining(start + 20 * min),
            15 * min,
            "frozen while paused"
        );
        s.toggle_pause(start + 20 * min);
        assert_eq!(s.remaining(start + 30 * min), 5 * min);

        assert_eq!(s.next_phase(), Phase::ShortBreak);
        s.advance(start + 35 * min);
        assert_eq!((s.phase, s.completed), (Phase::ShortBreak, 1));
        s.advance(start + 40 * min);
        assert_eq!(s.next_phase(), Phase::LongBreak, "after the second");
        s.advance(start + 65 * min);
        assert_eq!((s.phase, s.completed), (Phase::LongBreak, 2));
        assert_eq!(s.remaining(start + 65 * min), 15 * min);
    }

    #[test]
    fn clock_and_big_digits() {
        assert_eq!(clock(Duration::from_secs(24 * 60 + 5)), "24:05");
        let rows = big_text("1:0", true);
        assert_eq!(rows[0], " ##     ###");
        assert_eq!(rows[1], "  #  #  # #");
    }
}
