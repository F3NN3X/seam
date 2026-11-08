//! Code shared by the tray app and the settings panel.

pub mod api;
pub mod config;
pub mod desk;
pub mod direction;
pub mod geom;
pub mod hotkey;
pub mod layout;
pub mod log;
pub mod palette;
pub mod state;
pub mod updates;
pub mod workspace;
pub mod zones;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod ease;
#[cfg(windows)]
pub mod install;
#[cfg(windows)]
pub mod monitors;
#[cfg(windows)]
pub mod package;
#[cfg(windows)]
pub mod panelwin;
#[cfg(windows)]
pub mod process;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod winutil;
#[cfg(windows)]
pub mod winver;
