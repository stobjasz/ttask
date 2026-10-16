# ttask

A keyboard-driven personal task tracker for the terminal. Tasks live in one
local SQLite file, plain or encrypted with a password (SQLCipher). Nothing
leaves your machine unless you export it.

User guide: [docs/USER_GUIDE.md](docs/USER_GUIDE.md) · Design: [DESIGN.md](DESIGN.md).

## Quick start

```sh
ttask init                  # or: ttask init --encrypt
ttask add 'Pay rent #home +bills !high due:fri every:month remind:due-1h'
ttask                       # full-screen TUI
```

## Quick-add syntax

Single-quote quick-add text on the command line: shells treat `#word` as a
comment and `!word` as history expansion. Used by `ttask add`, the TUI `a` prompt, and `ttask edit` (where `none`
clears a field and `-+tag` removes a tag).

| Token | Meaning |
| --- | --- |
| `#work`, `#work/clientA`, `#none` | project (one level of nesting) |
| `+tag`, `-+tag` | add / remove a tag |
| `!low` `!med` `!high` `!urgent`, `!0`–`!3`, `!!` | priority |
| `due:fri`, `due:tom@17:00`, `due:+3d`, `due:2026-12-24`, `due:none` | due date |
| `sched:mon` | scheduled (hidden from Today until then) |
| `wait:fri`, `wait:someday`, `wait:none` | hide until a date / park indefinitely / bring back |
| `every:week`, `every:weekday`, `every:mon,thu`, `every:month@15`, `every:month@lastfri`, `every!:3d` | recurrence (`!` = after completion) |
| `~30m`, `~2h` | estimate |
| `remind:due-1h`, `remind:09:00`, `remind:tom@8am` | reminder |
| `\#literal` | escape |

## CLI

```sh
ttask ls                    # open tasks; views: inbox today next upcoming overdue waiting someday all logbook
ttask ls next               # actionable tasks, most urgent first (--sort urgency anywhere)
ttask ls '#work and (due:<=today or !>=2) and not +someday'
ttask ls today --json       # every read command speaks JSON
ttask show 12 --history
ttask edit 12 'due:+3d -+later !urgent New title'
ttask edit 12               # $EDITOR on a TOML form
ttask done 12 13            # recurring tasks spawn their next instance
ttask reopen | cancel | rm | skip <ids>
ttask done --filter '+bills due:<=today' [--yes]   # also reopen/cancel/rm; one undo step
ttask count today          # just the number, for prompts and status bars
ttask open 12 [N] [--all]  # open the task's http/https/mailto link(s)
ttask undo / ttask redo     # every action, including imports
ttask search invoice
ttask checklist 12 add passport ; ttask checklist 12 toggle 1
ttask block 12 --on 9       # 12 waits on 9
ttask timer start 12 ; ttask timer stop
ttask project ls|add|rename|archive|rm ; ttask tag ls|rename|rm
ttask filter save "This week" 'due:<=eow' ; ttask ls "This week"
ttask context define work '#work' ; ttask context none   # focus every list on one area
ttask import tasks.json | todo.txt | export.csv | taskwarrior.json | issues.json [--dry-run] [--map title=Name]
ttask export --format json|csv|markdown|todotxt|ics [--output f] [--encrypt] [--filter Q]
ttask remind [--run | --install | --uninstall]
ttask db info|backup|check|passwd|encrypt|decrypt|gc|gc-backups
ttask gc ; ttask lock ; ttask config [path|init] ; ttask completions powershell
```

Filter fields: `project tag status priority due sched created done text`,
`has:notes|due|subtasks`, `is:recurring|blocked|subtask|overdue`, `id:N`,
bare words (full-text), `"quoted phrases"`, `and`/`or`/`not`, parentheses.
Use `not +tag` (or `--` before an argument starting with `-`).

Exit codes: 0 ok, 1 usage or other error, 2 not found, 3 wrong or missing
password, 4 database locked, corrupt, from a newer ttask, or not a ttask
database (other files are refused before anything is written to them).

## TUI keys (vim preset)

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| `j`/`k` `↓`/`↑` | move | `a` / `A` | quick-add / subtask |
| `h`/`l` `Tab` | switch pane | `e` / `E` | edit title / notes in `$EDITOR` |
| `g`/`G` | top / bottom | `M` | modify with quick-add tokens |
| `Enter` | open / full-screen detail | `x` / `X` | toggle done / cancel |
| `/` | filter view | `1`–`4` | priority low…urgent |
| `f` `Ctrl-f` | full-text search (live) | `d` `m` `t` | due / move / tags |
| `:` | command palette | `c` | add checklist item |
| `u` / `Ctrl-r` | undo / redo | `Space` | mark (list), toggle item (detail) |
| `v` | mark for multi-select | `J`/`K` | reorder (inbox/projects) |
| `s` | skip occurrence | `D` | delete (asks) |
| `T` | start/stop timer | `S` | save current filter |
| `w` | wait / someday | `C` | switch context |
| `o` | open link | `O` `Ctrl-o` | open another database file |
| `b` | waits on (dependencies) | `P` | make subtask of |
| `H` | task history | `y` / `Y` | copy title / id |
| `Ctrl-s` | cycle sort | `L` | lock now (encrypted) |
| `?` | help | `q` | quit |

`keymap = "simple"` drops the vim keys (reorder with `Alt-↑/↓`); any action
can be rebound under `[keys]` in the config.

## Configuration

`ttask config init` writes a commented `config.toml` (path: `ttask config
path`, override with `TTASK_CONFIG`). Highlights:

- `database.path`, `database.backups = { daily = 7, weekly = 4 }` — rotating
  backups in `<db dir>/backups/`, made on the first open each day.
- `security.remember = "keychain"` caches the password in the OS keychain;
  `security.password_cmd = "pass show ttask"`; `security.idle_lock_minutes`
  re-locks the TUI.
- `ui.theme = auto|dark|light|ascii`, `ui.keymap = vim|simple`,
  `ui.show_detail_pane = auto|always|never`. `NO_COLOR` is honoured.

For an encrypted database the password comes from `--password-file`, else
`password_cmd`, the keychain, `TTASK_PASSWORD` (warns), then a prompt. A
forgotten password cannot be recovered.

## Building

```sh
cargo build --release
cargo test --workspace
```

SQLCipher and OpenSSL are compiled from source and linked statically, so the
binary has no runtime dependencies. On Windows (MSVC) the OpenSSL build needs a
native Perl — Git's MSYS perl does not work:

```powershell
winget install StrawberryPerl.StrawberryPerl
$env:OPENSSL_SRC_PERL = 'C:\Strawberry\perl\bin\perl.exe'   # or put Strawberry first on PATH
```

The first build compiles OpenSSL and takes a few minutes; later builds are cached.

**Portable Windows exe:** `.\scripts\build-release.ps1` builds a single
`dist\ttask-<version>-x86_64-pc-windows-msvc.exe` (about 12 MB) with the C
runtime linked statically, so it only depends on Windows system DLLs, plus a
`.sha256` checksum. Copy the exe anywhere and run it.

## Layout

| Crate | Responsibility |
| --- | --- |
| `crates/ttask-core` | Domain types, dates, quick-add, recurrence, filter language; no I/O |
| `crates/ttask-store` | DB opener (plain/encrypted), migrations, repository, undo journal, maintenance |
| `crates/ttask-app` | Services (one undoable transaction per action), views, import/export, backups |
| `crates/ttask-tui` | ratatui TUI: state machine, rendering, keymaps, palette |
| `crates/ttask` | The `ttask` binary: CLI, config, password sources, keychain, reminders |
