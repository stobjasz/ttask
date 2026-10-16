//! Handing a link to the OS: the default browser or mail client.

use std::io;
use std::process::{Command, Stdio};

use ttask_core::links::is_openable;

/// Opens `link` with the platform opener without waiting for it. Anything
/// [`is_openable`] rejects (other schemes, whitespace) is refused.
pub fn open_link(link: &str) -> io::Result<()> {
    if !is_openable(link) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to open `{link}`: only http, https and mailto links open"),
        ));
    }
    let mut child = opener(link)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap it so a long TUI session does not collect zombies.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn opener(link: &str) -> Command {
    let mut cmd = if cfg!(windows) {
        // ShellExecute via url.dll rather than `cmd /c start`, so `&` and `^`
        // in the link are not shell syntax.
        let mut cmd = Command::new("rundll32.exe");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(link);
    cmd
}
