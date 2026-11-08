# Changelog

All notable changes to Seam. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). A release takes the section headed with its version as its notes, so move **Unreleased** under a new `## [X.Y.Z] - YYYY-MM-DD` heading before tagging.

## [Unreleased]

### Added

- An installer for the GitHub version. It installs for you only, with no admin prompt, and adds Seam to the Start menu and to Installed apps.
- **Tell me when a pinned window closes**, on the Zones page. A click on the notification starts the app again and pins its new window. Off until you turn it on.
- **Offer to restore my windows after I sign in**, on General. Seam remembers which apps were where and which were pinned, and after sign-in a click puts them back. Off until you turn it on.

### Fixed

- Opening a search result scrolls its page to the setting it found, so rows near the bottom, such as Pin border colour, are in view.
- If an updated Seam doesn't start, the updater puts the old version back and starts it.

### Security

- The updater only downloads from Seam's own GitHub releases.

## [0.4.0] - 2026-11-08

### Added

- Workspaces: capture which apps sit in which zone, then switch back with a shortcut, the tray menu or the API. Missing apps start, and other windows can be left, minimized or closed.
- Line Up also offers a border between screens that sit side by side on the desk but don't touch in Windows, such as a tall screen beside two stacked ones.

### Changed

- Releases are signed, so the built-in updater installs them. Versions 0.2.0 and 0.3.0 can't check the signature, so download this version by hand once.

### Security

- Seam's apps check that the program behind Seam's local connection runs as you, and the updater only installs into its own folder. This matters on a PC shared with other Windows accounts.

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
