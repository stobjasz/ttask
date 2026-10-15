//! Urgency: one number answering "what should I do next?", modelled on
//! Taskwarrior's weighted sum. Every weight can be changed under
//! `[urgency]` in the config.

use jiff::civil::Date;
use serde::Deserialize;

use crate::date::days_between;
use crate::task::Task;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UrgencyWeights {
    /// Tagged `+next`.
    pub next: f64,
    /// Scaled 0.2 (due in 2+ weeks) to 1.0 (a week or more overdue).
    pub due: f64,
    /// Another open task waits on this one.
    pub blocking: f64,
    pub priority_urgent: f64,
    pub priority_high: f64,
    pub priority_medium: f64,
    /// Scheduled for today or earlier.
    pub scheduled: f64,
    /// Its timer is running.
    pub active: f64,
    /// Scaled by age, full weight at one year.
    pub age: f64,
    /// Scaled 0.8 / 0.9 / 1.0 for one / two / more tags.
    pub tags: f64,
    pub project: f64,
    pub notes: f64,
    /// Waiting on an open task (usually negative).
    pub blocked: f64,
}

impl Default for UrgencyWeights {
    fn default() -> Self {
        UrgencyWeights {
            next: 15.0,
            due: 12.0,
            blocking: 8.0,
            priority_urgent: 9.0,
            priority_high: 6.0,
            priority_medium: 3.9,
            scheduled: 5.0,
            active: 4.0,
            age: 2.0,
            tags: 1.0,
            project: 1.0,
            notes: 1.0,
            blocked: -5.0,
        }
    }
}

/// The urgency of an open task; closed tasks score 0.
pub fn urgency(task: &Task, today: Date, now_ms: i64, active: bool, w: &UrgencyWeights) -> f64 {
    if !task.is_open() {
        return 0.0;
    }
    let mut score = 0.0;
    if task.tags.iter().any(|t| t.eq_ignore_ascii_case("next")) {
        score += w.next;
    }
    if let Some(due) = task.due_date {
        score += w.due * due_factor(days_between(due, today));
    }
    if task.blocking {
        score += w.blocking;
    }
    score += match task.priority.value() {
        3 => w.priority_urgent,
        2 => w.priority_high,
        1 => w.priority_medium,
        _ => 0.0,
    };
    if task.scheduled_date.is_some_and(|d| d <= today) {
        score += w.scheduled;
    }
    if active {
        score += w.active;
    }
    let age_days = ((now_ms - task.created_at).max(0) / 86_400_000) as f64;
    score += w.age * (age_days / 365.0).min(1.0);
    score += w.tags
        * match task.tags.len() {
            0 => 0.0,
            1 => 0.8,
            2 => 0.9,
            _ => 1.0,
        };
    if task.project_id.is_some() {
        score += w.project;
    }
    if !task.notes.trim().is_empty() {
        score += w.notes;
    }
    if task.blocked {
        score += w.blocked;
    }
    (score * 100.0).round() / 100.0
}

/// 1.0 when a week or more overdue, falling linearly to 0.2 for tasks due
/// two weeks or more ahead. `overdue_days` is negative for future dates.
fn due_factor(overdue_days: i64) -> f64 {
    if overdue_days >= 7 {
        1.0
    } else if overdue_days >= -14 {
        (overdue_days + 14) as f64 * 0.8 / 21.0 + 0.2
    } else {
        0.2
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;
    use crate::task::{Priority, Progress, Status};

    const TODAY: Date = date(2026, 10, 15);
    const NOW: i64 = 1_760_529_600_000; // 2026-10-15 12:00 UTC

    fn task() -> Task {
        Task {
            id: 1,
            uid: "u".into(),
            parent_id: None,
            project_id: None,
            project: None,
            title: "t".into(),
            notes: String::new(),
            status: Status::Open,
            priority: Priority::NONE,
            due_date: None,
            due_time: None,
            scheduled_date: None,
            estimate_min: None,
            recur_rule: None,
            recur_mode: None,
            recur_series: None,
            wait_until: None,
            someday: false,
            tags: Vec::new(),
            sort_order: 0.0,
            created_at: NOW,
            updated_at: NOW,
            completed_at: None,
            subtasks: Progress::default(),
            checklist: Progress::default(),
            blocked: false,
            blocking: false,
            urgency: 0.0,
        }
    }

    fn score(t: &Task) -> f64 {
        urgency(t, TODAY, NOW, false, &UrgencyWeights::default())
    }

    #[test]
    fn due_scales_with_closeness() {
        let mut overdue = task();
        overdue.due_date = Some(date(2026, 10, 1));
        let mut today = task();
        today.due_date = Some(TODAY);
        let mut far = task();
        far.due_date = Some(date(2027, 1, 1));
        assert_eq!(score(&overdue), 12.0);
        assert!((score(&today) - 12.0 * (14.0 * 0.8 / 21.0 + 0.2)).abs() < 0.01);
        assert!((score(&far) - 2.4).abs() < 0.01);
        assert!(score(&overdue) > score(&today) && score(&today) > score(&far));
    }

    #[test]
    fn factors_add_up() {
        let mut t = task();
        t.tags = vec!["next".into()];
        t.priority = Priority::new(2).unwrap();
        t.blocking = true;
        t.project_id = Some(1);
        assert_eq!(score(&t), 15.0 + 6.0 + 8.0 + 0.8 + 1.0);
        t.blocked = true;
        assert_eq!(score(&t), 15.0 + 6.0 + 8.0 + 0.8 + 1.0 - 5.0);
        assert!(urgency(&t, TODAY, NOW, true, &UrgencyWeights::default()) > score(&t));
    }

    #[test]
    fn closed_tasks_score_zero_and_age_caps() {
        let mut done = task();
        done.priority = Priority::new(3).unwrap();
        done.status = Status::Done;
        assert_eq!(score(&done), 0.0);
        let mut old = task();
        old.created_at = NOW - 3 * 365 * 86_400_000;
        assert_eq!(score(&old), 2.0);
    }
}
