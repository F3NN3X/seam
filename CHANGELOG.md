# Changelog

All notable changes to Seam. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). A release takes the section headed with its version as its notes, so move **Unreleased** under a new `## [X.Y.Z] - YYYY-MM-DD` heading before tagging.

## [Unreleased]

### Fixed

- **Undo of Tile or a layout pick maximizes again** the windows that were maximized before. Undo of a split does too.
- **"When I sign in" with Do nothing on every desk** no longer starts the sign-in timer. Changing a desk's choice later in that start starts it.
- **A pin the window manager refuses** is no longer tried again on every pass over the open windows. A window gets one more try when it is shown again, then I leave it alone, with one line in the log.
- **Alt+drag at quit** keeps its hook until you let go of the button, for at most 2 seconds, so the release doesn't reach the app.
- **Stream Deck plugin 0.8.2:** the Apply layout key asks Seam for a screen's layouts once, not on every redraw.
- **The log says when Ease cursor movement is missing** from the registry on a Windows that has the setting, and that Seam reads it as on.
- A window in a snap-beside group now leaves it as soon as I drag it away, not a moment after. Closing or minimizing a window in a group is now noticed however many windows the groups hold, where only the first 128 were.
- **Keys during a drag** no longer slip through to the app when a drag starts right after the last one, and a key held past the drop no longer repeats into it. Esc during a divider handle's drag no longer reaches the focused app as well.
- **Alt as the show key, or the Alt+drag key,** now lets Alt+Tab through to Windows, and the wheel and 1, 2, 3, 0 steer while it is held. With Win as the Alt+drag key, Win with the wheel, Tab and 1, 2, 3, 0 steers during that drag instead of reaching Windows.
- Snipping Tool's capture overlay, the lock screen and the Xbox Game Bar no longer make Seam step aside for a moment. A Direct3D game on another screen no longer makes it step aside for a maximized app with an auto-hidden taskbar.
- **A split or pull-out made while Seam was busy** is saved after all, with its Undo, instead of being dropped and leaving the saved layout old.
- **Windows Seam only knows by their zone mark** (after a restart) are renumbered with the others on a split or pull-out, and Undo gives their numbers back.
- **A zone swap** leaves a window alone that I moved by hand before its turn came, and says so in the log.
- **A workspace row no longer keeps a window that isn't its own** when Windows reuses a window handle for another program's window.
- **Alt+drag** forgets an app's verdict the moment the app exits, so a new program that gets its process id isn't judged by it.

### Changed

- With the debug log on, each step aside and return says what decided it: the window's class and exe, its rect against its screen and Windows' full-screen state. The zone swap's look at the open windows now starts when a drag does, so it is ready when zones show.

## [0.7.0] - 2026-11-13

### Added

- **Pull a window out of its zone**: drag a snapped window off its zone and hold Ctrl as you let go where no zone shows. If it was alone in its zone, the layout loses that zone, the neighbouring zone takes the space, the windows there move to fill it, and the layout shows as **Changed** with a notice and Undo for 8 seconds. Without Ctrl the gap stays, as before. With **Move and resize from anywhere in a window** on and Ctrl+Alt as its key, Ctrl is held for the whole drag, so it does nothing there.
- **Move and resize from anywhere in a window** on Zones, off by default: hold Alt and drag anywhere in a window to move it, or right-drag to resize it from the nearest edge or corner, as WinScreen and AltSnap do. Win or Ctrl+Alt can be the key instead, for apps that use Alt+click. Holding the show key snaps it into zones, and snapped neighbours follow a resize. `windows.alt_drag` (`on`, `key`) in `config.json`.
- **Snap beside a window** on Zones, on by default: while zones show during a drag, point at the edge of another window and the drop puts the two side by side, half each, or 1/3 and 2/3 with the wheel or 1, 2 and 3. On a screen with zones it splits the zone that window holds, saves the layout as Custom and offers Undo for 8 seconds. On a screen with no zones the two become a group until Seam restarts: they resize together from the edge between them, the gap has a handle, a third window dropped beside one joins, and a window dragged away, minimized, maximized or closed leaves. The middle of a held zone still swaps. `zones.snap_beside` in `config.json`.
- **Fit group to screen** on Shortcuts (no key until you set one) and `window.fit` in the API: stretches the focused window's group over its screen, keeping shares and order, or gives a window in no group the whole work area, with Undo for 8 seconds.
- **When I sign in** on General replaces the restore switch with one choice for each desk: **Do nothing** (the default), **Offer my last session** (as before) or **Open a workspace**. With **Ask first** on, the default, Seam shows "Open <workspace>? Click to open" and switches on the click. With it off, it switches about 10 seconds after the taskbar is up. It only acts when Seam starts at sign-in, waits up to 3 minutes while a full-screen app is in front, and a pause by hand cancels it. It acts on the desk that is current when it fires. A deleted workspace, or one that isn't for the desk, opens nothing and the log says so. `sign_in` (`mode`, `workspace`, `ask`) on a desk through `config.patch`; `restore_session` on on becomes **Offer my last session** on every desk the first time Seam starts, and a desk seen later starts at **Do nothing**.
- **Stream Deck plugin 0.8.1** carries the Apply layout and Pause Seam keys and Open alongside. Each plugin change now raises its version, since Stream Deck only replaces an installed plugin with a newer one.
- **Keep on top** for an app rule, on the app's row under **...** on **Where apps open**, and **Keep out of workspace switches**: its windows are pinned the way a pin by hand is, with the pin border, **Release pinned windows**, the close notice and the API, when they open or when you make the rule while the app is open. Choose **Doesn't move** as its place to only pin an app. **Release pinned windows** frees them until the window is opened again, and the log says how many were pinned by rules. `pin` (`top`, `sticky` or `both`) on an entry of `app_rules`.
- Stream Deck: an **Apply layout** key (pick a screen and a layout; it lights while the screen has it), a **Pause Seam** key (lit while paused, "Full screen" while Seam waits for a full-screen app) and an **Open alongside** checkbox on the workspace key. `zones.apply` and `zones.layouts` in the API, and `layout` on each screen in the state.
- **Step aside for games and full-screen apps** on General, on by default: while a game, a full-screen video or a slide show is in front, Seam unregisters its shortcuts so the app gets those keys, shows no zones and hides the divider handles, and turns Spotlight and the focus border off. Everything comes back when you leave it. Seam finds it from Windows' own full-screen state (`SHQueryUserNotificationState`) and from a window that covers a whole screen, on each focus change, with no polling. `step_aside_fullscreen` in `config.json`.
- **Pause Seam**: a tray item (**Pause Seam** / **Resume Seam**), a shortcut (`hotkeys.pause_seam`, no key until you set one) and `seam.pause` in the API. It does all of the above, and also turns the pointer remap and edge resistance off (without changing your setting), stops app rules and auto-apply placing windows, and waits to check workspace drift. A pause isn't saved, so a restart resumes. `paused` and `stepped_aside` in the state, a grey tray icon and General say which.
- Dragging a window with zones showing previews the drop: the lit zone is where it lands, and grey outlines show where windows it swaps with go. While zones show, the **mouse wheel** steps through the whole zone, 1/2, 1/3, 2/3, the zone with its neighbour and the whole screen, with the share on the side nearest the pointer, and **right-click** drops it without a zone. The keys do the same: **Tab** steps through the zone, the zone with its neighbour and the whole screen (**Shift+Tab** back unless Shift shows zones), **1**/**2**/**3**/**0** give the window a half, a third, two thirds or all of it, the **arrows** pick the side, and **Esc** drops it without a zone. A part splits the zone, saves the layout as Custom and offers Undo for 8 seconds. Every other key and click reaches your apps. **Pause to snap** (`zones.pause_to_snap`) shows zones when you hold still, and **Show keys while dragging** (`zones.drag_hints`) lists the keys on the screen; both off by default.
- **Update from screen removes apps that aren't open** on Workspaces, off by default: with it on, Update from screen removes the rows of apps with no window open, instead of keeping them, and a notification offers Undo for 8 seconds, which puts them back where they were. Quick save, **Where it is now** and Capture never remove a row. `workspaces_update_removes_closed` in `config.json`; `workspace.capture` with `into` follows it.
- **Swap with the window already in a zone** on Zones, on by default: dropping a window on a zone another snapped window holds makes them trade places, instead of one lying on the other. If the dropped window was loose, the other takes its old spot and size, and a drop across several held zones sends those windows to its old zones in the same order. Off keeps the overlap. `zones.swap_on_drop` in `config.json`.
- Dragging the edge of a window in a zone moves the windows in the zones on the other side with it, live. No window goes below the smallest size its app allows: the next divider moves instead. When you let go, the window fits its zone, the layout is saved as Custom and every window stays snapped. Esc puts everything back. **Resize neighbours with a window** on Zones, on by default, and `zones.resize_neighbours` in `config.json`.
- Handles in the gap between snapped windows. Drag one to move that divider, with the windows on both sides following live, and the layout is saved as Custom. Right-click one to give a window 1/2, 1/3, 2/3, 1/4 or 3/4 of it and its neighbour, or to take a window out of its zones. **Handles between snapped windows** on Zones, on by default, and `zones.divider_handles` in `config.json`. **Size menu** on Shortcuts (no key until you set one) opens the same choices for the focused window, for a gap too narrow for handles. `window.share` and `window.ungroup` in the API.
- **Tile this screen's windows**: puts the loose windows on the screen under the pointer into its zones. A shortcut (no key until you set one), **Tile windows** in the tray menu (a submenu of screens when you have several), `screen.tile` in the API, and a **Tile screen** Stream Deck action. A screen with no layout is given one for the number of windows, and a notification offers Undo for 8 seconds.
- **Main + stack** and **Stack + main** layouts on the Zones page: a main zone of two thirds beside two stacked zones. An older Seam reads a screen with one of them as having no zones.
- **Where apps open**, a card on the Zones page: add an app, pick its screen and zone (or the full screen), and its windows go there once whenever they open, workspace or not. A workspace in use that has a row for the app wins. Dialogs, apps that run as administrator, apps under **Left alone** and windows that open during a switch are never moved, and a rule whose screen isn't connected does nothing. `app_rules` in the config holds them.
- **Grow window into its neighbour**, **Shrink window** and **Give window two thirds of its group** on Shortcuts, with no key until you set one. They move the dividers beside a window in a zone, and the windows in the other zones follow. No window goes below the smallest size its app allows: the divider beyond it moves instead. A second press of two thirds puts the zones back. The changed layout is saved as Custom. `window.resize` and `window.priority` in the API.
- **Switch back to the previous workspace**: a shortcut (no key until you set one), the first item of the tray's Workspaces menu, `workspace.switch_back` in the API, and a Stream Deck action. `previous_workspace` on a desk in `config.json`.
- **Open alongside**, on a workspace's **...** menu, switches once and leaves the other windows alone. `workspace.switch` takes `alongside`.

- **Focus on this window** minimizes the other windows on its screen, and pressing it again brings them back. A shortcut (no key until you set one), `window.focus_only` in the API (`scope` `workspace` minimizes everything outside the workspace in use), and a Stream Deck action.
- **Spotlight** dims everything but the window you're using. A shortcut (no key until you set one), `window.spotlight` in the API, `spotlight` in the state, and a Stream Deck key that shows whether it's on.
- A library of custom layouts on the Zones page, shared by every desk and screen: each has a tile after the templates, and a **...** menu with Edit, Rename, Duplicate and Delete. Editing one changes every screen that uses it, and deleting one leaves those screens with their zones. **New layout** starts one. `custom_layouts` in `config.json`, and `custom` on a screen's layout.
- **Quick layout**: a small panel on the screen under the pointer for picking its zone layout from the keyboard. Numbered thumbnails of the built-in layouts, the custom layouts made for that screen and None; 1 to 9 or the arrows and Enter pick, Ctrl+Z undoes the last pick, Esc or a click outside closes it. A shortcut (no key until you set one), `zones.quick` in the API, and a **Quick layout** Stream Deck action.
- **Save from windows** on the Zones page makes a library layout from where the windows on a screen are now, and snaps them into it. `zones.from_windows` in the API.
- Picking a layout on the Zones page puts the windows on that screen into the new zones, one window to a zone, snapped windows first. When windows moved, a notification offers Undo for 8 seconds.
- **Edge resistance** on Displays, off by default: the pointer waits at the edge to another screen until you push that many pixels past it, so you can grab a window's edge or corner at a screen border. It gives way while a mouse button is down, so window drags aren't slowed. `pointer.edge_resistance_px` in `config.json`, 0 to 100.
- Workspaces set each screen's zone layout. **Zones** in the workspace editor has a row per screen: **Keep as is**, **No zones**, a built-in layout or a library layout, with a thumbnail. A switch, Snap back and Open alongside set the layouts before placing the apps, and a last session restore doesn't. Capture, **Update from screen** and quick save record every screen's layout, and editing a library layout updates the workspaces that use it. A screen's layout that isn't the workspace's shows as moved, and **Put windows back** fixes it. Import maps the layouts to your screens and drops their links to the other machine's library. `zones` on a workspace in `config.json` and in `workspaces.list`.
- **Remember folders and pages** on Workspaces, off by default: a capture, and **Add an app** from the windows open now, also save the folder each Explorer window shows and the page each browser shows, and a switch takes the window the row was saved from or started, and once that's closed starts the app there in a new window. Other Explorer and browser windows are left alone, and an open window is never sent anywhere. Only folders on this PC's own drives and http or https pages are kept, and a page loses its whole query when any parameter's name contains a word like token, key, auth, code, sess or state. Private windows are skipped as best Seam can tell from the title. Workspace and settings Export leave them out, and both kinds of import drop them. **Forget saved folders and pages** removes them while the setting is off. `workspaces_remember_places` in `config.json`, and `place` on a workspace row.
- **Glued edge push** on Displays sets how far to push past a glued edge before the pointer crosses, 60 px by default and 0 to cross at once. `pointer.glue_push_px` in `config.json`, 0 to 200.

### Changed

- The Zones page shows the preview across the page, the built-in layouts in one row of smaller tiles under it, then **Your layouts**. A layout you make is for the screen you made it on, and **Used on...** in its **...** menu ties it to other screens or any screen. **Show all screens** lists every layout. `screens` on a `custom_layouts` entry, and one without it is for any screen.
- The **Custom** tile on the Zones page is gone. Clicking it only opened the editor for the screen's layout. **Edit layout** on a template now saves the result as a new library layout instead of changing the screen alone, and a screen whose layout is none of the tiles shows **Changed**, with **Save as layout**. Screens with a Custom layout from before show as Changed, and nothing is added to the library until you save one.

- **Never manage these apps** moved from General to the Zones page, where it is the **Left alone** list under **Where apps open**. It is still `zones.exclude`.
- When a window won't move because its app runs as administrator, the balloon names the app and tells it once per app instead of once per run. Clicking it opens Help, which has a new topic, "Apps running as administrator".

## [0.6.5] - 2026-11-09

### Added

- **Glue** in Line Up: screens that touch on your desk but have a gap between them in Windows can be glued with G, and the pointer crosses the gap where they physically meet. Line Up offers each such pair after the touching borders. A pair found from the screens' sizes starts glued, and a pair found only from the pixel layout starts off. Align arrangement keeps glued screens apart. Displays marks a glued border "Glued to the right edge of 1". `glued` on a border in `config.json` and in the state's `borders`.
- A glued edge holds the pointer until you push about 60 px past it, so an auto-hide taskbar on that edge still shows. Past the end of a glue, the edge is a dead edge that Seam holds.
- **Not on my desk**: press N in Line Up, or use the border's **...** menu on Displays (**On my desk** undoes it), for screens that touch in Windows but not on your desk. The pointer stops at that edge both ways, or crosses to a glued screen past it. Displays shows such a border as "Not on your desk". `not_on_desk` on a border in `config.json` and in the state's `borders`.
- Line Up names both screens with their model in each step, such as "Line up screen 3 (HDMI) with screen 4 (U3477)", and says which way to move each line, such as "straight below" or "level with".
- Line Up draws a band of faint diagonal stripes along the shared edge of the two screens, 10 mm apart on the desk, so you can see at a glance whether the edge runs straight across.
- **Type measurements in Line Up** on Displays, off by default. With it on, M in Line Up takes the offset between two screens as a number in cm and moves the lines to match. `lineup_measure` in `config.json`.
- **Two views of the layout** on Displays: **Seam** (your desk, with glued borders dashed and borders that aren't on your desk dotted) and **Windows** (the arrangement as Windows has it, in pixels, with "18 px gap" labels). The choice is remembered while the Seam window is open. `rect` on each of the state's `screens`.
- **Show on screens** on Displays draws the chosen view on every real screen for a few seconds, with each screen's number and model and a band along each edge that has a neighbour. Esc, a click or 8 seconds closes it. `layout.show` in the API, for the Seam window only.
- **Saved desks** and the border rows on Displays show when a desk was lined up ("Lined up today", "yesterday", "3 days ago", then the date), and dates are in your local time. A desk lined up by an earlier version shows plain "Lined up" until you line it up again. `lined_up_at` in `config.json`.
- A **Help** page in the Seam window, with a topic for each part of Seam and a button to its page. Search finds the topics by their text.
- Seam can turn on Windows' **Ease cursor movement between displays** for you: the Seam window asks once while it's off, and the Displays alert has **Turn it on**. Seam never turns it off. `cursor_easing.set` in the API, and `windows_cursor_easing` in the state.
- **Set as default** on a workspace's "..." menu: when no workspace is remembered, it's the one in use when Seam starts. `workspace.set_default` in the API, and `default` in `workspaces.list`.
- A screen reader now reads the Seam window's sidebar rows, switches, choices, pause button and "..." buttons by name, such as "More for Desk" and "Remap pointer at borders, switch, on".
- The debug log's once-a-minute hook line has histograms of Seam's own time and of other apps' hooks, and counts of events over 1 ms, to find where hook spikes come from.

### Changed

- The Seam window opens centred on the screen the mouse pointer is on, instead of always the primary screen.
- A refused arrangement names the screens, such as "Moving screen 3 would overlap screen 1", instead of a general reason.
- The sidebar's App group (Shortcuts, General, Help and About) sits at the bottom, above the version. About's Help section is now called Support.
- The settings window opens a little wider, and the workspace list is narrower, so the workspace editor's columns show in full. Workspace cards show up to six app icons before "+n".
- The workspace in use shows a green dot and "Active" in the Workspaces list, instead of "in use", and "Active, moved" once one of its windows moved.
- After a restart or a desk change, the workspace last in use on that desk shows as active again, else the default, else the only one whose open apps are all in place. Nothing moves.

### Fixed

- **Turn it on** for Ease cursor movement now checks that Windows kept the setting, and says so if it didn't.
- Seam is listed in Installed apps as "Seam", without a version in its name that went stale after an in-app update.
- A debug build of the tray no longer aborts on a window no workspace placed.
- Running `seam-settings.exe --page` or `panel.open` with a page while the Seam window is already open now switches that window to the page. Before, it only came to the front.
- Apps that close to the tray, such as Stream Deck, Spotify, Discord and Steam, show "Running in background" in the workspace editor instead of "Not running", and don't count as moved.
- A workspace card's name gets the full width, with its marks on a line under it, instead of a few letters.
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
