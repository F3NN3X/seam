//! Opt-in debug log (`--log` or `SEAM_LOG=1`), appended to %APPDATA%\Seam\seam.log.
//!
//! Lines go to a writer thread, so file I/O never runs inside the mouse hook.
//! With logging off, `info!` is one atomic load: no formatting, no allocation.
//! Only the tray calls `init`, so the panel's calls into the lib write nothing.

use crate::config::Config;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

enum Msg {
    Line(String),
    Flush(Sender<()>),
}

static TX: OnceLock<Sender<Msg>> = OnceLock::new();

fn requested<A: AsRef<OsStr>>(mut args: impl Iterator<Item = A>, env: Option<&OsStr>) -> bool {
    env.is_some_and(|v| v == "1") || args.any(|a| a.as_ref() == "--log")
}

/// Start the writer thread if logging was asked for. Stays off, silently, if
/// the file can't be opened.
pub fn init() {
    if !requested(std::env::args_os(), std::env::var_os("SEAM_LOG").as_deref()) {
        return;
    }
    let path = Config::path().with_file_name("seam.log");
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else { return };
    let (tx, rx) = channel();
    let writer = std::thread::Builder::new().name("seam-log".into()).spawn(move || {
        for msg in rx {
            match msg {
                Msg::Line(s) => {
                    let _ = file.write_all(s.as_bytes());
                }
                Msg::Flush(done) => {
                    let _ = done.send(());
                }
            }
        }
    });
    if writer.is_ok() {
        let _ = TX.set(tx);
    }
}

#[inline]
pub fn enabled() -> bool {
    TX.get().is_some()
}

pub fn write(args: fmt::Arguments) {
    let Some(tx) = TX.get() else { return };
    let _ = tx.send(Msg::Line(format!("{} {args}\n", stamp())));
}

#[cfg(windows)]
fn stamp() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(windows))]
fn stamp() -> String {
    String::new()
}

/// Wait for queued lines to reach the file. The writer dies with the process.
pub fn flush() {
    let Some(tx) = TX.get() else { return };
    let (done, wait) = channel();
    if tx.send(Msg::Flush(done)).is_ok() {
        let _ = wait.recv();
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::log::enabled() {
            $crate::log::write(format_args!($($arg)*));
        }
    };
}
pub use crate::log_info as info;

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(args: &[&str], env: Option<&str>) -> bool {
        requested(args.iter(), env.map(OsStr::new))
    }

    #[test]
    fn off_unless_asked() {
        assert!(!asked(&["seam.exe"], None));
        assert!(!asked(&["seam.exe", "--line-up"], Some("0")));
    }

    #[test]
    fn flag_or_env_turns_it_on() {
        assert!(asked(&["seam.exe", "--log"], None));
        assert!(asked(&["seam.exe"], Some("1")));
    }
}
