//! Seam — line up your screens on Windows 11 so the pointer crosses where they
//! physically meet. A Windows take on https://screenshim.app/ plus a cursor
//! remap, since Windows lays monitors out in raw pixels.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod geom;
mod layout;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod arrange;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod log;
#[cfg(windows)]
mod monitors;
#[cfg(windows)]
mod overlay;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Seam is a Windows app. Build it with `cargo build --release` on Windows, or ./build-cross.sh.");
}
