//! Code shared by the tray app and the settings panel.

pub mod api;
pub mod beside;
pub mod classify;
pub mod config;
pub mod desk;
pub mod direction;
pub mod geom;
pub mod groups;
pub mod hotkey;
pub mod layout;
pub mod library;
pub mod log;
pub mod palette;
pub mod state;
pub mod updates;
pub mod workspace;
pub mod zones;

#[cfg(windows)]
pub mod appicon;
#[cfg(windows)]
pub mod apps;
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
pub mod restart_apps;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod winutil;
#[cfg(windows)]
pub mod winver;
