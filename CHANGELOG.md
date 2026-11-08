# Changelog

All notable changes to Seam. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). A release takes the section headed with its version as its notes, so move **Unreleased** under a new `## [X.Y.Z] - YYYY-MM-DD` heading before tagging.

## [Unreleased]

## [0.5.0] - 2026-11-08

### Added

- Windows by direction on the desk: Shift+Win+Alt+arrows moves the focused window to the next zone or screen that way, and Ctrl+Shift+Alt+arrows focuses the nearest window that way. Swap trades places with it, and has no key until you set one.
- **Pin window on top**, and **Keep window out of workspace switches** for a window a switch should leave alone. **Release pinned windows**, in the tray menu, takes every pin back.
- A focus border in your Windows accent colour, off until you turn it on, and an amber border on pinned windows. Both colours can be changed on the Zones page: a swatch, None, or your own from a colour picker.
- Seam leaves placing windows to komorebi, GlazeWM or LeopardWM when one is running, or to any other window manager when you say so on the Zones page.
- A Stream Deck plugin with keys for workspaces, Line Up, the remap and zones, moving, focusing and pinning windows, and the current desk.
- Each workspace on the Workspaces page shows its apps as a row of icons under the name, up to eight and then "+n", instead of a window count.
- An installer for the GitHub version. It installs for you only, with no admin prompt, and adds Seam to the Start menu and to Installed apps.
- **Tell me when a pinned window closes**, on the Zones page. A click on the notification starts the app again and pins its new window. Off until you turn it on.
- **Offer to restore my windows after I sign in**, on General. Seam remembers which apps were where and which were pinned, and after sign-in a click puts them back. Off until you turn it on.
- **Who reopens the apps**, under it: Windows' Restart apps, with Seam only putting the windows back, or Seam, which turns Restart apps off meanwhile and gives it back afterwards.
- ARM64 builds: an ARM64 installer and zip next to the x64 ones. The updater keeps each install on its own build.

### Fixed

- Opening a search result scrolls its page to the setting it found, so rows near the bottom, such as Pin border colour, are in view.
- If an updated Seam doesn't start, the updater puts the old version back and starts it.

### Security

- The updater only downloads from Seam's own GitHub releases.
- The local API closes a connection that sends nothing for 30 s, unless it listens for events, so idle clients can't use up its connections.

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
