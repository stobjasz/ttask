# ttask

A keyboard-driven personal task tracker for the terminal. Tasks live in one
local SQLite file, plain or encrypted with a password (SQLCipher).

Design: [DESIGN.md](DESIGN.md). Status: milestone M0 (foundation) — storage,
encryption, and a minimal CLI. The TUI and quick-add syntax arrive in M1.

## Usage

```sh
ttask init                 # plain database
ttask init --encrypt       # password-encrypted database (prompts twice)

ttask add Pay rent
ttask ls                   # open tasks; --all includes done
ttask done 1 2
ttask --json ls            # machine-readable output
```

The database lives in the platform data directory (e.g.
`%APPDATA%\ttask\data\ttask.db` on Windows); override it with `--db <path>`
or `TTASK_DB`. For an encrypted database the password comes from
`--password-file <path>`, then `TTASK_PASSWORD` (prints a warning), then an
interactive prompt. A forgotten password cannot be recovered.

Exit codes: 0 ok, 1 usage or other error, 2 not found, 3 wrong or missing
password, 4 database locked, corrupt, or from a newer ttask.

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

## Layout

| Crate | Responsibility |
| --- | --- |
| `crates/ttask-core` | Domain types, `Clock`; no I/O |
| `crates/ttask-store` | DB opener (plain/encrypted detection, keying), migrations, repository |
| `crates/ttask-app` | Services used by the CLI and, later, the TUI |
| `crates/ttask` | The `ttask` binary (clap CLI) |
