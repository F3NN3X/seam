//! Seam — line up your screens on Windows 11 so the pointer crosses where they
//! physically meet. A Windows take on https://screenshim.app/ plus a cursor
//! remap, since Windows lays monitors out in raw pixels.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
use seam::{config, desk, ease, geom, hotkey, layout, log, monitors, tray};

#[cfg(windows)]
mod altdrag;
#[cfg(windows)]
mod api_server;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod arrange;
#[cfg(windows)]
mod dividers;
#[cfg(windows)]
mod dragkeys;
#[cfg(windows)]
mod focus_only;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod identify;
#[cfg(windows)]
mod live;
#[cfg(windows)]
mod minsize;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod places;
#[cfg(windows)]
mod quicklayout;
#[cfg(windows)]
mod showlayout;
mod snap;
#[cfg(windows)]
mod spotlight;
#[cfg(windows)]
mod standby;
#[cfg(windows)]
mod switcher;
#[cfg(windows)]
mod wincontrol;
#[cfg(windows)]
mod wm;
#[cfg(windows)]
mod zone_paint;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Seam is a Windows app. Build it with `cargo build --release` on Windows, or ./build-cross.sh.");
}
