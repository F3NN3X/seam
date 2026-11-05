//! Code shared by the tray app and the settings panel.

pub mod config;
pub mod desk;
pub mod geom;
pub mod layout;
pub mod log;
pub mod status;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod ease;
#[cfg(windows)]
pub mod monitors;
#[cfg(windows)]
pub mod tray;
