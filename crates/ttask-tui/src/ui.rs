//! Rendering. Reads the [`Tui`] state; the only things it writes back are
//! layout facts (page size, width, scroll offset).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ttask_core::date::{days_between, format_time, relative_label};
use ttask_core::duration::format_minutes;
use ttask_core::quickadd::{Change, parse_mods};
use ttask_core::recur::{RecurMode, RecurRule};
use ttask_core::{Status, Task};
use unicode_width::UnicodeWidthStr;

use crate::keymap::Action;
use crate::state::{Focus, Mode, PromptKind, SidebarEntry, Tui};

const SIDEBAR_WIDTH: u16 = 26;

pub fn draw(f: &mut Frame<'_>, tui: &mut Tui) {
    let area = f.area();
    tui.width = area.width;
    f.render_widget(Block::new().style(tui.theme.base), area);
    if let Mode::Locked { input, error } = &tui.mode {
        draw_lock(f, tui, area, &input.display(), error.as_deref());
        return;
    }
    let footer_height = match &tui.mode {
        Mode::Prompt(p) if matches!(p.kind, PromptKind::Add { .. } | PromptKind::Modify(_)) => 2,
        _ => 1,
    };
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(footer_height),
    ])
    .areas(area);
    draw_header(f, tui, header);
    if tui.detail_full {
        draw_detail(f, tui, body, true);
    } else {
        let show_detail = tui.detail_visible() && body.width >= SIDEBAR_WIDTH + 60;
        let sidebar_width = if body.width < 70 { 0 } else { SIDEBAR_WIDTH };
        let constraints = if show_detail {
            vec![
                Constraint::Length(sidebar_width),
                Constraint::Min(30),
                Constraint::Percentage(38),
            ]
        } else {
            vec![Constraint::Length(sidebar_width), Constraint::Min(20)]
        };
        let panes = Layout::horizontal(constraints).split(body);
        if sidebar_width > 0 {
            draw_sidebar(f, tui, panes[0]);
        }
        draw_list(f, tui, panes[1]);
        if show_detail {
            draw_detail(f, tui, panes[2], false);
        }
    }
    draw_footer(f, tui, footer);
    match &tui.mode {
        Mode::Help { scroll } => draw_help(f, tui, area, *scroll),
        Mode::Palette(_) => draw_palette(f, tui, area),
        Mode::Confirm { message, .. } => draw_confirm(f, tui, area, message),
        _ => {}
    }
}

fn pane_block<'a>(tui: &Tui, title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused {
        tui.theme.border_focus
    } else {
        tui.theme.border
    };
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
        .title(Span::styled(format!(" {title} "), style))
}

fn draw_header(f: &mut Frame<'_>, tui: &Tui, area: Rect) {
    let mut left = vec![
        Span::styled(" ttask ", tui.theme.accent),
        Span::styled("─ ", tui.theme.dim),
        Span::styled(
            tui.current.name.clone(),
            tui.theme.base.add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" ({})", tui.tasks.len()), tui.theme.dim),
    ];
    if !tui.filter.is_empty() {
        left.push(Span::styled(format!("  / {}", tui.filter), tui.theme.today));
    }
    if !tui.marked.is_empty() {
        left.push(Span::styled(
            format!("  {} marked", tui.marked.len()),
            tui.theme.marked,
        ));
    }
    let mut right = Vec::new();
    if let Some((entry, task)) = &tui.timer {
        let minutes = entry.minutes(tui.app.now_ms());
        right.push(Span::styled(
            format!(
                "{} {} {}m  ",
                tui.theme.glyphs.timer,
                truncate(&task.title, 20),
                minutes
            ),
            tui.theme.today,
        ));
    }
    if tui.settings.encrypted {
        right.push(Span::styled(
            format!("{} ", tui.theme.glyphs.lock),
            tui.theme.ok,
        ));
    }
    let right_width: usize = right.iter().map(|s| s.content.width()).sum();
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width as u16)])
        .areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        r,
    );
}

fn draw_sidebar(f: &mut Frame<'_>, tui: &Tui, area: Rect) {
    let focused = tui.focus == Focus::Sidebar && !tui.detail_full;
    let block = pane_block(tui, "Views", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let height = inner.height as usize;
    let offset = tui.sidebar_idx.saturating_sub(height.saturating_sub(1));
    let width = inner.width as usize;
    let lines: Vec<Line> = tui
        .sidebar
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, entry)| match entry {
            SidebarEntry::Header(name) => Line::from(Span::styled(
                format!("─ {name} "),
                tui.theme.dim.add_modifier(Modifier::BOLD),
            )),
            SidebarEntry::View { view, count, depth } => {
                let name = match view.kind {
                    ttask_app::ViewKind::Project(_) if *depth > 0 => view
                        .name
                        .rsplit('/')
                        .next()
                        .unwrap_or(&view.name)
                        .to_owned(),
                    _ => view.name.clone(),
                };
                let indent = " ".repeat(1 + 2 * *depth as usize);
                let count = if *count > 0 {
                    count.to_string()
                } else {
                    String::new()
                };
                let name_width = width.saturating_sub(indent.len() + count.len() + 1);
                let label = format!("{indent}{}", pad(&truncate(&name, name_width), name_width));
                let selected = i == tui.sidebar_idx;
                let style = if selected && focused {
                    tui.theme.selected
                } else if selected {
                    tui.theme.accent
                } else {
                    tui.theme.base
                };
                Line::from(vec![
                    Span::styled(label, style),
                    Span::styled(
                        format!(" {count}"),
                        if selected && focused {
                            style
                        } else {
                            tui.theme.dim
                        },
                    ),
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_list(f: &mut Frame<'_>, tui: &mut Tui, area: Rect) {
    let focused = tui.focus == Focus::List;
    let title = tui.current.name.clone();
    let block = pane_block(tui, &title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let height = inner.height as usize;
    tui.page = height.saturating_sub(1).max(1);
    if tui.tasks.is_empty() {
        let hint = if tui.filter.is_empty() {
            format!(
                "Nothing here. Press {} to add a task.",
                tui.keymap.keys_for(Action::Add)
            )
        } else {
            "No tasks match the filter (Esc clears it).".to_owned()
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, tui.theme.dim)).alignment(Alignment::Center),
            inner,
        );
        return;
    }
    // Keep the selection on screen.
    if tui.task_idx < tui.list_offset {
        tui.list_offset = tui.task_idx;
    } else if tui.task_idx >= tui.list_offset + height {
        tui.list_offset = tui.task_idx + 1 - height;
    }
    let today = tui.app.today();
    let width = inner.width as usize;
    let lines: Vec<Line> = tui
        .tasks
        .iter()
        .enumerate()
        .skip(tui.list_offset)
        .take(height)
        .map(|(i, (task, depth))| {
            task_row(
                tui,
                task,
                *depth,
                width,
                today,
                i == tui.task_idx && focused,
                i == tui.task_idx,
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn task_row(
    tui: &Tui,
    task: &Task,
    depth: u8,
    width: usize,
    today: ttask_core::jiff::civil::Date,
    selected_focused: bool,
    selected: bool,
) -> Line<'static> {
    let t = &tui.theme;
    let g = t.glyphs;
    let marked = tui.marked.contains(&task.id);
    let check = match task.status {
        Status::Open => g.open,
        Status::Done => g.done,
        Status::Cancelled => g.cancelled,
    };
    let mut left: Vec<Span> = vec![
        Span::styled(if marked { g.marked } else { " " }.to_owned(), t.marked),
        Span::raw("  ".repeat(depth as usize)),
        Span::styled(
            format!("{check} "),
            if task.is_open() { t.base } else { t.dim },
        ),
        Span::styled(format!("{:<3} ", task.priority.marks()), t.priority),
    ];
    let mut title = task.title.clone();
    if task.recur_rule.is_some() {
        title.push(' ');
        title.push_str(g.recur);
    }
    if task.blocked {
        title.push(' ');
        title.push_str(g.blocked);
    }
    // Right-hand metadata.
    let mut meta: Vec<Span> = Vec::new();
    if let Some(project) = task
        .project
        .as_ref()
        .filter(|_| !matches!(tui.current.kind, ttask_app::ViewKind::Project(_)))
    {
        meta.push(Span::styled(format!("#{project} "), t.project));
    }
    for tag in task.tags.iter().take(3) {
        meta.push(Span::styled(format!("+{tag} "), t.tag));
    }
    if task.subtasks.total > 0 {
        meta.push(Span::styled(
            format!("{}/{} ", task.subtasks.done, task.subtasks.total),
            t.dim,
        ));
    } else if task.checklist.total > 0 {
        meta.push(Span::styled(
            format!("{}/{} ", task.checklist.done, task.checklist.total),
            t.dim,
        ));
    }
    if let Some(due) = task.due_date {
        let mut label = relative_label(due, today);
        if let Some(time) = task.due_time {
            label.push(' ');
            label.push_str(&format_time(time));
        }
        let style = if task.is_overdue(today) {
            t.overdue
        } else if due == today && task.is_open() {
            t.today
        } else {
            t.dim
        };
        meta.push(Span::styled(format!("{label:>9}"), style));
    } else if !task.is_open() {
        if let Some(done) = task.completed_at {
            let date = ttask_core::date_of_ms(done, &tui.app.clock().time_zone());
            meta.push(Span::styled(
                format!("{:>9}", relative_label(date, today)),
                t.dim,
            ));
        }
    }
    let left_width: usize = left.iter().map(|s| s.content.width()).sum();
    let meta_width: usize = meta.iter().map(|s| s.content.width()).sum();
    let title_room = width.saturating_sub(left_width + meta_width + 1);
    let title = truncate(&title, title_room);
    let gap = width.saturating_sub(left_width + title.width() + meta_width);
    let title_style = if task.is_open() { t.base } else { t.done };
    left.push(Span::styled(title, title_style));
    left.push(Span::raw(" ".repeat(gap)));
    left.extend(meta);
    let mut line = Line::from(left);
    if selected_focused {
        line = line.patch_style(t.selected);
    } else if selected {
        line = line.patch_style(Style::new().add_modifier(Modifier::BOLD));
    }
    line
}

fn draw_detail(f: &mut Frame<'_>, tui: &Tui, area: Rect, full: bool) {
    let focused = tui.focus == Focus::Detail || full;
    let block = pane_block(tui, "Detail", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(d) = &tui.detail else {
        f.render_widget(
            Paragraph::new(Span::styled("No task selected", tui.theme.dim)),
            inner,
        );
        return;
    };
    let t = &tui.theme;
    let task = &d.task;
    let today = tui.app.today();
    let tz = tui.app.clock().time_zone();
    let label = |s: &str| Span::styled(format!("{s:<11}"), t.dim);
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            task.title.clone(),
            t.base.add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "#{} · {} · priority {}",
                task.id,
                task.status,
                task.priority.name()
            ),
            t.dim,
        )),
        Line::raw(""),
    ];
    if let Some(parent) = &d.parent {
        lines.push(Line::from(vec![
            label("Subtask of"),
            Span::raw(parent.title.clone()),
        ]));
    }
    if let Some(project) = &task.project {
        lines.push(Line::from(vec![
            label("Project"),
            Span::styled(format!("#{project}"), t.project),
        ]));
    }
    if !task.tags.is_empty() {
        let tags: Vec<String> = task.tags.iter().map(|t| format!("+{t}")).collect();
        lines.push(Line::from(vec![
            label("Tags"),
            Span::styled(tags.join(" "), t.tag),
        ]));
    }
    if let Some(due) = task.due_date {
        let mut text = format!("{due} ({})", relative_label(due, today));
        if let Some(time) = task.due_time {
            text = format!(
                "{due} {} ({})",
                format_time(time),
                relative_label(due, today)
            );
        }
        let days = days_between(today, due);
        if task.is_open() && days < 0 {
            text.push_str(&format!(
                " · {} day{} overdue",
                -days,
                if days == -1 { "" } else { "s" }
            ));
        }
        let style = if task.is_overdue(today) {
            t.overdue
        } else {
            t.base
        };
        lines.push(Line::from(vec![label("Due"), Span::styled(text, style)]));
    }
    if let Some(sched) = task.scheduled_date {
        lines.push(Line::from(vec![
            label("Scheduled"),
            Span::raw(format!("{sched} ({})", relative_label(sched, today))),
        ]));
    }
    if let Some(rule) = task
        .recur_rule
        .as_ref()
        .and_then(|r| r.parse::<RecurRule>().ok())
    {
        let mut text = rule.describe();
        if task.recur_mode == Some(RecurMode::Completion) {
            text.push_str(" after completion");
        }
        lines.push(Line::from(vec![label("Repeats"), Span::raw(text)]));
    }
    if task.estimate_min.is_some() || d.tracked_minutes > 0 {
        let mut text = task
            .estimate_min
            .map(|m| format!("est. {}", format_minutes(m)))
            .unwrap_or_default();
        if d.tracked_minutes > 0 {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            text.push_str(&format!(
                "tracked {}",
                format_minutes(d.tracked_minutes as u32)
            ));
        }
        lines.push(Line::from(vec![label("Time"), Span::raw(text)]));
    }
    for blocker in &d.blocked_by {
        let style = if blocker.is_open() { t.overdue } else { t.dim };
        lines.push(Line::from(vec![
            label("Waits on"),
            Span::styled(format!("#{} {}", blocker.id, blocker.title), style),
        ]));
    }
    for dependent in &d.blocking {
        lines.push(Line::from(vec![
            label("Blocks"),
            Span::raw(format!("#{} {}", dependent.id, dependent.title)),
        ]));
    }
    for reminder in d.reminders.iter().filter(|r| r.fired_at.is_none()) {
        let at = ttask_core::jiff::Timestamp::from_millisecond(reminder.fire_at)
            .map(|ts| {
                ts.to_zoned(tz.clone())
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        lines.push(Line::from(vec![label("Reminder"), Span::raw(at)]));
    }
    lines.push(Line::from(vec![
        label("Created"),
        Span::styled(
            ttask_core::date_of_ms(task.created_at, &tz).to_string(),
            t.dim,
        ),
    ]));
    if let Some(done) = task.completed_at {
        lines.push(Line::from(vec![
            label("Closed"),
            Span::styled(ttask_core::date_of_ms(done, &tz).to_string(), t.dim),
        ]));
    }
    if !task.notes.is_empty() {
        lines.push(Line::raw(""));
        for line in task.notes.lines() {
            lines.push(Line::raw(line.to_owned()));
        }
    }
    let item_style = |i: usize| {
        if focused && i == tui.detail_idx {
            t.selected
        } else {
            t.base
        }
    };
    if !d.checklist.is_empty() {
        let done = d.checklist.iter().filter(|c| c.checked).count();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("Checklist {done}/{}", d.checklist.len()),
            t.accent,
        )));
        for (i, item) in d.checklist.iter().enumerate() {
            let mark = if item.checked { "[x]" } else { "[ ]" };
            let style = if item.checked { t.done } else { item_style(i) };
            let style = if focused && i == tui.detail_idx {
                t.selected
            } else {
                style
            };
            lines.push(Line::from(Span::styled(
                format!("{mark} {}", item.text),
                style,
            )));
        }
    }
    if !d.subtasks.is_empty() {
        let offset = d.checklist.len();
        let done = d.subtasks.iter().filter(|s| !s.is_open()).count();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("Subtasks {done}/{}", d.subtasks.len()),
            t.accent,
        )));
        for (i, sub) in d.subtasks.iter().enumerate() {
            let glyph = if sub.is_open() {
                t.glyphs.open
            } else {
                t.glyphs.done
            };
            let style = if sub.is_open() {
                item_style(offset + i)
            } else {
                t.done
            };
            let style = if focused && offset + i == tui.detail_idx {
                t.selected
            } else {
                style
            };
            lines.push(Line::from(Span::styled(
                format!("{glyph} {}", sub.title),
                style,
            )));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_footer(f: &mut Frame<'_>, tui: &Tui, area: Rect) {
    let t = &tui.theme;
    match &tui.mode {
        Mode::Prompt(prompt) => {
            let label = format!("{}: ", prompt.kind.label());
            let input_line = Line::from(vec![
                Span::styled(label.clone(), t.accent),
                Span::raw(prompt.input.display()),
            ]);
            let mut lines = vec![input_line];
            if area.height > 1 {
                lines.push(preview_line(
                    tui,
                    &prompt.kind,
                    &prompt.input.text(),
                    prompt.error.as_deref(),
                ));
            } else if let Some(err) = &prompt.error {
                lines[0]
                    .spans
                    .push(Span::styled(format!("  {err}"), t.error));
            }
            f.render_widget(Paragraph::new(lines), area);
            let x = area.x + label.width() as u16 + prompt.input.cursor_column();
            f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
        }
        _ => {
            if let Some(status) = &tui.status {
                let style = if status.error { t.error } else { t.ok };
                f.render_widget(
                    Paragraph::new(Span::styled(format!(" {}", status.text), style)),
                    area,
                );
                return;
            }
            let hints = [
                (Action::Add, "add"),
                (Action::Edit, "edit"),
                (Action::ToggleDone, "done"),
                (Action::Due, "due"),
                (Action::Filter, "filter"),
                (Action::Palette, "palette"),
                (Action::Help, "help"),
                (Action::Quit, "quit"),
            ];
            let mut spans = vec![Span::raw(" ")];
            for (i, (action, name)) in hints.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" · ", t.dim));
                }
                let key = tui.keymap.first_key(*action);
                spans.push(Span::styled(key, t.accent));
                spans.push(Span::styled(format!(" {name}"), t.dim));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
        }
    }
}

/// Live parse preview under the quick-add / modify prompt.
fn preview_line(tui: &Tui, kind: &PromptKind, text: &str, error: Option<&str>) -> Line<'static> {
    let t = &tui.theme;
    if let Some(err) = error {
        return Line::from(Span::styled(format!("  {err}"), t.error));
    }
    if text.trim().is_empty() {
        return Line::from(Span::styled(
            "  e.g. Pay rent #home +bills !high due:fri every:month ~30m remind:due-1h",
            t.dim,
        ));
    }
    let today = tui.app.today();
    match parse_mods(text, today) {
        Err(e) => Line::from(Span::styled(format!("  {e}"), t.error)),
        Ok(m) => {
            let mut spans = vec![Span::styled("  → ", t.dim)];
            let mut push = |s: String, style: Style| {
                if spans.len() > 1 {
                    spans.push(Span::styled(" · ", t.dim));
                }
                spans.push(Span::styled(s, style));
            };
            match &m.title {
                Some(title) => push(title.clone(), t.base.add_modifier(Modifier::BOLD)),
                None if matches!(kind, PromptKind::Add { .. }) => {
                    push("(needs a title)".into(), t.error)
                }
                None => {}
            }
            match &m.project {
                Some(Change::Set(path)) => push(format!("#{}", path.join("/")), t.project),
                Some(Change::Clear) => push("inbox".into(), t.project),
                None => {}
            }
            for tag in &m.add_tags {
                push(format!("+{tag}"), t.tag);
            }
            for tag in &m.remove_tags {
                push(format!("-+{tag}"), t.tag);
            }
            if let Some(p) = m.priority {
                push(p.name().into(), t.priority);
            }
            match &m.due {
                Some(Change::Set(spec)) => {
                    let mut s = format!("due {} ({})", spec.date, relative_label(spec.date, today));
                    if let Some(time) = spec.time {
                        s.push_str(&format!(" {}", format_time(time)));
                    }
                    push(s, t.today);
                }
                Some(Change::Clear) => push("no due date".into(), t.today),
                None => {}
            }
            if let Some(Change::Set(d)) = &m.sched {
                push(format!("starts {d}"), t.dim);
            }
            if let Some(Change::Set(minutes)) = m.estimate {
                push(format!("~{}", format_minutes(minutes)), t.dim);
            }
            match &m.recur {
                Some(Change::Set((rule, mode))) => {
                    let mut s = rule.describe();
                    if *mode == RecurMode::Completion {
                        s.push_str(" after completion");
                    }
                    push(s, t.accent);
                }
                Some(Change::Clear) => push("no repeat".into(), t.accent),
                None => {}
            }
            if !m.reminders.is_empty() {
                push(
                    format!(
                        "{} reminder{}",
                        m.reminders.len(),
                        if m.reminders.len() == 1 { "" } else { "s" }
                    ),
                    t.dim,
                );
            }
            Line::from(spans)
        }
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn draw_help(f: &mut Frame<'_>, tui: &Tui, area: Rect, scroll: u16) {
    let rect = centered(area, 72, area.height.saturating_sub(4));
    f.render_widget(Clear, rect);
    let block = pane_block(tui, "Help (Esc to close)", true);
    let mut lines = vec![Line::from(Span::styled("Keys", tui.theme.accent))];
    for (action, _, label) in Action::ALL {
        let keys = tui.keymap.keys_for(*action);
        if keys.is_empty() {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(format!("  {keys:<18}"), tui.theme.accent),
            Span::raw((*label).to_owned()),
        ]));
    }
    lines.extend([
        Line::raw(""),
        Line::from(Span::styled("Quick-add", tui.theme.accent)),
        Line::raw("  #project #project/sub  +tag  !low !med !high !urgent (or !0-!3)"),
        Line::raw("  due:fri due:tom@17:00 due:+3d  sched:mon  ~30m"),
        Line::raw("  every:week every:weekday every:mon,thu every:month@15 every!:3d"),
        Line::raw("  remind:due-1h remind:09:00  \\#escaped"),
        Line::raw(""),
        Line::from(Span::styled("Filter", tui.theme.accent)),
        Line::raw("  project:work and (due:<=today or !>=2) and not +someday"),
        Line::raw("  fields: project tag status priority due sched created done text"),
        Line::raw("          has:notes is:recurring is:blocked is:subtask is:overdue"),
    ]);
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0)),
        rect,
    );
}

fn draw_palette(f: &mut Frame<'_>, tui: &Tui, area: Rect) {
    let Mode::Palette(palette) = &tui.mode else {
        return;
    };
    let rect = centered(area, 64, 18);
    f.render_widget(Clear, rect);
    let block = pane_block(tui, "Command palette", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", tui.theme.accent),
            Span::raw(palette.input.display()),
        ])),
        input_area,
    );
    f.set_cursor_position((
        input_area.x + 2 + palette.input.cursor_column(),
        input_area.y,
    ));
    let height = list_area.height as usize;
    let offset = palette.selected.saturating_sub(height.saturating_sub(1));
    let width = list_area.width as usize;
    let lines: Vec<Line> = palette
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, &item_index)| {
            let item = &palette.items[item_index];
            let hint = truncate(&item.hint, 24);
            let label_room = width.saturating_sub(hint.width() + 1);
            let label = pad(&truncate(&item.label, label_room), label_room);
            let line = Line::from(vec![
                Span::raw(label),
                Span::raw(" "),
                Span::styled(hint, tui.theme.dim),
            ]);
            if i == palette.selected {
                line.patch_style(tui.theme.selected)
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list_area);
}

fn draw_confirm(f: &mut Frame<'_>, tui: &Tui, area: Rect, message: &str) {
    let width = (message.width() as u16 + 6).max(30);
    let rect = centered(area, width, 5);
    f.render_widget(Clear, rect);
    let block = pane_block(tui, "Confirm", true);
    let text = vec![
        Line::raw(message.to_owned()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y", tui.theme.accent),
            Span::raw(" yes   "),
            Span::styled("n", tui.theme.accent),
            Span::raw(" no"),
        ]),
    ];
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center),
        rect,
    );
}

fn draw_lock(f: &mut Frame<'_>, tui: &Tui, area: Rect, masked: &str, error: Option<&str>) {
    let rect = centered(area, 50, 7);
    let block = pane_block(tui, "ttask is locked", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![
        Line::raw("Enter the database password to continue."),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Password: ", tui.theme.accent),
            Span::raw(masked.to_owned()),
        ]),
    ];
    if let Some(err) = error {
        lines.push(Line::from(Span::styled(err.to_owned(), tui.theme.error)));
    }
    f.render_widget(Paragraph::new(lines), inner);
    f.set_cursor_position((inner.x + 10 + masked.width() as u16, inner.y + 2));
}

/// Cuts `s` to `width` columns, ending in `…` when shortened.
fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

fn pad(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_owned()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}
