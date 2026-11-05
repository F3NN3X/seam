//! Code shared by the tray app and the settings panel.

pub mod api;
pub mod config;
pub mod desk;
pub mod geom;
pub mod hotkey;
pub mod layout;
pub mod log;
pub mod state;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod ease;
#[cfg(windows)]
pub mod monitors;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod winver;
