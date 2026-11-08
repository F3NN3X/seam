# Changelog

All notable changes to Seam. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). A release takes the section headed with its version as its notes, so move **Unreleased** under a new `## [X.Y.Z] - YYYY-MM-DD` heading before tagging.

## [Unreleased]

### Added

- **Set as default** on a workspace's "..." menu: when no workspace is remembered, it's the one in use when Seam starts. `workspace.set_default` in the API, and `default` in `workspaces.list`.
- A **Help** page in the Seam window, with a topic for each part of Seam and a button to its page. Search finds the topics by their text.
- Seam can turn on Windows' **Ease cursor movement between displays** for you: the Seam window asks once while it's off, and the Displays alert has **Turn it on**. Seam never turns it off. `cursor_easing.set` in the API, and `windows_cursor_easing` in the state.

### Changed

- The sidebar's App group (Shortcuts, General, Help and About) sits at the bottom, above the version. About's Help section is now called Support.
- The Seam window opens centred on the screen the mouse pointer is on, instead of always the primary screen.
- The settings window opens a little wider, and the workspace list is narrower, so the workspace editor's columns show in full. Workspace cards show up to six app icons before "+n".
- The workspace in use shows a green dot and "Active" in the Workspaces list, instead of "in use", and "Active, moved" once one of its windows moved.
- After a restart or a desk change, the workspace last in use on that desk shows as active again, else the default, else the only one whose open apps are all in place. Nothing moves.

### Fixed

- Apps that close to the tray, such as Stream Deck, Spotify, Discord and Steam, show "Running in background" in the workspace editor instead of "Not running", and don't count as moved.
- The "..." on each app in the workspace editor no longer gets cut off: the name and Goes to columns shrink and end in "..." instead, down to the panel's smallest size.

- A workspace counts as moved as soon as you snap one of its windows with Win+arrow or use the maximize button, not only after a drag.
- Reordering or removing apps in a workspace no longer mixes up which open window belongs to which app, so Snap back and opening apps put them in the right place.
- An app you open while a switch is still starting the others now goes to its place once the switch ends, with **Put apps in place when they open** on.
- After a quick save, **Update from screen** or **Where it is now**, moving a saved window counts as moved at once, and reopening a saved app puts it back, without switching first.
- A workspace app icon that couldn't load is tried again when you come back to the Workspaces page a minute later.

## [0.6.0] - 2026-11-08

### Added

- The Stream Deck plugin is a download on each release, next to the setups and zips.

- Workspaces remember whether each window was minimized, maximized or full screen, which window was in front of which, and which had the focus, and a switch brings all of that back. A minimized window stays minimized and restores into its zone later.
- Edit a workspace on the Workspaces page: where each app goes (including where it is now), how it shows, the start order by drag or Alt+Up and Alt+Down, and removing an app. Each change saves at once, with Undo.
- **Add an app** to a workspace from the windows open now or from the Start menu's apps, Store apps included.
- Launch details per app: a delay of up to 30 s, and waiting for an app's window before the next ones start.
- **Never include this app** on a workspace's app, and the **Never manage these apps** list on General to take it back.
- **Pinned windows stay when I switch workspaces**, on the Zones page: a switch leaves a window pinned on top where it is. Off until you turn it on.
- The workspace editor shows how each app is doing now: in place, moved, minimized, not running, or can't be moved because it runs as administrator. **Put windows back** shows when something moved.
- **Snap back to this workspace** puts moved and minimized windows back: a shortcut (no key until you set one), `workspace.snap_back` in the API, and a Stream Deck action. A workspace key on the Stream Deck shows an amber dot while its workspace has drifted.
- **Put apps in place when they open**, on the Workspaces page: an app of the workspace in use that you open by hand goes to its place once. Off until you turn it on.
- **When a window moves to another screen**, on the Zones page: a window moved by key or by another app, or put back by a workspace, onto another screen can keep its pixel size or its real size instead of Windows' scaling, which stays the default. **Also when I drag a window to another screen** does the same after a mouse drag, and is off until you turn it on.
- **Save windows to the active workspace** saves the windows into the workspace in use, with Undo by clicking the notification for 8 s, or into a new "Workspace N" when none is in use: a shortcut (no key until you set one), `workspace.quick_save` in the API, and a **Save windows** Stream Deck action.

### Changed

- Capturing a workspace only saves app windows with a taskbar button. Overlays (Discord, GPU, Xbox Game Bar), helper and input windows are left out, and so are the apps the zones leave alone.
- A switch that has nothing to move now changes nothing and takes a few milliseconds, and one that does only pauses for windows going to a screen with another scale.
- A switch places each app it starts as soon as its window shows, instead of waiting for the slowest one, and apps that are already running go first.
- **Update from screen** keeps each app's launch details and place in the order, and no longer drops apps that aren't running.

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
