//! Code shared by the tray app and the settings panel.

pub mod config;
pub mod geom;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod ease;
#[cfg(windows)]
pub mod tray;
