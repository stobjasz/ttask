# ttask User Guide

ttask is a personal task tracker for the terminal. You use it two ways:

- **The CLI** — `ttask add …`, `ttask ls`, `ttask done 3` — for quick capture and scripts.
- **The TUI** — run `ttask` with no arguments — a full-screen, keyboard-driven view for planning and review.

Both work on the same local SQLite file, which can be encrypted with a password. Nothing leaves your machine unless you export it.

**Contents**

1. [Quick start](#1-quick-start)
2. [Core ideas](#2-core-ideas)
3. [Installing](#3-installing)
4. [Creating a database](#4-creating-a-database)
5. [Adding tasks: quick-add syntax](#5-adding-tasks-quick-add-syntax)
6. [Finding tasks: views and filters](#6-finding-tasks-views-and-filters)
7. [Changing tasks](#7-changing-tasks)
8. [Projects, tags and saved filters](#8-projects-tags-and-saved-filters)
9. [Subtasks, checklists, dependencies, time tracking](#9-subtasks-checklists-dependencies-time-tracking)
10. [Recurring tasks](#10-recurring-tasks)
11. [Reminders](#11-reminders)
12. [The TUI](#12-the-tui)
13. [Encryption and passwords](#13-encryption-and-passwords)
14. [Backups and maintenance](#14-backups-and-maintenance)
15. [Import and export](#15-import-and-export)
16. [Configuration reference](#16-configuration-reference)
17. [Scripting with ttask](#17-scripting-with-ttask)
18. [Troubleshooting](#18-troubleshooting)

---

## 1. Quick start

Five minutes from nothing to a working task list.

```sh
# 1. Create your database (add --encrypt to protect it with a password)
ttask init

# 2. Capture a few tasks; tokens like #project, +tag, !priority and due: are parsed out.
#    Single-quote the text: shells treat #word as a comment and !word as history.
ttask add 'Pay rent #home +bills !high due:today every:month'
ttask add 'Renew passport #admin due:+10d remind:due-1h ~30m'
ttask add 'Book flights #travel/europe due:fri'
ttask add Call plumber

# 3. See what's open
ttask ls
```

```
   1 [ ] !!  Pay rent ↻  #home  +bills  today
   3 [ ]     Book flights  #travel/europe  Fri
   2 [ ]     Renew passport  #admin  Oct 25
   4 [ ]     Call plumber
```

```sh
# 4. Finish something — a recurring task schedules its next instance
ttask done 1
# Completed task 1: Pay rent (next: 5 on 2026-11-15)

# 5. Changed your mind? Every action can be undone
ttask undo

# 6. Open the full-screen TUI
ttask
```

In the TUI: `j`/`k` (or arrows) move, `a` adds a task, `x` completes it, `d` sets a due date, `/` filters, `:` opens the command palette, `?` shows every key, `q` quits.

That's enough to use ttask daily. The rest of this guide covers each feature in depth.

---

## 2. Core ideas

| Concept | What it is |
| --- | --- |
| **Task** | A title plus optional notes, project, tags, priority, due date (with optional time), scheduled date, estimate, recurrence, reminders. Every task has a short numeric **id** (shown in lists) and a permanent **uid**. |
| **Status** | `open`, `done` or `cancelled`. Lists show open tasks unless you ask otherwise. |
| **Project** | A container like `work`, optionally one level deep: `work/clientA`. Tasks without a project are in the **Inbox**. |
| **Tag** | A free-form label like `+phone`. A task can have many. |
| **Priority** | `low` (the default, no marks), `medium` `!`, `high` `!!`, `urgent` `!!!`. |
| **Due date** | When it must be done. Optionally with a time (`due:fri@17:00`). Overdue tasks are shown in red. |
| **Scheduled date** | When you want to *start* it. A task appears in **Today** from its scheduled date. |
| **View** | A named filter: Inbox, Today, Upcoming, Overdue, All, Logbook — plus every project, tag and saved filter. |
| **Subtask** | A task inside another task (one level). |
| **Checklist** | Lightweight items inside a task (`[ ] passport`), without dates or tags. |
| **Dependency** | "Task B waits on task A". B shows as *blocked* until A is closed. |

---

## 3. Installing

ttask is a single binary with no runtime dependencies.

### From source

You need Rust (stable) and a C toolchain. SQLCipher and OpenSSL are compiled in, so the first build takes a few minutes.

```sh
git clone <repo-url> ttask && cd ttask
cargo build --release
# binary: target/release/ttask (ttask.exe on Windows)
```

**Windows:** the OpenSSL build needs a native Perl (Git's bundled Perl does not work):

```powershell
winget install StrawberryPerl.StrawberryPerl
$env:OPENSSL_SRC_PERL = 'C:\Strawberry\perl\bin\perl.exe'
cargo build --release
```

Put the binary somewhere on your `PATH`.

### Shell completions

```sh
ttask completions bash > ~/.local/share/bash-completion/completions/ttask
ttask completions zsh  > "${fpath[1]}/_ttask"
ttask completions fish > ~/.config/fish/completions/ttask.fish
ttask completions powershell >> $PROFILE
```

---

## 4. Creating a database

```sh
ttask init             # plain SQLite file
ttask init --encrypt   # encrypted; asks for a password twice (minimum 10 characters)
```

**Where it lives** (shown by `ttask config`):

| OS | Default database |
| --- | --- |
| Windows | `%APPDATA%\ttask\data\ttask.db` |
| macOS | `~/Library/Application Support/ttask/ttask.db` |
| Linux | `~/.local/share/ttask/ttask.db` |

Use a different file with `--db <path>` on any command, the `TTASK_DB` environment variable, or `database.path` in the [config file](#16-configuration-reference). This lets you keep separate databases, e.g. an encrypted `work.db` and a plain `home.db`:

```sh
ttask --db ~/tasks/work.db init --encrypt
TTASK_DB=~/tasks/work.db ttask ls
```

> **An encrypted database cannot be recovered without its password.** Store it in a password manager.

---

## 5. Adding tasks: quick-add syntax

```sh
ttask add <title and tokens…> [--notes TEXT] [--parent ID]
```

Everything that is not a token becomes the title. Tokens can appear anywhere:

```sh
ttask add 'Renew passport #admin +gov !high due:2026-11-30 ~30m'
ttask add 'Fix the sink #home due:sat'
ttask add 'Draft report due:fri' --notes "Use the Q3 numbers"
```

> **Quote quick-add text in single quotes.** In bash, zsh, fish and PowerShell an unquoted word starting with `#` begins a comment, so `ttask add Fix sink #home` silently drops `#home`. In bash and zsh, `!high` can trigger history expansion, even inside double quotes. Single quotes avoid both. (In `cmd.exe`, use double quotes.) Text without `#` or `!` needs no quotes.

### Token reference

| Token | Meaning | Examples |
| --- | --- | --- |
| `#name` | Project (created on first use) | `#home`, `#work/clientA` |
| `+name` | Tag | `+phone +errand` |
| `!…` | Priority | `!low` `!med` `!high` `!urgent`, `!0`–`!3`, `!` (medium), `!!` (high), `!!!` (urgent) |
| `due:` | Due date, optional `@time` | `due:fri`, `due:tom@17:00`, `due:+2w`, `due:2026-12-24` |
| `sched:` (or `start:`) | Scheduled start date | `sched:mon` |
| `every:` | Repeats on schedule | `every:week`, `every:weekday`, `every:month@15` |
| `every!:` | Repeats after completion | `every!:3d` |
| `~` | Estimate | `~30m`, `~2h`, `~1h30m` |
| `remind:` | Reminder (repeatable) | `remind:due-1h`, `remind:09:00`, `remind:tom@8am` |
| `\` | Escape: keep the word literally | `\#123`, `\+1` |

Tags start with a letter or `_` and may contain letters, digits, `_` and `-`. Project names cannot contain `/` (it separates parent and subproject).

### Date phrases

Used by `due:`, `sched:`, `remind:`, filters and the TUI date prompt. "Today" means your local calendar date.

| Phrase | Meaning |
| --- | --- |
| `today`, `tod` | today |
| `tomorrow`, `tom`, `tmr` | tomorrow |
| `yesterday` | yesterday |
| `mon` … `sun`, `monday` … | the next such day, **counting today** |
| `+3d`, `3d`, `+2w`, `+1m`, `+1y`, `-1d` | relative days / weeks / months / years |
| `eow`, `eom`, `eoy` | end of this week (Sunday) / month / year |
| `nw` (`next-week`), `nm` (`next-month`) | next Monday / the 1st of next month |
| `2026-12-24` | an exact date |
| `12-24` | this year, or next year if already past |

Add a time with `@`: `@17:00`, `@9:30`, `@17`, `@5pm`, `@5:30pm`. A time alone means today: `due:@18:00`.

### What happens on add

```
$ ttask add Book flights #travel/europe due:fri
Added task 3: Book flights (due 2026-10-16)
```

- Unknown projects and tags are created.
- If you set `every:` without `due:`, the due date becomes the first occurrence from today (`every:mon,wed` on a Thursday → next Monday).
- A mistyped token is an error and nothing is added: `` error: `due:someday`: unknown date phrase ``.

---

## 6. Finding tasks: views and filters

```sh
ttask ls [VIEW | SAVED-FILTER | QUERY…] [--all] [--sort ORDER] [--limit N] [--json]
```

Without arguments, `ls` shows every open task.

### Built-in views

| View | Shows |
| --- | --- |
| `inbox` | Open tasks without a project (manual order) |
| `today` | Due today or earlier, or scheduled today or earlier — i.e. what to work on now |
| `upcoming` | Due or scheduled after today |
| `overdue` | Open and past due |
| `all` | Every open task |
| `logbook` (or `done`) | Done and cancelled tasks, most recent first |

```sh
ttask ls today
ttask ls logbook --limit 20
```

### Filter queries

Anything that is not a view or saved-filter name is a **filter query**:

```sh
ttask ls '#work and (due:<=today or !>=2) and not +someday'
ttask ls +phone
ttask ls invoice              # full-text search in titles and notes
```

Quote the query in your shell when it contains `#`, `(`, `<`, `>` or `!`.

**Terms**

| Term | Matches |
| --- | --- |
| `+tag`, `tag:x`, `tag:none` | has tag / has no tags |
| `#name`, `project:name`, `#a/b` | in project (a bare name also matches its subprojects) |
| `#inbox`, `project:none` | no project |
| `status:open`, `status:done`, `status:cancelled`, `status:all` | status |
| `priority:2`, `!>=2`, `!high` | priority (0 low … 3 urgent) |
| `due:today`, `due:<=fri`, `due:>+7d`, `due:none`, `due:any` | due date |
| `sched:<=today`, `sched:none` | scheduled date |
| `created:>=-7d`, `done:today` | creation / completion date |
| `text:"two words"`, `invoice`, `"exact phrase"` | full-text (prefix match: `inv` finds *invoice*) |
| `has:notes`, `has:due`, `has:subtasks` | presence |
| `is:recurring`, `is:blocked`, `is:subtask`, `is:overdue`, `is:open`, `is:done` | state |
| `id:12` | one task |

Comparisons: `=` (default), `<`, `<=`, `>`, `>=`, followed by any [date phrase](#date-phrases) or number.

**Combining:** terms side by side are ANDed. Use `and`, `or`, `not` and parentheses; `not` binds tightest, then `and`, then `or`. A leading `-` also negates (`-+someday`), but on the command line write `not +someday` (or put `--` before the argument), because arguments starting with `-` look like options.

**Status:** a query that doesn't mention status shows only open tasks. Add `--all` or `status:all` to include closed ones.

### Sorting

`--sort` takes:

| Order | Meaning |
| --- | --- |
| `smart` (default) | due date (undated last), then priority, then manual order |
| `manual` | your own order (set with `J`/`K` in the TUI) |
| `priority` | highest priority first |
| `created` | newest first |
| `completed` | most recently closed first |
| `title` | alphabetical |

### Reading a list

```
   1 [ ] !!  Pay rent ↻  #home  +bills  today
   3 [ ]     Book flights  #travel/europe  [0/1]  Sun
   6   [ ]     Compare prices  #travel/europe
   2 [ ]     Renew passport  #admin  +gov  (0/1)  Oct 25
```

id · status (`[ ]` open, `[x]` done, `[-]` cancelled) · priority marks · title · `↻` repeats · project · tags · `[done/total]` subtasks or `(done/total)` checklist · `blocked` · due date. Subtasks are indented under their parent. Overdue dates are red and today's are yellow on a colour terminal.

### Search

```sh
ttask search passport          # open tasks, best match first
ttask search passport --all    # include closed tasks
```

Matches are highlighted with `[…]` in a snippet under each result.

---

## 7. Changing tasks

### Edit with tokens

`ttask edit` takes the same tokens as quick-add, plus ways to remove things:

```sh
ttask edit 12 'due:+3d !urgent'             # change due date and priority
ttask edit 12 -+later +q4                   # remove tag "later", add "q4"
ttask edit 12 'due:none sched:none #none'   # clear due, scheduled and project
ttask edit 12 every:none remind:none ~0     # stop repeating, drop reminders and estimate
ttask edit 12 Final report title            # leftover words replace the title
ttask edit 12 13 14 +review                 # several tasks at once
ttask edit 12 --notes "New notes"           # replace notes
ttask edit 12 --parent 4                    # make it a subtask of 4 (--parent 0: top level)
```

Put `--notes`/`--parent` **before** the change tokens (`edit` accepts tokens beginning with `-`, so options after them would be read as tokens).

### Edit in your editor

`ttask edit 12` with nothing else opens a form in `$EDITOR`:

```toml
title = "Renew passport"
project = "admin"     # name or parent/name
tags = ["gov"]
priority = "low"   # low, medium, high, urgent
due = "2026-10-25"         # 2026-11-30, 2026-11-30@17:00, fri, +3d
scheduled = ""
estimate = "30m"    # 30m, 2h
repeat = ""      # week, weekday, month@15, FREQ=...; prefix ! for after completion
notes = '''
bring 2 photos
'''
```

Save and close to apply; close without changes to cancel. Empty strings clear a field. See [Troubleshooting](#18-troubleshooting) if your editor returns immediately.

### Status changes

```sh
ttask done 3 4        # complete (recurring tasks spawn their next instance)
ttask reopen 3        # back to open
ttask cancel 5        # closed, but not done
ttask rm 6            # delete (with its subtasks); undoable, purged after 30 days
ttask skip 7          # recurring: jump to the next occurrence without completing
```

Each id is handled separately: `ttask done 1 99` completes task 1 and reports that 99 doesn't exist (exit code 2).

Completing a task also completes its open subtasks.

### Undo, redo, history

Every change — including bulk edits, completions with their spawned instance, deletions and whole imports — is one undo step:

```sh
ttask undo     # Undid completed, spawned: Pay rent
ttask redo
ttask show 12 --history
```

```
  History
  2026-10-15 21:24  created
  2026-10-15 21:30  updated: due_date, priority
  2026-10-16 08:02  completed
```

A new change after an undo discards the redo history, as in an editor.

### Show a task

```sh
ttask show 2
```

```
#2 Renew passport  [open]
  project   admin
  tags      +gov
  priority  low
  due       2026-10-25 (Oct 25)
  estimate  30m
  reminder  2026-10-25 08:00
  created   2026-10-15 21:24
  uid       01M50GBXEGT6N9HQ0A5KBV9VWT

  Checklist 0/1
    1. [ ] photos
```

`show` also lists subtasks, dependencies (`waits on`, `blocks`), recurrence, tracked time and notes.

---

## 8. Projects, tags and saved filters

### Projects

Projects are created automatically by `#name`, or explicitly:

```sh
ttask project ls [--archived]          # with open-task counts
ttask project add work/clientA
ttask project rename work/clientA Initech
ttask project archive work             # hide it (and its subprojects) from lists
ttask project unarchive work
ttask project rm work                  # tasks move to the parent project, or the Inbox
```

Projects nest **one** level (`parent/child`). Project names are case-insensitive: `#Work` and `#work` are the same project.

### Tags

```sh
ttask tag ls
ttask tag rename phone call     # merges into "call" if it already exists
ttask tag rm someday            # removes the tag from every task
```

### Saved filters

Save any query under a name and use it like a built-in view — on the command line and in the TUI sidebar:

```sh
ttask filter save "This week" 'due:<=eow and not +someday'
ttask ls "This week"
ttask filter ls
ttask filter rm "This week"
```

---

## 9. Subtasks, checklists, dependencies, time tracking

### Subtasks

A subtask is a full task (own dates, tags, priority) inside a parent. Subtasks nest one level.

```sh
ttask add Compare prices --parent 3     # inherits the parent's project
ttask edit 6 --parent 0                 # promote to top level
```

Lists show subtasks indented under their parent, and the parent shows progress `[1/3]`. Completing or deleting a parent does the same to its open subtasks.

### Checklists

For quick steps that don't need dates:

```sh
ttask checklist 2 add bring photos     # alias: ttask cl
ttask checklist 2                       # list items, numbered
ttask checklist 2 toggle 1
ttask checklist 2 rm 1
```

Recurring tasks copy their checklist (unchecked) to each new instance.

### Dependencies

```sh
ttask block 12 --on 9     # 12 waits on 9
ttask unblock 12 --on 9
ttask ls is:blocked
```

A blocked task shows `blocked` (⧗ in the TUI) until every task it waits on is done or cancelled. ttask refuses dependencies that would form a cycle.

### Time tracking

```sh
ttask timer start 12      # stops any other running timer
ttask timer status        # Timing 12: Prep slides for 25 min
ttask timer stop
```

Tracked time appears in `ttask show` and in the TUI detail pane; the TUI header shows the running timer.

---

## 10. Recurring tasks

A recurring task is a single open task. When you complete it, ttask creates the next instance (same title, notes, project, tags, priority, estimate, unchecked checklist, and reminders shifted by the same offset).

### Patterns

| `every:` value | Meaning |
| --- | --- |
| `day`, `3d` | every day / every 3 days |
| `weekday`, `weekend` | Mon–Fri / Sat–Sun |
| `week`, `2w` | every week / every 2 weeks, on the due date's weekday |
| `mon`, `mon,wed,fri` | on those weekdays |
| `month`, `3m` | every month / 3 months, on the due date's day (clamped: the 31st becomes the 30th or 28th) |
| `month@15` | the 15th of every month |
| `month@last` | the last day of every month |
| `month@2tue`, `month@lastfri` | the 2nd Tuesday / last Friday |
| `year`, `2y` | every year / 2 years |
| `FREQ=WEEKLY;INTERVAL=2;BYDAY=MO;COUNT=6` | a raw RFC 5545 rule (supports `FREQ`, `INTERVAL`, `BYDAY`, `BYMONTHDAY`, `BYSETPOS`, `UNTIL`, `COUNT`) |

### On schedule vs after completion

- **`every:`** keeps the schedule. The next due date is the first occurrence after the old due date **that is also after today**. If you missed three weekly reviews and complete the overdue one today, you get *one* next review on the regular weekday — not three. A late daily task becomes due tomorrow.
- **`every!:`** counts from the day you complete it: `every!:3d` done on Thursday is next due on Sunday. Good for "water the plants".

### Other controls

```sh
ttask skip 7               # move to the next occurrence without completing
ttask edit 7 every:none    # stop repeating (keeps the current instance)
ttask edit 7 every:2w      # change the pattern
```

Recurring tasks show `↻`; `ttask show` describes the rule ("weekly on Mon, Thu", "monthly on the 2nd Tue after completion"). A `COUNT`-limited rule stops after the last occurrence.

---

## 11. Reminders

### Setting reminders

```sh
ttask add 'Dentist due:2026-11-03@10:00 remind:due-1h'
ttask add Call mom remind:tom@18:00
ttask edit 4 remind:09:00          # 09:00 on the due date (or today if none)
ttask edit 4 remind:none           # remove all reminders
```

| `remind:` | Fires at |
| --- | --- |
| `due` | the due time |
| `due-1h`, `due-30m`, `due-1d` | before the due time |
| `09:00`, `9am` | that time on the due date (today if no due date) |
| `tom@8am`, `2026-11-02@19:00` | that date and time |

Date-only due dates count as **09:00** for reminders: `remind:due-1h` on `due:fri` fires Friday at 08:00. A task can have several reminders. Each fires once, only while the task is open.

### Getting notified

Reminders arrive as desktop notifications from two places:

1. **The TUI**, while it's open — it checks every 30 seconds and also shows the reminder in the status line.
2. **A scheduled job**, so reminders work when ttask isn't running:

```sh
ttask remind --install      # run `ttask remind --run` every 5 minutes
ttask remind --uninstall
ttask remind                # list upcoming reminders
ttask remind --run          # fire anything due now (what the job runs)
```

| OS | `--install` creates |
| --- | --- |
| Windows | a Task Scheduler task "ttask reminders" (runs headless, no console flash) |
| macOS | a launchd agent `~/Library/LaunchAgents/com.ttask.remind.plist` |
| Linux | a systemd user timer `ttask-remind.timer` (or prints a crontab line if systemd isn't available) |

The job registers the database path you used with `--install`. For an **encrypted** database the job can't prompt for a password, so enable the keychain first (`remember = "keychain"`, see [§13](#13-encryption-and-passwords)) and open the database once interactively so the password is cached.

---

## 12. The TUI

Start it with `ttask` (no arguments). It needs an interactive terminal of at least 80×24.

```
 ttask ─ Today (3)                                                                        [L]
╭ Views ─────────────────╮╭ Today ──────────────────────────────────╮╭ Detail ──────────────────────╮
│─ Views                 ││ ○     Renew passp… #admin +gov yesterday││Renew passport                │
│ Inbox                 1││ ○ !!  Pay re… #home +bills 0/1     today││#4 · open · priority low      │
│ Today                 3││ ○     Call plumber       #home     today││                              │
│ Upcoming              2││                                         ││Project    #admin             │
│ Overdue               1││                                         ││Tags       +gov               │
│ All                   5││                                         ││Due        2026-10-14         │
│ Logbook                ││                                         ││(yesterday) · 1 day overdue   │
│─ Projects              ││                                         ││Created    2026-10-15         │
│ home                  2││                                         ││                              │
│ admin                 1││                                         ││                              │
│ travel                 ││                                         ││                              │
│   europe              1││                                         ││                              │
│─ Tags                  ││                                         ││                              │
│ +bills                1││                                         ││                              │
╰────────────────────────╯╰─────────────────────────────────────────╯╰──────────────────────────────╯
 a add · e edit · x done · d due · / filter · : palette · ? help · q quit
```

- **Header:** current view, task count, active filter, marked count, running timer, and `[L]` for an encrypted database.
- **Sidebar:** built-in views with open counts, projects (subprojects indented), tags in use, saved filters.
- **List:** the tasks of the selected view; subtasks indented under their parent.
- **Detail:** everything about the selected task, including notes, checklist and subtasks. It appears when the terminal is at least 100 columns wide; on narrower terminals press `Enter` for a full-screen detail view.
- **Footer:** key hints, status messages, or the active prompt.

The TUI refreshes automatically when another process (e.g. `ttask add` in another terminal) changes the database.

### Keys (default vim preset)

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑` | move down / up |
| `g` / `G`, `Home` / `End` | top / bottom |
| `Ctrl-d` / `Ctrl-u`, `PgDn` / `PgUp` | page down / up |
| `h` / `l`, `←` / `→`, `Tab` / `Shift-Tab` | switch pane |
| `Enter` | sidebar: go to the list · list: toggle full-screen detail |
| `Esc` | close detail → clear marks → clear filter → leave search |
| `a` | quick-add (into the project you're viewing) |
| `A` | add a subtask to the selected task |
| `e` | edit the title |
| `E` | edit notes in `$EDITOR` |
| `M` | modify with quick-add tokens (`due:+2d -+later !high`) |
| `x` | toggle done (spawns the next instance for recurring tasks) |
| `X` | cancel |
| `1` `2` `3` `4` | priority low / medium / high / urgent |
| `d` | set due date |
| `m` | move to project (`Tab` completes project names) |
| `t` | edit tags |
| `c` | add checklist items |
| `Space` | list: mark the task · detail: toggle the checklist item or subtask |
| `v` | mark / unmark for multi-select |
| `J` / `K` | move the task down / up (Inbox and project views) |
| `s` | skip a recurring task to its next occurrence |
| `D`, `Delete` | delete (asks for confirmation) |
| `T` | start / stop the timer on the selected task |
| `/` | filter the current view |
| `Ctrl-f` | full-text search (includes closed tasks) |
| `S` | save the current view + filter as a saved filter |
| `:`, `Ctrl-p` | command palette |
| `u`, `Ctrl-z` | undo |
| `Ctrl-r`, `Ctrl-y` | redo |
| `?`, `F1` | help |
| `q` | quit · `Ctrl-c` quits from anywhere |

The `simple` keymap (`keymap = "simple"`) removes the vim letters (`j k g G h l J K Ctrl-d Ctrl-u`); use arrows, `Home`/`End`, `PgUp`/`PgDn`, and `Alt-↑`/`Alt-↓` to reorder. Any action can be rebound ([§16](#16-configuration-reference)).

### Prompts

Several keys open a one-line prompt in the footer. Editing keys work as in a shell: `←`/`→`, `Home`/`End` (`Ctrl-a`/`Ctrl-e`), `Ctrl-←`/`Ctrl-→` by word, `Ctrl-w` delete word, `Ctrl-u` clear to start, `Ctrl-k` clear to end. `Enter` submits, `Esc` cancels.

- **Quick-add (`a`, `A`)** shows a live preview of how your text will be parsed:
  ```
  Add: Pay rent #home !high due:fri every:month
    → Pay rent · #home · high · due 2026-10-16 (tomorrow) · monthly
  ```
  Mistakes appear in red before you press Enter. The prompt stays open after each task, so you can add several in a row; `Esc` closes it.
- **Due (`d`)** accepts any date phrase, with a time after a space or `@` (`tom 17:00`, `fri@9am`). Empty or `none` clears the date.
- **Move (`m`)** takes `project` or `parent/child`; empty or `inbox` moves to the Inbox. `Tab` completes and cycles existing projects.
- **Tags (`t`)** shows the current tags; the line you submit becomes the new tag set (`+a +b` or `a b`).
- **Filter (`/`)** narrows the list as you type. The filter is combined with the view, stays active (shown in the header) until `Esc`, and can be saved with `S`.
- **Checklist (`c`)** stays open so you can add several items.

### Multi-select

Mark tasks with `v` (or `Space` in the list), then `x`, `X`, `1`–`4`, `d`, `m`, `t`, `M` or `D` act on all marked tasks. `Esc` clears the marks.

### Command palette

`:` opens a fuzzy finder over every action and every view, project, tag and saved filter. Type a few letters (`tdy` → *View: Today*, `undo`, `wor` → *Project: work*), pick with `↑`/`↓`, run with `Enter`.

### Idle lock

For an encrypted database with `idle_lock_minutes` set, the TUI hides all tasks after that much inactivity and asks for the password again.

### Themes

`theme = "auto"` (your terminal's colours), `"dark"`, `"light"`, or `"ascii"` (no colours, ASCII symbols: `( )`, `(x)`, `~` for repeating). `NO_COLOR=1` or `--no-color` removes colour from any theme.

---

## 13. Encryption and passwords

An encrypted database uses SQLCipher: every page of the file — tasks, notes, tags, search index — is encrypted with AES-256 and authenticated, with a key derived from your password. Without the password the file is unreadable; a tampered file fails to open.

### Where the password comes from

When a command needs to open an encrypted database, ttask tries, in order:

1. **`--password-file <path>`** — if given, *only* this is used (scripts fail fast instead of prompting).
2. **`password_cmd`** in the config, e.g. `pass show ttask` or `op read op://Private/ttask/password`. The first line of its output is the password.
3. **The OS keychain**, if `remember = "keychain"`.
4. **`TTASK_PASSWORD`** environment variable (prints a warning — it's visible to other processes and in shell history).
5. **An interactive prompt** — three attempts. Without a terminal, ttask stops with exit code 3 instead of waiting.

### Remembering the password

```toml
[security]
remember = "keychain"
```

After the next successful unlock the password is stored in Windows Credential Manager, the macOS Keychain, or the Linux Secret Service (GNOME Keyring / KWallet), keyed by the database path. `ttask lock` removes it. If you change the password elsewhere, the stale entry is discarded automatically.

### Changing encryption

```sh
ttask db encrypt      # plain → encrypted (asks for a new password)
ttask db passwd       # change the password
ttask db decrypt      # encrypted → plain (asks you to type "decrypt", or pass --yes)
```

Each one writes a new file and swaps it in atomically. The old file is kept as `ttask.db.bak` until you run `ttask db gc-backups`. Delete it once you're happy, especially after `passwd`: the `.bak` still opens with the old password.

### What encryption does and doesn't protect

- ✔ A stolen laptop or disk, a leaked backup, a file synced to the cloud.
- ✔ Tampering — detected on open.
- ✘ Malware running as you while ttask is unlocked.
- ✘ A weak password. ttask requires 10+ characters and rates its strength; a long passphrase is best.
- ✘ Files you create: plain exports (use `export --encrypt`), and notes while open in `$EDITOR` (a temporary file, deleted afterwards).
- The file's size and modification time are visible.

---

## 14. Backups and maintenance

### Automatic backups

On the first run each day, ttask copies the database to `backups/` next to it:

- `ttask-daily-YYYY-MM-DD.db` — the last **7** kept
- `ttask-weekly-YYYY-Www.db` — the last **4** kept

Backups keep the database's encryption (same password). Change the counts or folder in the config; set both counts to `0` to turn it off.

Before upgrading the database format, a new ttask version also saves `ttask.db.pre-vN.bak`.

### Manual backup and restore

```sh
ttask db backup                     # → backups/ttask-manual-<timestamp>.db
ttask db backup ~/safe/tasks.db     # never overwrites an existing file
```

Backups are safe while the TUI is open. **To restore**, close ttask and copy the backup over your database file (`ttask config` shows the path), then delete any `ttask.db-wal` / `ttask.db-shm` files left next to it.

### Checking and cleaning

```sh
ttask db info      # path, encryption, size, schema version, task counts
ttask db check     # integrity check; exit code 4 and the backup folder if anything is wrong
ttask gc           # purge tasks deleted >30 days ago, history >180 days old, unused tags; compact the file
```

### Cloud-synced folders

Keeping the live database in Dropbox/OneDrive/iCloud can produce conflicted copies if two machines write at once. Prefer keeping it local and pointing `backups.dir` (or `ttask db backup`) at the synced folder.

---

## 15. Import and export

### Export

```sh
ttask export                                   # JSON to stdout
ttask export --format json --output tasks.json
ttask export --format csv --output tasks.csv
ttask export --format markdown --output tasks.md
ttask export --format todotxt --output todo.txt
ttask export --encrypt --output tasks.age      # encrypted (asks for a passphrase)
```

| Format | Use it for | Contains |
| --- | --- | --- |
| `json` | backups, moving between machines | everything: projects, tags, tasks, subtasks, checklists, reminders, dependencies, time log, saved filters, history |
| `csv` | spreadsheets | one row per task: id, uid, title, status, priority, project, tags, dates, estimate, recurrence, parent, timestamps, notes |
| `markdown` | reading, sharing | a report grouped by project with checkboxes and notes |
| `todotxt` | [todo.txt](https://github.com/todotxt/todo.txt) apps | `(A) 2026-10-01 Title +project @tag due:… t:… rrule:… uid:…` |

Exporting an encrypted database to a plain file prints a warning. `--encrypt` writes an [age](https://age-encryption.org) file protected by a passphrase; `TTASK_EXPORT_PASSPHRASE` supplies it non-interactively.

### Import

```sh
ttask import tasks.json --dry-run    # report what would change, write nothing
ttask import tasks.json
ttask import todo.txt
ttask import tasks.age               # encrypted exports are detected and decrypted
ttask undo                           # reverts the whole import
```

The format is detected from the file; force it with `--format json|csv|todotxt|todoist|ticktick`.

- **Re-importing updates instead of duplicating.** Tasks are matched by `uid`, which JSON, CSV and todo.txt exports all carry.
- `--project NAME` puts tasks that have no project into `NAME`.
- Invalid rows (no title, bad tag) are skipped with a warning; the rest is imported.

**CSV from other tools.** Common headers are recognised: *Title/Task/Name/Content*, *Notes/Description*, *Project/List/Folder*, *Tags/Labels*, *Priority*, *Due/Due date/Deadline/Date*, *Scheduled/Start*, *Status/Done/Completed*, *Estimate/Duration*, *Repeat/Recurrence/RRULE*. Map anything else explicitly:

```sh
ttask import export.csv --map title=Name,due=Deadline,project=Board
```

Field names for `--map`: `uid title notes project tags priority due due_time scheduled status estimate recur recur_mode parent created updated completed_at`.

**Todoist.** Use Todoist's per-project CSV export. The file name becomes the project (override with `--project`), sections become subprojects, indented tasks become subtasks, `@labels` become tags, p1–p4 map to urgent–low, and dates like `every monday` become recurrences. Dates ttask can't read are kept in the task's notes.

**TickTick.** Use TickTick's backup CSV (Settings → Backup). Folder/List become the project, tags, notes, due/start dates, priority, repeat rules, completion and parent/child links are imported.

---

## 16. Configuration reference

Create a commented config with `ttask config init`; `ttask config path` prints its location (Windows `%APPDATA%\ttask\config\config.toml`, macOS `~/Library/Application Support/ttask/config.toml`, Linux `~/.config/ttask/config.toml`). `TTASK_CONFIG` points to a different file. Every key is optional; unknown keys are rejected so typos don't go unnoticed.

```toml
[database]
path = "~/tasks/ttask.db"            # default: platform data directory
backups = { daily = 7, weekly = 4, dir = "~/Dropbox/ttask-backups" }

[security]
remember = "keychain"                # "never" (default) | "keychain"
idle_lock_minutes = 30               # TUI re-locks encrypted databases after inactivity
password_cmd = "pass show ttask"     # first line of output is the password

[ui]
theme = "auto"                       # auto | dark | light | ascii
keymap = "vim"                       # vim | simple
show_detail_pane = "auto"            # auto (≥100 columns) | always | never

[keys]                               # rebind TUI actions
done = "ctrl-x"
quick_add = "n"
```

### Rebinding keys

Under `[keys]`, write `action = "key"`. The key replaces that action's default keys, and is removed from whatever action used it before.

**Keys:** a character (`x`, `J`, `/`), or `enter`, `esc`, `tab`, `shift-tab`, `space`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `delete`, `backspace`, `f1`–`f12`, with optional `ctrl-` / `alt-` prefixes (`ctrl-f`, `alt-up`).

**Actions:** `up` `down` `top` `bottom` `page_up` `page_down` `focus_next` `focus_prev` `focus_left` `focus_right` `open` `quick_add` `add_subtask` `edit` `edit_notes` `modify` `done` `cancel` `priority_1` `priority_2` `priority_3` `priority_4` `due` `move` `tags` `filter` `search` `palette` `undo` `redo` `mark` `reorder_up` `reorder_down` `delete` `skip` `checklist_add` `toggle_item` `timer` `save_filter` `help` `quit` `back`.

### Environment variables

| Variable | Effect |
| --- | --- |
| `TTASK_DB` | database path (same as `--db`) |
| `TTASK_CONFIG` | config file path |
| `TTASK_PASSWORD` | database password (warns; prefer `--password-file` or the keychain) |
| `TTASK_EXPORT_PASSPHRASE` | passphrase for `export --encrypt` and importing `.age` files |
| `NO_COLOR` | disable colour |
| `VISUAL`, `EDITOR` | editor for notes and the edit form (default: Notepad on Windows, `vi` elsewhere) |

---

## 17. Scripting with ttask

- **`--json`** on any read command (`ls`, `show`, `search`, `add`, `done`, `project ls`, `db info`, …) prints machine-readable JSON:
  ```sh
  ttask --json ls today | jq -r '.[] | "\(.id)\t\(.title)"'
  ttask --json add "Ping Bob due:tom" | jq .id
  ```
- **`--quiet` / `-q`** prints only errors.
- **`--no-color`** (or `NO_COLOR=1`) for plain text.
- **Passwords:** use `--password-file` (with restrictive file permissions) or `password_cmd`; never put passwords on the command line.
- **Exit codes:**

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | usage error, invalid input, or another error |
| 2 | not found (task, project, database file) |
| 3 | wrong or missing password |
| 4 | database locked, corrupt, or from a newer ttask |

Examples:

```sh
# Capture from anywhere
alias t='ttask add'
t "Reply to Anna +email due:tom"

# Morning summary
ttask ls today --no-color

# Nightly encrypted off-site export
TTASK_EXPORT_PASSPHRASE="$(pass show ttask-export)" \
  ttask --password-file ~/.ttask-pw export --encrypt --output ~/Dropbox/ttask-$(date +%F).age
```

---

## 18. Troubleshooting

**"No database at …; run `ttask init` first"** — no database exists at that path yet. Check `ttask config` for the path ttask is using, or pass `--db`.

**"wrong password" (exit 3)** — three prompt attempts failed, or a `--password-file` / `TTASK_PASSWORD` is wrong. If the keychain cached an old password it's removed automatically; run the command again and type the password.

**"no terminal to prompt on"** — an encrypted database was opened from a script or scheduled job. Use `--password-file`, `password_cmd`, or `remember = "keychain"`.

**"the editor … returned immediately"** — your editor handed the file to another window and exited (Windows 11 Notepad, VS Code without `--wait`), so ttask can't see your changes. Set an editor that waits:
```powershell
$env:EDITOR = "code --wait"              # VS Code (PowerShell; add to $PROFILE)
$env:EDITOR = "notepad++ -multiInst -nosession"
```
```sh
export EDITOR="code --wait"     # or nano, vim, micro, hx …
```

**"the database is locked by another process" (exit 4)** — another ttask is writing right now. It retries for 3 seconds; try again. A crashed process can't hold the lock.

**"schema version … is newer than this ttask supports"** — the database was opened with a newer ttask. Upgrade ttask.

**`#project` disappears from my `add`, or bash says "event not found"** — the shell treated `#…` as a comment or `!…` as history. Single-quote the text: `ttask add 'Fix sink #home !high'`.

**`ttask ls -+someday` gives "unexpected argument"** — write `ttask ls not +someday`, or `ttask ls -- -+someday`.

**The TUI shows no colours / odd symbols** — use `theme = "ascii"` for terminals without Unicode, and check `NO_COLOR` isn't set.

**Reminders don't fire** — check `ttask remind` lists them; run `ttask remind --run` by hand; for encrypted databases make sure the password is in the keychain. On Linux the keychain needs a running Secret Service (GNOME Keyring or KWallet).

**I deleted something by mistake** — `ttask undo` (repeat it to step further back). If you've made other changes since, restore the task from a backup ([§14](#14-backups-and-maintenance)); deleted tasks stay in the database until `ttask gc` purges them after 30 days, so recent backups contain them too.

**Something looks corrupt** — `ttask db check`, then restore the newest file from `backups/` ([§14](#14-backups-and-maintenance)).
