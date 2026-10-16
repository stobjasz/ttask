//! The Agenda view: a Monday–Sunday grid of what is due (or, without a due
//! date, scheduled) each day. The selected day's tasks are the TUI's task
//! list, so every task key works on them.

use ttask_core::Task;
use ttask_core::date::start_of_week;
use ttask_core::jiff::ToSpan;
use ttask_core::jiff::civil::Date;

pub struct Agenda {
    /// Monday of the week shown.
    pub monday: Date,
    /// 0 = Monday.
    pub day: usize,
    pub columns: [Vec<Task>; 7],
}

impl Agenda {
    pub fn new(today: Date) -> Agenda {
        Agenda {
            monday: start_of_week(today),
            day: today.weekday().to_monday_zero_offset() as usize,
            columns: Default::default(),
        }
    }

    pub fn date(&self, day: usize) -> Date {
        self.monday + (day as i64).days()
    }

    pub fn selected_date(&self) -> Date {
        self.date(self.day)
    }

    pub fn shift_week(&mut self, weeks: i64) {
        self.monday += (weeks * 7).days();
    }

    /// Done and open tasks of the week, waiting ones included.
    pub fn query(&self) -> String {
        let (first, last) = (self.date(0), self.date(6));
        format!(
            "not status:cancelled wait:any ((due:>={first} and due:<={last}) or \
             (due:none and sched:>={first} and sched:<={last}))"
        )
    }

    /// Sorts `tasks` (already in list order) into their days.
    pub fn fill(&mut self, tasks: Vec<Task>) {
        self.columns = Default::default();
        let (first, last) = (self.date(0), self.date(6));
        for task in tasks {
            let day = task
                .due_date
                .filter(|d| (first..=last).contains(d))
                .or(task.scheduled_date);
            if let Some(day) = day.filter(|d| (first..=last).contains(d)) {
                let i = ttask_core::date::days_between(first, day) as usize;
                self.columns[i].push(task);
            }
        }
    }
}
