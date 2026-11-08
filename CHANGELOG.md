# Changelog

All notable changes to Seam. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). A release takes the section headed with its version as its notes, so move **Unreleased** under a new `## [X.Y.Z] - YYYY-MM-DD` heading before tagging.

## [Unreleased]

### Added

- Workspaces: capture which apps sit in which zone, then switch back with a shortcut, the tray menu or the API. Missing apps start, and other windows can be left, minimized or closed.
- Line Up also offers a border between screens that sit side by side on the desk but don't touch in Windows, such as a tall screen beside two stacked ones.

### Changed

- Releases are signed, so the built-in updater installs them. Versions 0.2.0 and 0.3.0 can't check the signature, so download this version by hand once.

## [0.3.0] - 2026-11-05

### Added

- Zones: hold Shift while you drag a window to drop it into a zone. Hold Ctrl as well to span several.
- Layouts per screen: Columns, Rows, Grid, Priority, or your own.
- A zone editor that works on the real screen at full size, with Undo, or as a live view of the preview.
- Shortcuts and API methods to move a window to the next or previous zone.
- Experimental: keep windows in their zones after a screen or resolution change. Off by default.

This build is unsigned, so Windows warns about an unknown publisher the first time. The built-in updater stays idle until releases are signed, so to update, download the new zip and replace both exes.

## [0.2.0] - 2026-11-05

### Added

- A new Seam window: a physical map of your screens, borders, saved desks, shortcuts, search, and light and dark themes.
- Saved desks: each set of screens keeps its own borders, and your v0.1 settings carry over.
- Global shortcuts for Line Up and pausing the remap.
- A local API, so tools on this PC can read status and control Seam.
- A built-in updater, which stays idle until releases are signed.

This build is unsigned, so Windows warns about an unknown publisher the first time. To update from this version, download the new zip and replace both exes.
