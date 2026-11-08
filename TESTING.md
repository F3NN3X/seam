# Manual test checklist

Run these on a release build, with logging on unless a step says otherwise:

```
target\release\seam.exe --log
```

Quit any running Seam first. The log is `%APPDATA%\Seam\seam.log`. Back up `%APPDATA%\Seam\config.json` before you start.

## Test desk

| Line Up number | Log index | Screen | Resolution | Scaling | Position |
|---|---|---|---|---|---|
| 1 | 0 | PG32UCDM | 3840×2160 | 125% | Primary, centre |
| 2 | 1 | LG HDR QHD | 2560×1440 | 100% | Above screen 1 |
| 3 | 2 | HDMI panel | 1920×550 | 100% | Below screen 4 |
| 4 | 3 | U3477 | 1440×3440 | 100% | Right of screen 1 |

The HDMI panel reports no DPI (`raw_dpi=0`), so **R** can only guess a scale of 1 for that border.

Line Up has three borders, in this order. You drag the lines on the second screen named:

1. Screen 2 → screen 1
2. Screen 1 → screen 4
3. Screen 4 → screen 3

On this desk, **Also align Windows arrangement** is expected to refuse. See [Align arrangement](#align-arrangement).

## Upgrading from v0.1

Start with a v0.1 `config.json` that has saved borders.

- [ ] **Migration.** Start Seam once. `config.json` now has `"version": 2`, a `desks` entry with every border from the old `layouts`, and no `layouts`. `"tray_click"` is `"line_up"`.
- [ ] **Backup.** `config.v1.json.bak` sits next to `config.json` and holds the original file. Restart Seam. It's unchanged.
- [ ] **Crossings.** The log shows `resolved 3 of 3 saved link(s)`, and crossings land where they did before.
- [ ] **Desk name.** The connected desk's `name` is `PG32UCDM + 3 more` and its `last_seen` is now. Change the `name` by hand and restart. Your name stays.
- [ ] **New screens.** Unplug a screen and replug it. The smaller set gets its own desk, named after its first screen, and the full desk is untouched.
- [ ] **Going back.** Quit Seam. To return to v0.1, replace `config.json` with `config.v1.json.bak`. A v0.1 build can't read the borders in a v2 file.

## Tray and shortcuts

- [ ] **Menu.** Right-click the tray icon: status line, **Open Seam…**, **Line Up Screens…**, **Remap pointer at borders**, **Release pinned windows**, **Quit Seam**.
- [ ] **Left-click, migrated.** With `"tray_click": "line_up"`, left-click opens Line Up. Press Esc.
- [ ] **Left-click, new install.** With `"tray_click": "open"`, left-click opens the Seam window.
- [ ] **Line up shortcut.** Press Ctrl+Shift+Alt+L. Line Up opens. Press Esc.
- [ ] **Pause shortcut.** Press Ctrl+Shift+Alt+P. Crossings stop remapping and the tooltip says paused. Press it again and they work.
- [ ] **Shortcut taken.** Have another app hold Ctrl+Shift+Alt+L, then start Seam. Seam runs, and `state.get` lists it in `hotkey_errors`.
- [ ] **Window shortcuts.** Shift+Win+Alt+arrows and Ctrl+Shift+Alt+arrows are registered by default, and next zone, previous zone, the swaps and the three pin keys aren't. If another app holds one, `state.get` lists it in `hotkey_errors` (for example `move_up`).
- [ ] **Ease balloon.** With Ease cursor movement off, click the balloon. The Seam window opens.

## Local API

Use a small client that opens `\\.\pipe\seam-<your username>` and writes one JSON line at a time.

- [ ] **Own account only.** Run the client as another Windows user. It's refused.
- [ ] **State.** `{"jsonrpc":"2.0","id":1,"method":"state.get"}` returns the 4 screens and 3 lined-up borders.
- [ ] **Remap.** `remap.set` with `{"on":false}`. Crossings stop remapping, and a subscribed client gets `state.changed`. Turn it back on.
- [ ] **Events.** Subscribe with `events.subscribe`, unplug the U3477. `desk.changed` arrives. Open and cancel Line Up. `lineup.finished` arrives with `saved` false.
- [ ] **Identify.** `screen.identify` with `{"number":4}`. The number shows on the U3477 for 1.5 s.
- [ ] **Off switch.** Set `api.enabled` to false in `config.json` and reload. Calls from your client get error 5.
- [ ] **Someone else's pipe.** Quit Seam. From another Windows user, or a script that runs as a different account, create `\\.\pipe\seam-<your username>` first, then start `seam-settings.exe`. Nothing connects, and the log says the pipe is served by another user's process.
- [ ] **Copied exe.** Copy `seam.exe` to another folder and connect from it. `app.quit` and `update.ready` are refused with error 4, and with `api.enabled` false the calls get error 5.
- [ ] **Capture.** A test client that isn't Seam gets error 4 from `workspace.capture`, and `workspace.switch` still works.
- [ ] **Bad input.** Send a line that isn't JSON, an unknown method, and a line over 1 MB. You get errors, or the connection closes, and Seam keeps running.
- [ ] **Slow client.** Subscribe and stop reading while a second client toggles the remap for 30 s. Events go out at most 10 times a second, so the first client is dropped after about 26 s, and crossings stay smooth.
- [ ] **Idle client.** In PowerShell run `$c = [System.IO.Pipes.NamedPipeClientStream]::new('.', "seam-$env:USERNAME", 'InOut'); $c.Connect()` and send nothing. After 30 s the log says `api client dropped: no request in 30 s`, and `$c.ReadByte()` returns -1. Connect again, send a `state.get` line every 20 s for a minute. It stays open.
- [ ] **Subscriber stays.** Connect the same way, send `{"jsonrpc":"2.0","id":1,"method":"events.subscribe","params":{"topics":["state"]}}` and a newline, then wait 2 minutes without sending anything. It's still connected, nothing is logged about it, and toggling the remap from the tray still delivers `state.changed`. The Stream Deck plugin and the panel stay connected the same way.
- [ ] **Busy.** Open the tray menu and send `remap.set`. It's answered once the menu closes.
- [ ] **Hook timing.** Note `hook stats` with a client spamming `state.get`. It matches a run without one.

## Line Up

- [ ] **Open.** Left-click the tray icon. Every screen shows its number. Border 1 of 3 is active, and the keyboard works straight away.
- [ ] **Border 1.** Drag a line on screen 1, then drag empty space to move both. Both lines follow the pointer, and the pointer crosses at the new position while you adjust.
- [ ] **Border 2.** Press Enter. The active screen moves to screen 4, and arrow keys, Shift, Ctrl and Tab all work there.
- [ ] **Border 3.** Press Enter. The active screen moves to screen 3.
- [ ] **Backspace.** Goes back one border and keeps what you set there. On border 1 it does nothing.
- [ ] **R.** Resets the current border to the guess from EDID. On border 3 this is a scale of 1.
- [ ] **Esc.** Closes every overlay without saving. `config.json` is unchanged and the previous layout is active again.
- [ ] **Enter during a drag, middle border.** Hold the mouse button mid-drag and press Enter. The drag ends, the next border opens, and the app stays up. Releasing the button later moves nothing.
- [ ] **Enter during a drag, last border.** Same, on border 3. Line Up finishes and the app stays up.
- [ ] **Finish.** Press Enter on the last border. A balloon appears, the saved layout has 3 borders, and the log shows `resolved 3 of 3 saved link(s)`.
- [ ] **Screens that only meet on the desk.** With screen 2 above 1 and screen 4 to the right of both, open Line Up again after the first run. A fourth border, screen 2 to screen 4 on the right edge of 2, follows the other three, even though 2 and 4 don't touch in Windows. Line it up and finish: the log shows `resolved 4 of 4 saved link(s)`, and Displays lists it as lined up.
- [ ] **Crossing it.** Move left off the upper part of screen 4. The pointer enters screen 2 where the lines meet, not the top-right of screen 1. Lower down it still enters screen 1. Move right off screen 2 and it enters screen 4.
- [ ] **Align arrangement.** With that border saved, Align arrangement only moves screens along borders that touch in Windows, and the 2 to 4 border stays lined up afterwards.

## Zones

Set up first: on Zones, pick **Priority** for screen 1 and **Grid** for screen 2, with **Shift** to show and **Ctrl** to span.

- [ ] **Page.** Zones is in the sidebar with no preview note. The header says "Hold Shift while you drag a window to drop it into a zone", the tabs list the 4 screens, and the preview shows each zone's number and width in px. The chosen tile has an amber ring.
- [ ] **Layouts.** Pick each of None, Columns, Rows, Grid and Priority. The preview changes, and `config.json` holds the layout under `desks[...].zones` for that screen's device path. **None** removes the entry. **Custom** is greyed out with a tooltip.
- [ ] **Shift-drag, each screen.** Drag a window with Shift held on every screen of the test desk, including the portrait U3477. The zones show with their numbers, the zone under the pointer turns amber, and letting go snaps the window with its visible edges on the zone.
- [ ] **Release Shift.** Hold Shift, move over a zone, release Shift, then let go of the window. The overlay goes away and the window stays where you dropped it.
- [ ] **Span.** Hold Shift and Ctrl together and drag across two zones. The amber box covers both, and the window fills it.
- [ ] **Across the 125% and 100% border.** Drag a window from the 125% screen to the 100% one with Shift held, and back. It lands on the zone at the right size both ways, with no jump afterwards.
- [ ] **Maximized.** Snap a maximized window. It restores, then fills the zone.
- [ ] **Gap 0 and 16.** Move the Gap slider to 0, then 16. The value under the title follows it, and the next snap leaves that much space between zones and at the screen edge, scaled on a screen with 125% scaling.
- [ ] **Restore size.** Snap a window, then drag it out by its title bar and drop it outside every zone. It gets its old size back when you let go, with the pointer at the same place along its width. Drop it on another zone and it snaps there, and dragging it out of that one later still restores the size from before the first snap. Turn **Restore size when dragged out** off and it keeps the zone's size. A window with no sizing border never changes size.
- [ ] **Chrome tab tear-off.** With zones on, drag a tab out of a Chrome window and drop it. The new window isn't snapped by accident. Drag it back onto the first window's tab strip: the tabs merge, and neither window is snapped or resized.
- [ ] **Top edge.** Drag a window to the top edge of a screen with zones on. Windows maximizes it, and Seam doesn't move it afterwards. Hold Shift over a zone and drop a window there: it snaps to the zone as normal.
- [ ] **Hidden owner.** Use an app whose main window has a hidden or zero-size owner window (many browsers and some Electron apps). Shift-drag it onto a zone. It snaps. A dialog that is open over its parent window still doesn't snap.
- [ ] **Elevated window, no overlay.** Run Task Manager as administrator and start dragging it with Shift held. No overlay appears at all, and you get the one balloon for the session. A normal window still shows zones straight afterwards.
- [ ] **Button released early.** Start a window move with Alt+Space then M and the arrow keys (no mouse button), with zones on. Nothing snaps and nothing is left on screen when you press Enter.
- [ ] **Swapped buttons.** Swap the primary mouse button in Windows settings and repeat a Shift-drag. It still snaps, and releasing the button ends the drag.
- [ ] **Fixed-size window.** Shift-drag Calculator or a similar fixed-size window onto a zone. It keeps its size and sits at the zone's top-left corner.
- [ ] **Window ignoring DPI.** Snap an old app that ignores DPI scaling between the 125% and 100% screens. It lands on the zone in one move with no extra jump.
- [ ] **Keys.** Choose Alt to show zones. The span choice that matches is greyed out with a tooltip. Choose the key span uses to show zones: span moves to another key.
- [ ] **Zones off.** Turn **Zones on** off, or run `zones.set` with `{"on":false}`. Shift-drag shows nothing.
- [ ] **Next and previous zone.** On Shortcuts, record a key for next zone and one for previous zone (they're not set by default). Press next with a window focused. An unsnapped window snaps to the zone holding its centre. Press again to step through the zones, and past the last one it lands in the first zone of the next screen. Previous goes back, wrapping the other way.
- [ ] **Zone hotkeys in the panel.** On Shortcuts, Zones shows both shortcuts. Record new keys and they work from another app. Record one another app holds and the row says so in red.
- [ ] **Zone key on a move key.** On Shortcuts, set next zone to the same combo as move right, then restart Seam. The log says `hotkey next_zone cleared: a directional key holds it now`, `config.json` has `next_zone` as `null`, and the move key works. Restart again: no such line.
- [ ] **API.** `zone.move` with `{"dir":"next"}` moves the focused window as the hotkey does. With zones off or no layout set it answers error 3 with the reason.
- [ ] **Excluded app.** Add an exe name such as `notepad.exe` to `zones.exclude` in `config.json` and reload. Dragging it with Shift held shows no overlay.
- [ ] **Elevated window.** Run Task Manager as administrator and Shift-drag it onto a zone. It doesn't move, and you get one balloon: "Seam can't move windows from apps running as administrator." A second try in the same session shows no second balloon.
- [ ] **Game with ClipCursor.** With zones on, run a game that confines the pointer. No overlay appears and the game isn't disturbed.
- [ ] **Windows snap flyout.** Dragging a window to the top edge still shows Windows' own layouts. Seam leaves that setting alone.
- [ ] **Hook timing.** Note `hook stats`, then drag windows with Shift held for 10 s. `own max` and `avg` match a run with nothing dragged, and the pointer stays smooth.
- [ ] **Overlay timing.** With `--log`, drag windows with Shift held across every screen of the test desk. `zone overlay: main` lines are the tray's time per change (expected well under 2000 us). `zone overlay: worker` lines give the overlay thread's own time: drawing a base once per screen after start-up or a screen change, `worker show` per change. The first drag after start-up shows zones on a screen only once its base is drawn, and later drags find them ready. Compare `hook stats` `own max` in the same minute with and without drags.
- [ ] **Changing screens.** Change a screen's resolution, then drag with Shift held. The zones match the new size, with no leftover overlay at the old one. Turn Zones off, and Task Manager's memory for `seam.exe` drops back.
- [ ] **Overlay clean-up.** After every drag no overlay stays on any screen, including when you press Esc mid-drag or close the dragged window.

## Window control

Set up first: Priority zones on screen 1, Grid on screen 2, none on the U3477. Open a few windows on every screen, some snapped and some not.

- [ ] **Move keys register.** Start Seam and check `state.get`: no `move_*` entry in `hotkey_errors`. (Win+Alt+arrows were the first choice, but Windows 11 build 26300 holds them.)
- [ ] **Move in zones.** Snap a window into screen 1's left zone and press Shift+Win+Alt+Right. It moves into the zone to its right, at the zone's size, with no jump afterwards. Win+Alt+Left brings it back. Up and down step between stacked zones.
- [ ] **Move across screens.** Press Shift+Win+Alt+Up on a snapped window on screen 1. It lands in the nearest zone of screen 2, at the right size on the 125% screen, with no jump after it settles. Press Shift+Win+Alt+Down to come back.
- [ ] **Move to a screen with no zones.** Press Shift+Win+Alt+Right on a window in screen 1's rightmost zone. It goes to the U3477 at the same place in its work area, sized for that screen's scaling, and is no longer snapped.
- [ ] **Move with no zone.** Press Shift+Win+Alt+Left on an unsnapped window on the U3477, near its top. It goes to screen 2 (the upper screen on the desk), not screen 1, in the same place there. Lower down it goes to screen 1.
- [ ] **Nothing that way.** Press Shift+Win+Alt+Up on a window on screen 2. Nothing moves, and the log says `Nothing in that direction`.
- [ ] **Focus.** Press Ctrl+Shift+Alt+Right with a window focused on screen 1. The nearest window to its right takes focus, on the same screen first, then on the U3477. Ctrl+Shift+Alt+Left from the top of the U3477 reaches a window on screen 2.
- [ ] **Focus, empty screen.** Leave screen 2 with no windows and press Ctrl+Shift+Alt+Up from screen 1. Focus stays where it was and the log says `No window on the screen in that direction`.
- [ ] **Swap.** Record keys for the swaps on Shortcuts. Snap two windows side by side on screen 1 and press swap right on the left one. They change places, each filling the other's zone. Swap an unsnapped window with a snapped one on another screen: each takes the other's place, and only the snapped place is kept as snapped.
- [ ] **Left alone.** Run Task Manager as administrator and press Shift+Win+Alt+Right on it. Nothing moves and the log says it runs as administrator. With the Seam window focused, nothing moves either. Swap towards Task Manager doesn't move it or the other window.
- [ ] **Hung window.** Hang a test app (a debugger break, or a test app that sleeps on its UI thread) and press Shift+Win+Alt+Right on it. Nothing moves and the log says it isn't responding. Focus and swap skip it.
- [ ] **Rapid presses.** Hold Shift+Win+Alt and tap Right five times quickly. The window ends up where the last press sends it from the place it had reached, with no window left half-moved.
- [ ] **Panel.** On Shortcuts, the Windows section lists move, focus and swap in each direction with their keys. Record a new key and it works from another app. Search "swap" or "focus" and the rows are found.
- [ ] **API.** `window.move`, `window.focus` and `window.swap` with `{"dir":"left"}` act on the focused window and answer `{"ok":true}`. With nothing that way, on an administrator window, or on a hung one they answer error 3 with the reason. `{"dir":"next"}` answers error -32602.
- [ ] **API focus.** From a test client run from a terminal, send `window.focus` with `{"dir":"right"}` while another app is focused. The window to the right takes focus, its taskbar button doesn't just flash, and the pointer doesn't move.
- [ ] **Hook timings unchanged.** Note `hook stats`, then use every move, focus and swap key and the API calls for 30 s while moving the pointer across the borders. `own max` and `avg` match a run without them, crossings still remap, and `SPI_SETFOREGROUNDLOCKTIMEOUT` is unchanged (`SystemParametersInfo` reads the same before and after).

## Pin, border and Release all

Set up first: record keys for **Pin window on top**, **Keep window out of workspace switches** and **Release pinned windows** on Shortcuts. Turn on the debug log.

- [ ] **Pin on top.** Focus Notepad and press the pin-on-top key. Click another window over it: Notepad stays on top. Press the key again with Notepad focused. It goes back to normal and other windows cover it again.
- [ ] **Already on top.** Keep a window on top with another tool (PowerToys Always On Top, or a video player's own setting), then press the pin key on it. The log says it's already on top, and Release all leaves it on top.
- [ ] **Left alone.** Run Task Manager as administrator and press the pin key on it. Nothing changes and the log says it runs as administrator. With the Seam window or the desktop focused, nothing is pinned either.
- [ ] **Sticky.** Make a browser window sticky. Switch to a workspace whose rule matches that browser: it isn't moved, and `workspace.switched` lists it as skipped with reason `sticky`. Switch to a workspace with **Other windows** on Minimize and then Close: the sticky window stays open where it was, and isn't listed. Capture: the sticky window isn't in the new workspace.
- [ ] **Focus border.** Turn on **Focus border** on the Zones page. The focused window's border turns the Windows accent colour, the same one Settings > Personalisation > Colours shows. Change the accent there: the border follows within a moment. Click another window: the colour moves to it, and the first one is back to the Windows default. Focus an administrator window or the Seam window: no window has the colour. Turn the switch off: the coloured border goes back to default at once.
- [ ] **Pin border.** With the focus border off, pin a window on top: its border turns amber at once. Unpin it: the border is the Windows default. Sticky does the same. With the focus border on, pin the focused window: it turns amber, not the accent colour. Click another window and back: the pinned one stays amber when it loses focus and when it gets it again. Unpin the focused window: it takes the focus colour. Pin two windows, click **Release pinned windows**: both borders are default. Close a pinned window and pin another: only the new one is amber. Pin one, kill the tray, start it again: the pinned window is amber again.
- [ ] **Apps that paint their own border.** Pin Discord (or another Chromium app) and Windows Terminal, with the focus border off. Both keep the pin colour, also after focusing them, focusing another window and coming back several times, and when a Terminal theme or Discord repaints its frame. Unpin them and turn the focus border on: both get the focus colour when focused and keep it for as long as they have focus, including right after a focus change (it may flicker for up to half a second at most). Leave a pin on for a minute: a colour an app changed back is restored within 2 s. With a `wm stats` line in the log, `max` stays well under 1000 us, and with no pin and the focus border off there are no stats lines and no timers.
- [ ] **Border swatches.** On the Zones page, Windows card, each row has eight round swatches. Pick each colour in the pin row with a pinned window on screen: the border changes at once. Do the same in the focus row with the focus border on: the focused window's border changes at once, and the pinned one doesn't. Pick **Windows accent** and change the accent in Settings: both follow. Turn the focus border off: the focus row dims, its swatches don't respond and the tooltip says why. Tab to a swatch: it shows a ring, and Enter and Space pick it. The chosen swatch always has a ring. Hover each swatch: a tooltip names it (Windows accent, Seam amber, Blue, Teal, Green, Red, Pink, White). Set `windows.pin_color` to `"nope"` in `config.json`: the pin row shows amber and pins are amber. Search for "pin border" and "focus border colour": both rows are found. `state.get` shows `wm.focus_color` and `wm.pin_color`.
- [ ] **None and custom swatches.** Each row starts with a slashed swatch (tooltip "None") and ends with a "+" swatch (tooltip "Custom colour"). Pick None in the pin row with a pinned window: its border goes back to the default at once, new pins get no colour, and the log has no wm stats lines while nothing else is painting. Pick None in the focus row with the focus border on: no window gets the focus colour, and the toggle stays on. Pick a colour again: it applies at once. Pick None for both and pin a window: the pinned window keeps whatever border its app has, nothing is re-applied. Click the "+" swatch: a colour picker opens next to it, in the panel's light or dark theme (check both), with Palette and HSLA tabs and a hex field. Pick a palette colour: the border changes at once, the picker closes, and the "+" swatch shows that colour with a ring and a check that reads well (try white and near-black). Open it again: it shows the colour you picked. On the HSLA tab drag a slider: the border follows the drag, `config.json` isn't rewritten on every tick (it holds the last colour about a quarter of a second after you stop), and alpha changes nothing. Type `#12abEF` in the picker's hex field and press Enter: the border changes and `config.json` holds `#12ABEF`. Type an incomplete or wrong value: nothing is saved. Press Escape or click outside: it closes and keeps the last colour. Pick a named swatch while a custom colour is set: the named colour applies, and opening the picker again starts on it. With a named colour set and no custom one, the picker opens on that colour. Tab to the "+" swatch and press Enter or Space: the picker opens. With a custom pin colour on Discord and Windows Terminal, switch focus back and forth: the colour stays, as with the named colours. Set `windows.pin_color` to `"#GGGGGG"` through the API: the tray stores the default, `amber`. `state.get` shows the stored strings.
- [ ] **Windows 10.** On a Windows 10 PC, the switch turns on and nothing happens, and the log has no errors.
- [ ] **Release all.** Pin two windows on top, make one sticky, and leave the focus border on. Click **Release pinned windows** in the tray menu. Both pinned windows can be covered again, the sticky one moves with workspaces again, and the border goes back to default until focus changes. The log says `released 2 pinned windows`. The release key does the same.
- [ ] **Quit.** Pin a window and quit Seam from the tray. The window can be covered again and its border is the default.
- [ ] **Update keeps pins.** Pin a window on top, then install an update from About (or let one install itself). The new tray starts with `--updated-from`, the log says `wm start: 1 pinned windows kept`, and the window is still on top.
- [ ] **Crash.** Pin a window on top, make another sticky, and leave the focus border on. Kill the tray with Task Manager, then start it again. The log says `wm start: 2 pinned windows kept, 1 borders reset`. The pinned window is still on top and the sticky one is still skipped by a switch. Release all then takes both back.
- [ ] **Closed pinned window.** Pin a window and close it. Release all says `released 0 pinned windows`.
- [ ] **API.** `window.pin` with `{"kind":"top"}` toggles the focused window and answers `{"on":true}`, then `{"on":false}`. `{"kind":"sticky","on":true}` answers `{"on":true}` twice in a row. `{"kind":"side"}` answers error -32602. On an administrator window it answers error 3. `wm.release` answers `{"ok":true,"released":n}`. `state.get` shows `wm.focus_border` following the switch.
- [ ] **Hook timings.** With the focus border on and a pinned window, move focus around for 60 s while moving the pointer across the borders. A `wm stats` line appears every 30 s with `max` well under 1000 µs, `hook stats` `own max` and `avg` match a run with the border off, and crossings still remap. Turn the border off and unpin everything: no more `wm stats` lines come, because the hooks are gone.

## Pinned window closed

Set up first: turn on **Tell me when a pinned window closes** in the Zones page's Windows card, and the debug log.

- [ ] **Off by default.** On a fresh config the switch is off. Pin Notepad and close it: no notification, and the log has no `a pinned window closed` line.
- [ ] **Win32 app.** Turn the switch on. Pin Notepad on top and close it. The tray shows "Pinned window closed" with "Notepad was pinned and has closed. Click to reopen". Click it: Notepad starts, and its new window is on top with the pin border within 15 s. The log says `reopen notepad: started, pinning its new window` and `pin Top: on`.
- [ ] **Sticky.** Make a browser window sticky and close it, then click the notification. The new browser window is sticky: a workspace switch skips it with reason `sticky`. A window that was both on top and sticky comes back as both.
- [ ] **Packaged app.** Pin Windows Terminal and close it. Click the notification: it starts through its AUMID and its window is pinned again.
- [ ] **No click.** Close a pinned window and let the notification go. Nothing starts. Seam never starts an app without the click.
- [ ] **Second close.** Pin Notepad and Paint, close Notepad, then Paint before clicking. The notification names Paint, and a click starts Paint only.
- [ ] **Nothing appears.** Pin a portable app's window, close it, then rename or delete its exe. Click the notification: nothing starts, the log says `reopen ...: not started (not installed)`, and there's no second notification. An app whose window takes longer than 15 s logs `no window within 15 s` and stays unpinned.
- [ ] **Release all doesn't count.** Pin two windows, click **Release pinned windows**, then close both. No notification.
- [ ] **Quit doesn't count.** Pin a window and quit Seam from the tray: no notification, and closing the window afterwards shows nothing either.
- [ ] **Close other windows doesn't count.** Pin Notepad on top (not sticky), and switch to a workspace without Notepad whose **Other windows** is Close. Notepad closes and no notification shows.
- [ ] **Update keeps it.** Pin a window, install an update, then close the window. The new tray shows the notification and the click reopens it.
- [ ] **Search.** Search for "pinned window closes": the row is found, and opening it scrolls the Windows card into view.

## Other window managers

- [ ] **komorebi.** Start komorebi. Within 30 s the log says `other window manager: komorebi`, the Zones page shows a note at the top naming it, and the Windows card says it was found. Drag a window with Shift held: no zones show. Press next zone and Shift+Win+Alt+Right: nothing moves and the log gives the reason. `zone.move`, `window.move` and `window.swap` answer error 3 with it. `state.get` shows `wm.other_wm` as `komorebi`.
- [ ] **Still working.** With komorebi running, crossings still remap, Line Up opens, a workspace switch runs, Ctrl+Shift+Alt+arrows still move focus, and pins still work.
- [ ] **Refit steps aside.** With **Keep windows in their zones** on and komorebi running, change the scaling of a screen. No `zone refit` line appears.
- [ ] **Stop it.** Quit komorebi. Within 30 s the note goes away and snapping works again. Repeat with GlazeWM.
- [ ] **Manual switch.** With no other window manager running, turn on **Another window manager handles my windows**. Snapping and window moves stop at once and the note shows. `state.get` shows `wm.manual_other_wm` true. Turn it off and they come back.

## Zone editor

- [ ] **Open.** On Zones, press **Edit layout** under the preview. The preview becomes the editor, with a toolbar under it: Split side by side, Split top and bottom, Merge with neighbour, Undo, Reset, Done. Press the **Custom** tile on a screen with no custom layout: it opens the editor too.
- [ ] **No layout yet.** On a screen set to None, open the editor. It starts as one zone covering the screen, and splitting it works.
- [ ] **Select.** Click a zone: it gets an amber outline and a faint amber fill. Tab to the preview, then use the arrow keys to move the selection from zone to zone. Tab again reaches every toolbar button, in order.
- [ ] **Split.** Select a zone and press **Split side by side**: it becomes two zones, with the old number kept on the first. Do the same with **Split top and bottom**. The zones renumber left subtree first.
- [ ] **Too small or too deep.** Keep splitting one zone. When its halves would be under 120 px the split buttons turn grey and a tooltip says the zone is too small to split further. Nothing crashes at any depth, and at 128 zones the tooltip says a layout can have 128 zones at most.
- [ ] **Drag a divider.** Drag a divider. The zones follow the pointer, a tag next to it shows the two widths in px, and the divider pulls to 1/4, 1/3, 1/2, 2/3 and 3/4 when it comes within 2% of them. It never goes below 10% or above 90%.
- [ ] **Merge.** Select one of two sibling zones and press **Merge with neighbour**: they become one. Select a zone whose neighbour is split into more zones: the button is grey and its tooltip says merging needs the zone next to it to be a single zone.
- [ ] **Undo.** Press Ctrl+Z after a split, a merge, a Reset and a divider drag. Each undoes exactly one step, and a drag is one step. The Undo button's tooltip names the last step, for example "Undo split (Ctrl+Z)". With focus anywhere on the page outside the editor, Ctrl+Z does nothing.
- [ ] **Reset.** Press **Reset**. The tree goes back to the template the layout started from, or to how it was when the editor opened for a custom one. It can be undone.
- [ ] **Done.** Press **Done**. The editor closes, the Custom tile is highlighted, the preview shows the tree, and `config.json` holds `"template": "custom"` plus the tree for that screen. Shift-drag a window: it snaps to the new zones, with no restart.
- [ ] **Esc.** Edit, then press Esc with focus on a toolbar button, then again with focus on the preview. Both times the edits are thrown away with no dialog and `config.json` is unchanged.
- [ ] **Leaving with edits.** Edit, then press another page in the sidebar, another screen tab or another layout tile. One dialog asks "Discard layout changes?" with Discard and Keep editing. Keep editing leaves everything as it was, Discard goes where you were headed. Without any edits there is no dialog.
- [ ] **Only Done writes.** Watch `config.json` while editing. It doesn't change until Done.
- [ ] **Every screen.** Edit the layout of the portrait U3477 and the 1920x550 panel, which are the tricky shapes. Dividers, px widths and the saved layout follow the real screen size.
- [ ] **Screens change.** With the editor open, unplug a screen. The editor closes without saving.

## On-screen editor

Do each of these in both modes where it says so, on every screen of the test desk: the 125% screen, the 100% screen, and the portrait U3477.

- [ ] **Open, edit on screen (default).** On Zones, press **Edit layout**. A borderless window appears over the selected screen's work area (the taskbar stays visible). It is dimmed, shows each zone's number and width in px centred, and you can see windows behind it. The Seam window stays in front with its toolbar, and the editor has no taskbar button and doesn't show in Alt+Tab.
- [ ] **Real pixels, each scaling.** Put a 1 px-accurate reference on the screen (a ruler or a screenshot). With a half-and-half layout the width tag reads half the work area in pixels, and the divider sits at the middle of the work area at 125%, 100% and on the portrait screen.
- [ ] **Select and drag.** Click a zone on the screen: it gets the amber outline and fill, and the preview shows the same selection. Drag a divider: it snaps like the preview (quarters and thirds), the tag shows the two widths in real px, and the preview follows. Drag one in the preview: the screen follows.
- [ ] **Toolbar.** Split, Merge, Undo, Reset and Done in the Seam window change the on-screen zones at once. Clicking on the screen doesn't bring the screen window in front of the Seam window.
- [ ] **Esc and Ctrl+Z.** With focus in the Seam window, then after clicking a zone on the screen: Ctrl+Z undoes one step and Esc closes the editor without saving, both times.
- [ ] **Live view.** Turn off **Edit layouts on the screen**, then press Edit layout. The same window appears, but clicking anywhere on it reaches the app underneath (try a link in a browser, and a title bar), the pointer doesn't change over dividers, and nothing selects. Edit in the preview: the screen follows live.
- [ ] **Switching mode.** Turn the setting on and off while the editor is open. The window is replaced with the other kind. Edits aren't lost.
- [ ] **Another screen.** With the editor open, pick another screen tab. If there are unsaved edits one dialog asks first. The window then moves to that screen, drawn at that screen's size and scaling.
- [ ] **Closing.** Done, Esc, another page in the sidebar, closing the panel, and quitting from the tray each close the window. Restart the tray while editing: the window closes. Unplug a screen, or change a resolution, while editing: it closes and `config.json` doesn't change.
- [ ] **Only Done writes.** Watch `config.json` while editing in either mode. It changes only on Done.
- [ ] **Edges.** Look for any gap, blur or offset between the window edge and the work area at 125%. Note what you see.

## Re-fit after a display change (experimental)

Snap a few windows first: a normal one on screen 1, a spanning one across two zones, one on the 125% screen and a fixed-size one. Do each check with **Keep windows in their zones** on Zones turned on, then again with it off.

- [ ] **Off by default.** On a fresh `config.json` the switch is off. With the debug log on, changing resolution writes no `zone refit` line.
- [ ] **Change resolution.** With it on, change screen 1's resolution and change it back. About two seconds after the last change the snapped windows are back on their zones, and the log has one `zone refit: N found, N moved, N skipped` line. With it off nothing moves.
- [ ] **Change scaling.** Change screen 2 from 125% to 100% and back. Snapped windows return to their zones, with no jump after they settle.
- [ ] **Unplug and replug.** Unplug the U3477, wait, replug it. Windows that were on it come back to their zones. Windows Windows moved to another screen while it was away go to that screen's zones, and a zone that doesn't exist there leaves them alone.
- [ ] **Sleep and wake.** Sleep the PC and wake it with the setting on. Windows that moved while the screens woke are put back.
- [ ] **Restart the tray.** Snap a window, quit and start Seam, then change the resolution. The window is still re-fitted. Dragging it out first stops that.
- [ ] **Moved by hand.** Snap a window, drag it somewhere else and drop it outside every zone, then change the resolution. It stays where you put it. Do the same after resizing it by its edge, and after dragging it to the top edge so it maximizes.
- [ ] **Left alone.** A minimized window, a maximized one and Task Manager run as administrator don't move and no balloon appears.
- [ ] **During a drag.** Change the resolution while holding a window mid-drag with Shift held. Nothing moves until you let go, and the re-fit runs afterwards.
- [ ] **During Line Up.** Open Line Up and change the resolution. The re-fit waits until Line Up closes.
- [ ] **Restore size.** After a re-fit, drag a re-fitted window out of its zone. It still gets the size it had before it was first snapped.
- [ ] **Hook timing.** Note `hook stats` before and after a re-fit run. `own max` doesn't change.

## Workspaces

Use the 4-screen desk with Priority zones on screen 1.

- [ ] **Page.** Workspaces is in the sidebar with no "planned" note. With none saved on this desk it shows "No workspaces for <desk> yet" and a **Capture current windows** button.
- [ ] **Capture.** Open Windows Terminal (packaged), Notepad or another Win32 app, and Discord (Squirrel). Put them in zones and press **Capture current windows**. A workspace named "Workspace 1" is selected with its name field open and focused. Type a name and press Enter. The table lists the three apps with the right screen and zone, the tray's own windows and the Seam window are not in it, and `config.json` holds the workspace under `desks[...].workspaces`.
- [ ] **Switch from closed.** Close all three apps and press **Switch now**. Each starts (Terminal and Discord through their AUMID), lands in its zone, and the card says "in use". `workspace.switched` has `launched` 3.
- [ ] **Two Firefox windows.** Capture two Firefox windows in different zones, close them, switch. Each lands in its own zone.
- [ ] **Maximized.** Capture a maximized window, switch. It comes back maximized on its screen, and un-maximizing gives its saved normal size.
- [ ] **Other windows.** Set **Leave**: nothing else changes. Set **Minimize**: other windows on this desk's screens minimize, and the Seam window doesn't. Set **Close** with an unsaved Notepad document: Notepad asks to save and nothing else is closed.
- [ ] **Unticked app.** Untick Launch on an app, close it, switch. It isn't started, and nothing is reported as an error.
- [ ] **Hung app.** Switch while an app is hung. Seam stays responsive, the other windows still land, and the log's hook timing line doesn't move.
- [ ] **Newer switch wins.** Switch to one workspace, then another straight away. The second one finishes and only it is "in use".
- [ ] **Hotkeys.** On Shortcuts, give each workspace a shortcut (Ctrl+Shift+Alt+1, +2). Both work with the Seam window closed, and the tray submenu shows them at the right.
- [ ] **Conflict.** Give two workspaces the same shortcut. The second shows the red keycaps and the conflict line, and `hotkey_errors` has `workspace:<id>`.
- [ ] **Capture shortcut.** Set one for **Capture current windows** and press it. A balloon says it saved, and clicking it opens Workspaces.
- [ ] **Tray submenu.** The tray menu has **Workspaces** with the workspaces of this desk and a check on the active one. On a desk with none it isn't there. Choose one, and it switches.
- [ ] **Edit.** Change an app's **Goes to** to another zone, to "full" and to "Keep where it is", then switch. Each does what it says. Change **Other windows** and **Start missing apps**: both stay after reopening the window.
- [ ] **Update from screen.** Move windows, press **Update from screen**. The table follows, and an app whose Launch box was off keeps it off.
- [ ] **Rename and delete.** The **...** menu's Rename opens the name field. Delete removes the workspace at once and Undo, within 8 s, brings it back.
- [ ] **Full path.** Hover an app's name in the table. A tooltip shows the full path of the program that will run.
- [ ] **Export and import.** Export a workspace on the home desk. On "Laptop only", import it: windows map by number or model, and any that match nothing say "Screen not connected" and stay where they are. A workspace with the same name gets " 2".
- [ ] **Import is untrusted.** After an import every Launch box is empty and **Start missing apps** is off. Edit the file so one rule's `exe` is `\\server\share\tool.exe` and another's is `C:\Windows\notepad.txt`, and set an `aumid` of `a b;c`. After import those three values are empty. Tick Launch and Start missing apps by hand and switch: nothing starts for them.
- [ ] **Network path refused.** In `config.json`, give a rule of a saved workspace the `exe` `\\server\share\tool.exe` (no `aumid`) and switch with that app closed. Nothing runs, and `workspace.switched` lists it in `skipped` with the reason `refused`.
- [ ] **Only drive-letter programs.** In `config.json`, try these `exe` values on a rule with no `aumid`, one at a time, with the app closed, and switch: `/\server\share\x.exe`, `\/server\share\x.exe`, `C:/Tools/x.exe`, `cmd.exe`, `..\x.exe`, `C:x.exe`, `\\?\C:\Tools\x.exe`. Each is refused (`skipped` says `refused`) and nothing runs. A real `C:\...\x.exe` starts. An `.exe` on a mapped network drive (for example `Z:\x.exe`) is refused too.
- [ ] **Import can't arm Other windows.** Export a workspace set to **Close**, import it: **Other windows** reads **Leave**.
- [ ] **Nothing matched.** Set **Close** (or **Minimize**) on a workspace whose apps are all closed and none ticked to launch, then switch. No window is closed or minimized, and `skipped` has "Other windows" with `nothing matched`.
- [ ] **Late windows are left alone.** With **Close**, switch to a workspace that starts an app with a splash screen or an updater window (Discord). The app's main window stays open when it appears after the splash, and the app's own dialogs aren't closed.
- [ ] **Switch while launching.** Switch to a workspace that starts a slow app, then pick another workspace holding the same app within a few seconds. The app is started once, and the second switch places its window.
- [ ] **Already in place.** Switch twice in a row. The second time `skipped` is empty and `placed` counts every window.
- [ ] **Re-fit after a switch.** With **Keep windows in their zones** on, snap a window to zone 2, switch to a workspace that puts it in zone 3, then change the scaling. The window stays in zone 3.
- [ ] **Wild rect.** Give a rule `"target": {"rect": [1e9, 1e9, 1e9, 1e9]}` and switch. The window ends up on screen at a sane size.
- [ ] **No leaked menu.** Open the tray menu 50 times on a desk with no workspaces. The tray's USER object count in Task Manager doesn't climb.
- [ ] **Steam.** A rule whose `exe` is under `steamapps\` is never started, and is skipped with `steam game`.
- [ ] **Desk change.** Unplug to "Laptop only". That desk has its own workspaces and shortcuts, and the old shortcuts stop working. Plug back in and they return.
- [ ] **API.** With a test client, `workspaces.list` returns the workspaces, `workspace.switch` with `{"name":"coding"}` (any case) starts a switch and a subscribed client gets `workspace.switched`, and an unknown name gets error 1. `workspace.capture` with `{"name":"From API"}` returns an id, and with `into` overwrites that workspace.

### Only real app windows

Turn the debug log on. Each check captures, then reads the table and the log. A skipped window shows as `capture skipped <class> <exe>: <reason>`, and the capture line ends with a count per reason. Hidden and cloaked windows are only counted.

- [ ] **Busy desk.** With everything below open, capture. The table has only apps with a taskbar button, and the log's `workspace capture: N windows in X ms` is under 100 ms with about 30 windows open.
- [ ] **Discord overlay.** Start a game with Discord's overlay on and capture. Discord's main window is in the table, the overlay isn't (`no activate` or `transparent`).
- [ ] **NVIDIA or AMD overlay.** With GeForce Experience / the NVIDIA app or AMD Software running, capture. Their overlay and helper windows aren't in the table.
- [ ] **Xbox Game Bar.** Open Game Bar with Win+G, close it, capture. Nothing from `gamebar.exe` is in the table (`system`, or `cloaked`).
- [ ] **Tray app.** Close Spotify to the tray and capture: it isn't in the table (`hidden`). Show it again and capture: it is.
- [ ] **File Explorer.** Open two folders and capture. Both folder windows are in the table. The taskbar, the desktop and the tray overflow aren't (`shell`).
- [ ] **Dialogs and tool windows.** Open Notepad's Save as dialog and capture. Notepad is in the table and the dialog isn't (`owned`). A palette or tool window of an app isn't either (`tool window`).
- [ ] **Tiny and parked windows.** The log lists some helpers as `too small` or `off screen`, and none of them is in the table.
- [ ] **Input host.** Open the touch keyboard or emoji panel (Win+.), capture. `textinputhost.exe` isn't in the table.
- [ ] **Never include.** Add `notepad.exe` to the zones' exclude list in `config.json`. Capture with Notepad open: it isn't in the table, and the log says `never include`.
- [ ] **Pins still work.** Pin a Notepad window on top, then pin File Explorer. Both pin. A dialog or the taskbar can't be pinned.

### Window state

- [ ] **Gate A, all states.** Minimize two windows, maximize one, put a browser in full screen with F11 and leave one window normal, with focus on the normal one. Capture. In `config.json` the rules have `"shown"` `minimized` (twice), `maximized`, `fullscreen` and `normal`, a `"z"` from 0 up, and `"focused": true` on the one window that had focus. Rearrange everything: restore, un-maximize, leave full screen, move and restack. Switch back. The two windows are minimized again, and restoring them puts each in its zone. The maximized one is maximized on its screen, the browser is full screen again, the stacking is as it was, and focus is on the window that had it.
- [ ] **Minimized stays minimized.** Capture a workspace with a minimized window, restore that window and move it, switch. It's minimized again without showing on screen first, and restores into its zone.
- [ ] **Minimized on another screen.** Capture a minimized window whose zone is on screen 2, move it to screen 1 and minimize it, switch, restore it. It opens in its zone on screen 2.
- [ ] **Full screen lost its frame.** Capture a browser in F11 full screen, press F11 again so it has its title bar back, switch. It isn't forced into full screen, and `skipped` has it with `not full screen`.
- [ ] **PowerPoint slideshow.** If PowerPoint is installed, capture a running slideshow on screen 2, move it, switch. It goes back over the whole of screen 2.
- [ ] **Gate A, no-op switch.** Switch to the active workspace twice. The second time the log says `<name>: switched in X ms, plan Y ms, 0 moves, N already in place` with X under 30, nothing flickers, and `placed` still counts every window.
- [ ] **Two windows moved.** With every app running, move two windows out of place and switch. The log's switch line has `2 moves` and is under 300 ms.
- [ ] **Across a DPI change.** Move a window that belongs on the 100 % screen to the 150 % one, switch. It lands exactly in its zone, after one short pause only.
- [ ] **Focus without the hotkey.** Switch from the tray menu and from the API. The focused window from the capture gets the focus, and the log doesn't say `Windows kept the focus where it was`.
- [ ] **Old workspaces.** With a `config.json` from before this version, which has `"state": "max"` targets and no `shown`, switch: maximized windows come back maximized and nothing else changes. `config.json` isn't rewritten until something else is saved, and then those rules have `"shown": "maximized"`.
- [ ] **Administrator window.** Capture with an app running as administrator. A switch skips it with `elevated` and never changes its state or its stacking.
- [ ] **Gate A, hook.** During all of the above, the log's hook timing lines are the same as before.

### Editing a workspace

Each change below saves at once, with no Save button, and shows a notification with **Undo** for 8 s. Undo puts the workspace back as it was before that change. Check `config.json` after each one.

- [ ] **Goes to.** Pick "Screen 2, zone 1" for an app: the notice says "<app> goes to Screen 2, zone 1" and a switch puts it there. "Screen 1, full" maximizes it on screen 1. "Keep where it is" leaves it alone on a switch. The list has every zone of every screen, a "full" entry per screen, "Keep where it is" and "Where it is now".
- [ ] **Where it is now.** With the app open, move its window to another zone and pick **Where it is now**. The row says the new zone, and Undo gives the old one back. With the app closed it says "Where <app> goes stays as it was" and "That app has no window open".
- [ ] **Show as.** Open a row with a click on its name. Pick Minimized, switch: the window is minimized and restores into its place. Maximized sets Goes to "Screen N, full". Normal from Maximized puts the window back at the size it had before it was maximized.
- [ ] **Launch.** Untick an app: it doesn't start on a switch with it closed. Tick it again.
- [ ] **Reorder by drag.** Drag a row by its grip below the next one. The notice says "Moved <app> to place 2", and `windows` in `config.json` is in the new order.
- [ ] **Reorder by keyboard.** Tab to a row, press Alt+Down then Alt+Up. The row moves each time, the focus ring stays on it, and Alt+Up on the first row or Alt+Down on the last does nothing.
- [ ] **Remove.** "..." on a row, **Remove**. It's gone at once, and Undo brings it back in the same place.
- [ ] **Rename and switches.** Rename, the shortcut on Shortcuts, **Start missing apps** and **Other windows** still save, without a notice.
- [ ] **Refused.** With the tray stopped, change anything: "Seam isn't running" shows and nothing changes. In `config.json`, give a rule a zone the screen's layout doesn't have, then rename the workspace: that still saves, and the rule keeps its zone.
- [ ] **Not the panel.** A test client that isn't the panel gets error 4 from `workspace.update`.

### Adding an app

- [ ] **Open now.** **Add an app** opens a popover with a search field and the tabs **Open now** and **Installed**. Open now lists the app windows with a taskbar button, one per window, with each window's title under the app's name, and nothing from Seam. Click one: the popover closes, the app is the last row with "Keep where it is" and Launch ticked, and "Added <app>" has Undo.
- [ ] **Installed.** The Installed tab lists the Start menu's apps by name with their icons, and no services, uninstallers or links. Search "note": only matching apps stay. Search for something that isn't there: "Nothing matches ...".
- [ ] **A Store app.** Add Calculator from Installed with it closed. The row says "Packaged app". Switch: Calculator starts through its AUMID and isn't listed as `refused`.
- [ ] **Already there.** An app that's already a row shows "In this workspace" in both tabs. Clicking it adds a second row, for a second window of that app.
- [ ] **Never include.** "..." on a row, **Never include this app**. The row goes, its exe is in **Never manage these apps** on General and in `zones.exclude`, and a capture leaves it out. Undo brings the row back and takes it off the list. On General, **Remove** takes an app off the list with Undo. A Store app added from Installed has no Never include until a capture or Where it is now fills in its exe.

### Launch stages

Turn the debug log on. Close every app of a test workspace, with **Start missing apps** on and every app ticked.

- [ ] **Everything at once.** With no delay and nothing waiting, switch. Every app starts straight away, the log has one `stage 1 of 1` line, and each `<app> showed after X ms` line comes as its window appears, so the first windows are placed before the slowest app is up.
- [ ] **Delays.** Give the second app a 5 s delay (the row says "5 s after its stage starts") and switch. It starts about 5 s after the first. The slider shows "0 s" and "30 s" at its ends and moves in half seconds.
- [ ] **Wait for a window.** Tick **Wait for this app's window before opening the next** on the first app: a line with "then wait" shows under it. Switch: the apps below start only once its window is up, and the log has `stage 1 of 2` and `stage 2 of 2` with their times.
- [ ] **Took too long.** Make the waiting app one that doesn't open a window (a tray-only app) or that is slow, and switch. After 15 s the next stage starts anyway, the log's stage line ends with `too long: <app>`, and `workspace.switched` lists it in `skipped` as "Took too long to open".
- [ ] **Already running.** Leave one waiting app with a delay running and switch. It's placed at once, before anything starts, with no delay and no wait.
- [ ] **Start minimized.** Set an app to Show as Minimized, close it, switch. It starts and ends up minimized, and restores into its place.
- [ ] **Event-driven.** During a switch the log has no gap of a second between a window showing and its `showed after` line. With nothing waiting, the wm thread's stats lines show no extra events.
- [ ] **Newer switch.** Switch to a workspace with a 10 s delay, then to another within 2 s. The delayed app doesn't start.

### Update from screen keeps your edits

- [ ] **Merge.** Set a delay and a wait on two apps, reorder them and add an app from Installed that isn't running. Move the running windows and choose **Update from screen**. The running rows take their new places and keep their delays, waits, Launch ticks and order. The app that isn't running is still there. A window that wasn't in the workspace is added at the end.
- [ ] **Nothing deleted.** Close every app and choose Update from screen. Every row is still there.

### Pinned windows and switches

- [ ] **Off.** With **Pinned windows stay when I switch workspaces** off on Zones, pin Notepad on top and switch to a workspace that puts Notepad in a zone: it moves, and stays on top. With **Other windows** on Minimize, a pinned window that isn't in the workspace is minimized.
- [ ] **On.** Turn it on and do the same. Notepad isn't moved, `skipped` lists it as `pinned`, and Minimize and Close leave it alone. Capture: Notepad is in the new workspace. A sticky window is still left out of the capture.
- [ ] **API.** `config.get` has `windows.pins_stay_on_switch`.

### Live status and Snap back

Turn the debug log on. Use a workspace with four apps on two screens, switch to it, and leave the Workspaces page open on it.

- [ ] **In place.** After the switch every running app's row shows a filled green dot and "In place" in the **Now** column, a closed app a filled grey dot and "Not running", and no **Put windows back** button shows.
- [ ] **Will open.** For the closed app with Launch ticked and **Start missing apps** on, "Will open" shows under Goes to. Untick Launch: it goes. Turn Start missing apps off: it goes too.
- [ ] **Moved.** Drag one window to another zone. Within a second, with no click in the panel, its row shows a hollow ring and "Moved", and **Put windows back** appears under the table. The log has `workspace <id>: drifted`.
- [ ] **Minimized.** Minimize a window the workspace shows normally: "Minimized" with a hollow ring. A window set to Show as Minimized that is minimized shows "In place".
- [ ] **Administrator.** Start an app of the workspace as administrator: its row shows a red ring and "Can't move: runs as administrator", and a switch skips it.
- [ ] **Snap back from the panel.** Press **Put windows back**. The moved and minimized windows go back, the rows show "In place", the button goes, and the log has `workspace <id>: in place`.
- [ ] **Another workspace.** Select a workspace that isn't in use, move one of its running apps' windows: the rows follow it too, and its **Put windows back** switches to it.
- [ ] **Hotkey.** Set **Snap back to this workspace** on Shortcuts (it has no key until you set one). Move a window, press it: the window goes back. With no workspace in use (just after plugging a screen in or out), the log says `snap back hotkey: No workspace is in use on this desk`.
- [ ] **API.** `workspace.status {id}` answers `[{rule_index, state}]` with `in_place`, `moved`, `minimized`, `not_running` or `cant_move`, and error 1 for an id that isn't on this desk. `workspace.snap_back` answers `{ok: true}` and does what the button does, or error 1 with no workspace in use. `state.get` has `workspace.drifted`, true while a window is moved or minimized and false after Snap back. A subscriber to `workspace` gets `workspace.moved {id}` about half a second after a drag of that workspace's window ends.
- [ ] **No polling.** With the page open and nothing moving for a minute, the panel asks nothing: the log has no `drifted` or `in place` lines, and the wm stats lines show no events beyond the focus border's.
- [ ] **Only while shown.** On another page, drag a window: the Workspaces rows are right as soon as you come back.

### Auto-apply (Gate B)

Turn the debug log on, and **Put apps in place when they open** on, at the bottom of the Workspaces page (it's off on a new install; `autoapply.set {on}` does the same and answers `{on}`). Switch to a workspace whose apps are closed, with Start missing apps off, so the switch starts nothing.

- [ ] **Gate B, three apps by hand.** Open three of the workspace's apps from the Start menu. Each lands in its zone once, about 150 ms after its window shows, and the log has `auto-apply <workspace>: <app> for row N: placed in X ms` for each. Their rows show "In place".
- [ ] **Gate B, drag it away.** Drag one of them elsewhere. It stays where you put it, and its row shows "Moved".
- [ ] **Gate B, second window.** Open a second window of one of those apps (a new Firefox or Explorer window). It's left alone and the log has no auto-apply line for it.
- [ ] **Gate B, an app not in the workspace.** Open an app the workspace doesn't have. Nothing moves it and nothing is logged.
- [ ] **Gate B, during a switch.** Close the apps, turn Start missing apps on, switch, and open two of its apps by hand while the switch is still starting the others. Every window moves at most once: no window jumps twice, and the log has no auto-apply line for a window the switch placed.
- [ ] **Gate B, hook timings.** With auto-apply on, open and close apps and drag windows for 60 s while crossing the borders. `hook stats` `own max` and `avg` match a run with auto-apply off, and the `wm stats` lines every 30 s have `max` well under 1000 µs.
- [ ] **Drag while it settles.** Open an app and start dragging its window at once. Auto-apply leaves it where you drop it.
- [ ] **Restore isn't an open.** Minimize a window you dragged away and restore it. It isn't put back in its zone. Close an app to the tray and show it again: it isn't moved either.
- [ ] **Reopened.** Close one of the three apps and open it again. It goes back into its zone, because its row is free again.
- [ ] **Never manage.** Add an app of the workspace to **Never manage these apps** on General, then open it: it's left where it opens.
- [ ] **Administrator.** Open an app of the workspace as administrator: it's left alone without a notice.
- [ ] **Off.** Turn the switch off and open an app of the workspace. It stays where it opens, and the log's `wm: workspace in use true, auto-apply false` line shows the hooks changed.
- [ ] **No workspace.** Plug a screen in or out, so no workspace is in use, and open an app: nothing happens.
- [ ] **Another window manager.** With komorebi running, auto-apply does nothing.
- [ ] **Release all and quit.** Release pinned windows, then open an already placed app's second window: it's still left alone. Quit Seam and start it again: the windows placed before keep their stamp, so opening the workspace's apps again only places the ones with a free row.

### Keep physical size

Turn the debug log on. Use two screens of different density side by side, for example the 4K 27" at 150% and a 1440p 27" at 100%, plus the 1920×550 HDMI panel, which reports no size. Keep a ruler at hand. Every resize logs `physical size: <app> screen A → B, W×H px → W×H px (mm W×H)`.

- [ ] **Displays page.** Under Pointer, **Keep window size in centimetres between screens** is on, and **Keep size in centimetres when I drag between screens** is off. Its description says "Screen N doesn't report its size, so windows keep their pixel size there" with the HDMI panel's number. Unplug the HDMI panel: it says "Moves by key, by other apps and by workspaces to a screen of another density". Turn the first switch off: the second dims and a click on it changes nothing. Search finds both by "centimetres" and "dpi". `config.get` has `keep_physical_size` and `keep_physical_size_drag`.
- [ ] **Key move, no zones there.** Turn zones off on the 1440p screen. Put Notepad at its own size on the 4K screen, measure it, and press Shift+Win+Alt+Right until it moves to the 1440p screen. It measures the same, its centre is where the move put it, and the log has the line with screen numbers. Back again: same size.
- [ ] **Key move, too big.** Make Notepad nearly as wide as the 4K screen and move it. It's clamped to the 1440p work area, with no part off screen.
- [ ] **Key move into a zone.** With zones on the 1440p screen, move a window in a zone across. It takes the zone's size, and no `physical size` line is logged.
- [ ] **Swap.** Set a Swap key. Swap a free window on the 4K screen with a free one on the 1440p screen. Each keeps its own size in centimetres, centred where the other was.
- [ ] **API.** `window.move {dir:"right"}` from a script does what the key does, with the log line.
- [ ] **Setting off.** Turn the first switch off and move Notepad across: it's sized by Windows' scaling, as before, with no log line.
- [ ] **No EDID size.** Move Notepad onto the HDMI panel: it keeps its pixel size, and the log has one line saying the screen doesn't report its size.
- [ ] **Maximized and full screen.** Move a maximized window across: it's maximized on the other screen and nothing is logged.
- [ ] **Workspace, same screen.** Capture a workspace with Notepad at its own size. Move the taskbar to the left or turn auto-hide on, then switch: Notepad uses the saved fractions as before, with no `physical size` line.
- [ ] **Workspace, other screen.** Capture with Notepad at its own size on the 4K screen. Copy the workspace with Export, unplug the 4K screen, and switch: Notepad goes to the same place on the screen it's on now, at the size it measured at capture, and the log has `physical size: notepad screen at capture → N`. A workspace captured before this version (no `size_mm`) leaves it where it is, as before. Import the exported file on a desk where its screen number is the 1440p screen and switch: same size in centimetres.
- [ ] **Workspace on a screen with no size.** Capture on the HDMI panel: the rule has no `size_mm` in config.json, and switching always uses the fractions.
- [ ] **Auto-apply.** With the 4K screen unplugged and **Put apps in place when they open** on, open Notepad from that workspace by hand. It's placed once at its captured size in centimetres, with the log line.
- [ ] **Free drag, off.** With the drag switch off, drag Notepad from the 4K screen to the 1440p screen with the mouse. Windows' own scaling decides, and nothing is logged.
- [ ] **Free drag, on.** Turn the drag switch on and drag it again. About 120 ms after you let go it resizes to the same centimetres, centred where you dropped it, with the log line. A drag within one screen, a drag into a zone with the show key, and a drag of a maximized window off the top don't resize. A snapped window dragged out with **Restore size when dragged out** on gets its old size, not this one.
- [ ] **Free drag with zones off.** Turn zones off altogether: the drag switch still works. With komorebi running, or **Another window manager handles my windows** on, nothing resizes after a drag.
- [ ] **Hook timings.** With the drag switch on, drag windows across the borders for 60 s. `hook stats` `own max` and `avg` match a run with it off.

### Quick save

Turn the debug log on. Set **Save windows to the active workspace** on Shortcuts (it has no key until you set one).

- [ ] **Into the workspace in use.** Switch to a workspace, move two of its windows and open an app it doesn't have. Press the key. A notification says "Saved <name>" and "N windows saved. Click to undo". The editor shows the moved rows at their new places, the new app at the end, and the launch details and order of the rest as they were, the same as **Update from screen**. The log has `quick save: N windows into <name>`.
- [ ] **Undo.** Do it again and click the notification within 8 s. The workspace is as it was before the save, and the log has `quick save: undone`. Click a quick save's notification after 8 s (from the Action Center): nothing changes and the log says `quick save: too late to undo`.
- [ ] **No workspace in use.** Plug a screen in or out, so no workspace is in use, and press the key. A new workspace "Workspace N", with the next free number, appears on this desk, the notification says it's now in use, and the Stream Deck workspace key for it lights. Click the notification within 8 s: the workspace goes, and none is in use again.
- [ ] **Busy.** Press the key twice quickly: the second press logs `quick save hotkey: Already capturing`.
- [ ] **API.** `workspace.quick_save` answers `{id, created}` once the capture is saved, with the same notification. Any client may call it.
- [ ] **Stream Deck.** Put the **Save windows** action on a key and press it: the same as the hotkey.

## Settings panel

- [ ] **Open.** Choose **Open Seam…** in the tray menu. The panel opens in your Windows theme on the last page you used, and the sidebar shows the current remap state.
- [ ] **Icon.** The Seam mark shows on the title bar, the taskbar button, and both exes in Explorer.
- [ ] **Single panel.** Choose **Open Seam…** again. The open panel comes to the front and no second one starts.
- [ ] **By hand.** Close the panel and run `seam-settings.exe` directly, with Seam running and then with it closed. Each time the panel opens through the tray, and in the second case the tray starts too.
- [ ] **Tray quits.** Quit Seam with the panel open. The panel says Seam isn't running, and **Start** brings it back without restarting the panel by hand.
- [ ] **Light and dark.** Switch Windows between light and dark with the panel open. It follows straight away, and every page reads clearly in both.
- [ ] **Map.** Displays shows every screen at its real size, with amber on lined up borders. Clicking a screen flashes its number on that screen.
- [ ] **Remap switch.** Turn it off. Crossings stop remapping and the sidebar says paused. Turn it on and they work again.
- [ ] **Align switch.** Turn it off, then open the tray menu. **Also align Windows arrangement** is unticked. Turn it back on.
- [ ] **Rename.** Rename a desk from its **…** menu. The sidebar and tray menu show the new name.
- [ ] **Forget and Undo.** Forget the desk in use. The borders go to "Not lined up", the page keeps a sensible desk name, and **Saved desks** says there are none. Press **Undo** within 8 seconds and everything comes back. Forget it again, wait, and the notification goes on its own. Line up to save it again.
- [ ] **Record a shortcut.** On Shortcuts, press **Change**, then Ctrl+Alt+Shift+K. The new keys show and work from another app.
- [ ] **Recorder keys.** Press **Change** then Esc: nothing changes. Press **Change** then Backspace: the row says "Not set" and stays that way after quitting and restarting Seam. **Record** a shortcut again.
- [ ] **Shortcut problems.** Record Ctrl+Alt+K: the row warns that it types characters. Record a shortcut another app holds: the row says so and the keycaps turn red.
- [ ] **Keyboard.** Ctrl+F jumps to search and Esc clears it. Tab walks the sidebar then the page, with a ring on each control, and no ring appears after a mouse click. Space flips a switch. Left and Right change the Tray icon choice.
- [ ] **Search.** Type "log" and press a result. It opens that page.
- [ ] **Search scrolls to the row.** Make the panel window short enough that every page scrolls. On each page, open a search result for a row near the bottom and check the row is in view with some room above it, starting from the top and from the bottom of another page: Displays "Also align Windows arrangement", Zones "Pin border colour" and "Another window manager handles my windows", Workspaces "Other windows" (with a workspace selected), Shortcuts "Capture current windows", General "Debug log", About "Licenses" (shows the Help section) and "Include pre-releases". A result at the top of its page, such as Displays "Line up screens", opens the page at the top. Scrolling by hand afterwards works as before, and picking a page in the sidebar keeps working as it did.
- [ ] **Left-click.** Choose **Open Seam** under Tray icon, then left-click the tray icon. The panel opens. Set it back.
- [ ] **Start with Windows.** Turn it on, sign out and in. Seam starts, and the panel doesn't. Turn it off.
- [ ] **Debug log.** Turn it on, cross a border, and check `seam.log` grows. Turn it off.
- [ ] **Files.** **Open** shows the folder holding `config.json`. **Export…** then **Import…** round-trips your settings, and importing a broken file is refused with nothing changed.
- [ ] **About.** **Copy diagnostics** puts your version, Windows build, screens and borders on the clipboard, with no window titles or file paths.
- [ ] **Panel missing.** Rename `seam-settings.exe`, choose **Open Seam…**. A balloon says it's unavailable and Seam keeps running. Rename it back.

## Starting and signing in

- [ ] **By hand.** Run `seam.exe` with nothing running. The tray icon appears and the Seam window opens.
- [ ] **Sign-in.** Turn on **Start with Windows**, sign out and in. Seam is in the tray and no window opens. `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` holds `"…\seam.exe" --autostart`.
- [ ] **Run key from v0.1.** Put the old value, the quoted path alone, in that key. Start Seam. The value now ends in `--autostart`.
- [ ] **Window from the panel.** Quit Seam, run `seam-settings.exe --page about`. The tray starts, and one window opens on About.
- [ ] **`--line-up` and updates.** `seam.exe --line-up` opens Line Up and no window. A restart after an update opens no window either.

## Restoring the last session

Set up first: turn on **Start with Windows** and **Offer to restore my windows after I sign in** on General, and the debug log. Have zones on two screens.

- [ ] **Off by default.** On a fresh config the switch is off. Snap a few windows and wait a minute: `config.json` has no `last_session`, and the log has no `last session saved` line.
- [ ] **Saved after changes.** Turn the switch on. Snap a window into a zone, pin another on top and make a third sticky. About 30 s after the last of these, the log says `last session saved: n windows`, and `config.json` holds `last_session` under the desk, with `"pin": "top"` and `"pin": "sticky"` on those rules. Keep snapping every few seconds: no save until 30 s after you stop. Leave the PC alone for 5 minutes: no more saves.
- [ ] **Hidden.** The Workspaces page, the tray menu's Workspaces, Shortcuts and `workspaces.list` don't show a "Last session". `state.get` shows `session` with `saved_at` and `windows` for this desk, and `null` on a desk that has none.
- [ ] **Sign out and in.** Arrange Notepad, Windows Terminal and a browser in zones, pin Notepad on top, and sign out within 30 s of the last change. The log ends with `last session saved`. Sign in: about 5 s after the desktop appears, "Last session" says "Restore your last session? Click to restore". Click it with **Who reopens the apps** on Seam: missing apps start, every window goes back to its zone, and Notepad is on top with the pin border again. Windows that were already open, and anything else on screen, are left where they are.
- [ ] **No click.** Sign out and in again and let the notification go. Nothing starts and nothing moves.
- [ ] **Reboot.** Restart Windows with the same windows open. After sign-in the offer shows, and the click restores them as above. Window handles changed, so the pins come back by app.
- [ ] **Apps that start themselves.** Have Discord or another app that starts at sign-in in the session. After sign-in, click the offer: Seam uses the window that's already open, starts no second copy, and puts it in its zone.
- [ ] **Not at a hand start.** Quit Seam and start it from the Start menu: no offer. `seam.exe --updated-from x` after an update: no offer.
- [ ] **Slow shell.** If the notification can't show yet at sign-in, Seam tries again every 5 s for a minute.
- [ ] **Turned off.** Turn the switch off: `config.json` no longer has a `last_session` under any desk. Snap windows and sign out and in. No new save happens and no offer shows.
- [ ] **Shutdown is quick.** Sign out with many apps open: Windows doesn't list Seam as preventing sign-out. A capture that takes longer than 2 s leaves the earlier session saved, and the log says so.
- [ ] **Other desk.** Unplug to "Laptop only" and sign out and in. The offer is for that desk's own last session, or there is none.
- [ ] **API.** `session.restore` from the panel token or a Seam process runs the restore and a subscribed client gets `workspace.switched` with id `last_session`. From another client it answers error 4. With nothing saved for the desk it answers error 1.
- [ ] **Search.** Search for "restore": the General row is found, and opening it shows the row.

## Who reopens the apps

Set up first: turn on **Offer to restore my windows after I sign in** and the debug log. Check Windows' setting with `reg query "HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon" /v RestartApps` (Settings > Accounts > Sign-in options, "Automatically save my restartable apps and restart them when I sign back in").

- [ ] **Shown with restore only.** **Who reopens the apps** shows under the restore switch only while it's on, set to **Windows reopens them**, with a one-line caption that changes with the choice. `config.json` has `"restore_apps": "windows"` and `"restore_apps_saved": null`.
- [ ] **Confirm, cancel.** With Restart apps on (`RestartApps` 0x1), pick **Seam reopens them**. "Turn off Windows' Restart apps?" opens. **Cancel**: the choice stays on Windows, and `reg query` still shows 0x1.
- [ ] **Confirm, turn it off.** Pick it again and press **Turn it off**. `reg query` shows 0x0, the Sign-in options switch is off after reopening Settings, `config.json` has `"restore_apps": "seam"` and `"restore_apps_saved": 1`, and the log says `Windows' Restart apps turned off (was 0x1)`.
- [ ] **Switch back.** Pick **Windows reopens them** (no dialog). `reg query` shows 0x1 again, `restore_apps_saved` is `null`, and the log says `given back`.
- [ ] **Restore off gives it back.** Choose Seam again, then turn **Offer to restore my windows** off. `reg query` shows 0x1 and `restore_apps_saved` is `null`. Turn restore on again: the choice is still Seam, and the value is 0x0 again.
- [ ] **Stored once.** Before picking Seam, set the value to 5 (`reg add "HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon" /v RestartApps /t REG_DWORD /d 5 /f`). Pick Seam: `restore_apps_saved` is 5. Quit and start Seam twice: it stays 5. Switch back: `reg query` shows 0x5.
- [ ] **Value missing.** Switch to Windows mode, delete the value (`reg delete "HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon" /v RestartApps /f`), and pick Seam. `reg query` shows 0x0 and `restore_apps_saved` is `4294967295`. Switch back: the value is gone again, not 0.
- [ ] **Turned back on by hand.** In Seam mode, turn Restart apps back on in Sign-in options, then quit and start Seam. Seam leaves it on, the log says `Windows' Restart apps is on again, so apps may open twice`, `state.get` shows `restart_apps: true`, and General shows that note under the choice with **Open Sign-in options**, which opens Settings on Sign-in options.
- [ ] **Registry refused.** If writing the value fails (for example a policy locks it), the log says so, nothing crashes, and the choice stays as picked.
- [ ] **Sign in, Windows mode, Restart apps on.** Leave Notepad, Windows Terminal and a browser open in zones, one pinned, and sign out. Sign in and click the offer: Seam starts nothing. As Windows reopens the apps, each window goes to its zone and the pinned one is pinned again, within 60 s. The log says `Last session: placed n of m, never showed: ...` naming apps Windows didn't reopen, and focus isn't taken.
- [ ] **Sign in, Windows mode, Restart apps off.** Same, with Restart apps off: only apps that start themselves or are already open are placed. The rest are named in the log line after 60 s, and nothing starts.
- [ ] **Sign in, Seam mode.** With Restart apps turned off by Seam, sign out and in and click the offer: Seam starts the missing apps once each, with no second copies from Windows.
- [ ] **Sign in, Seam mode, Restart apps on by hand.** Turn it back on in Settings, sign out and in, and click the offer: apps may open twice, as the note on General says.

## Updates (GitHub build)

Needs the real minisign key in `SEAM_PUBLIC_KEY`, and a published pre-release on GitHub that's newer than the build under test. Cut `v0.2.1-beta.1` from a build with the key, mark it as a pre-release, and publish it. Test with a 0.2.0 build in a folder you can write to. Delete the pre-release afterwards.

- [ ] **No key.** On a build with the placeholder key, **Check now** ends in "This build has no update key yet, so no update can be trusted", with **Try again**. Nothing is downloaded, and `%LOCALAPPDATA%\Seam\updates\state.json` holds the same text in `error`.
- [ ] **Pre-releases off.** With **Include pre-releases** off, **Check now** says "Seam is up to date" and About shows no card.
- [ ] **Check and download.** Turn on **Include pre-releases** and press **Check now**. The card shows "Checking for updates", then "Downloading Seam 0.2.1-beta.1" with a bar, "X of Y MB" and a "0 MB" to "Y MB" range, then "Seam 0.2.1-beta.1 is ready to install" with the date, size, notes and "signature checked". The About entry in the sidebar gets an amber dot.
- [ ] **Notification.** A balloon says an update is ready, once. Click it and About opens. Quit and start Seam again: no second balloon for the same version.
- [ ] **Daily check.** Leave **Check for updates every day** on. Two minutes after Seam starts, `state.json` has a new `last_checked`, with no window and no console flash. Turn it off, restart, and wait three minutes: it doesn't change.
- [ ] **Bad download.** Change one byte of a file in `updates\<version>\` and press **Check now**. The folder is replaced and the check succeeds. Then publish a copy of the release with a broken `latest.json.minisig`: the check is refused with "The update's signature doesn't match Seam's key".
- [ ] **Install and restart.** Press **Install and restart**. Seam closes and comes back on the new version with your borders, and a balloon says "Updated to 0.2.1-beta.1". A minute later no `seam.exe.old` or `updates` folder is left.
- [ ] **Can't install here.** Put both exes in `C:\Program Files\Seam` and run from there with an update ready. **Install and restart** ends in "Can't install here" with **Download**, which opens the release page. Nothing changed.
- [ ] **Tray from another folder.** Run `seam.exe` from a second folder and `seam-settings.exe` from the first, with an update ready. **Install and restart** ends in "Seam is running from another folder, so nothing was changed". Neither tray closes and nothing starts.
- [ ] **Older release.** Stage an older signed release in `updates\<version>\`, and `--install` it. It's refused with "That isn't newer than this version of Seam".
- [ ] **Staged file swapped.** Change one byte of a staged exe right after the install starts (or use a debugger pause before the copy). Install is refused, the old version is back and the tray is running, and no `.new` file is left.
- [ ] **Rollback.** Make `seam-settings.exe` read-only and locked by another program, then install. Seam is back on the old version, the error is shown, and the tray is running.
- [ ] **On their own.** Turn on **Install updates on their own**. While you use the PC nothing happens. Leave it alone for two minutes with an update ready, and Seam restarts on the new version with the balloon. With Line Up open, it waits.
- [ ] **No network.** Turn Wi-Fi off and press **Check now**. About shows one line saying what happened, with **Try again**, and the remap keeps working.
- [ ] **Hook timing.** Compare `hook stats` across a check and an install with a run without one. Own max stays well under 1000 µs.

## Installer

Needs a `Seam-Setup` built by CI (the `installer` artifact of a run on `main`), and the previous release's setup or zip for the upgrade tests. Use a user that has no Seam, or remove `%LOCALAPPDATA%\Programs\Seam` first.

- [ ] **Fresh install.** On a user with no Seam, run the setup. There is no admin prompt, and no folder page. The Start menu has a Seam entry, Installed apps shows Seam and its version, and Seam starts from the finish page.
- [ ] **Upgrade.** Run a newer setup over an older one while Seam runs. Seam is quit and restarted, your settings are kept, and Installed apps shows the new version.
- [ ] **Built-in update.** On an installed copy, update from About. Afterwards Installed apps shows the new version, and `*.old` files stay in the folder until the next start.
- [ ] **Portable to installer.** Run the portable zip with **Start with Windows** on, then install. The portable tray is quit and the installed one runs. After signing out and in, only the installed copy starts.
- [ ] **Uninstall, keep settings.** Uninstall and answer No to the settings question. Seam is quit, and the folder, the Start menu entry, the Run value, the `*.old` files and `%LOCALAPPDATA%\Seam\updates` are gone. `%APPDATA%\Seam` is still there.
- [ ] **Uninstall, remove settings.** Answer Yes to the question. `%APPDATA%\Seam` is gone.
- [ ] **Uninstall with a portable copy owning autostart.** Install, then run a portable copy so the Run value points at it. Uninstall. The Run value is left alone.
- [ ] **Silent.** `Seam-Setup-x.y.z.exe /VERYSILENT` installs without starting Seam, and `unins000.exe /VERYSILENT` keeps your settings.
- [ ] **Store test package plus installed copy.** The dual-install guard works as before: the Store copy wins, and the banner names the installed copy.
- [ ] **Updater rollback when the new tray fails.** Stage an update, then make the new `seam.exe` unable to start, for example by replacing the staged copy with a zero-byte file after verification, in a debug build only. After install, the old version is back and running, About shows the error, and Installed apps still shows the old version.
- [ ] **Foreign download URL.** In a debug build, point an asset URL somewhere else. The check fails with "The release points somewhere unexpected" and nothing is downloaded.
- [ ] **Quit still running.** With Seam running from the installed folder, `seam-settings.exe --quit-tray` exits 0 and Seam is gone. With nothing running it also exits 0.
- [ ] **Hook timing.** Paste the `hook::log_stats` lines. Own max stays well under 1000 µs, as before.

## Release dry run (x64 and ARM64)

Needs a pre-release tag such as `v0.5.0-beta.1` pushed to my repo, so `release.yml` drafts it, and an ARM64 PC for the last three. Delete the draft and the tag afterwards.

- [ ] **Assets.** The draft holds `Seam-Setup-X.Y.Z.exe`, `Seam-Setup-X.Y.Z-arm64.exe`, `seam-X.Y.Z-windows-x64.zip`, `seam-X.Y.Z-windows-arm64.zip`, `seam.exe`, `seam-settings.exe`, `seam-arm64.exe`, `seam-settings-arm64.exe`, `latest.json` and `latest.json.minisig`, and nothing else.
- [ ] **Signature.** `minisign -Vm latest.json -P <key>` passes on the downloaded pair.
- [ ] **Manifest.** `latest.json` has `files` with the x64 `seam.exe` and `seam-settings.exe` only, and `arches` with `x86_64` and `aarch64`. Every `sha256` matches its asset (`Get-FileHash`), and the `aarch64` entries name `seam-arm64.exe` and `seam-settings-arm64.exe` as `asset`.
- [ ] **Machine type.** `dumpbin /headers` on the ARM64 exes, from the zip and from the installed folder, says `AA64 machine (ARM64)`. The x64 ones say `8664 machine (x64)`.
- [ ] **Zips.** Each zip holds `seam.exe`, `seam-settings.exe`, `README.md` and `LICENSE`, and its exes hash the same as that build's entries.
- [ ] **Old updater.** A 0.4.0 x64 build with **Include pre-releases** on finds the pre-release, downloads the x64 exes, and installs it.
- [ ] **ARM64 setup on x64.** On an x64 PC, `Seam-Setup-X.Y.Z-arm64.exe` refuses to install, saying it needs a different processor.
- [ ] **ARM64 install.** On the ARM64 PC, the ARM64 setup installs and Seam runs. Task Manager's Architecture column shows ARM64 for `seam.exe` and `seam-settings.exe`.
- [ ] **Switch builds.** On the ARM64 PC, install the x64 setup, then run the ARM64 one over it. Installed apps shows one Seam, the folder holds the ARM64 exes, and your settings stay. Going back the other way works the same.
- [ ] **ARM64 updater.** An ARM64 build older than the pre-release downloads `seam-arm64.exe` and `seam-settings-arm64.exe` and stages them as `seam.exe` and `seam-settings.exe` in `updates\<version>\`, and after **Install and restart** the exes are still ARM64.

## Microsoft Store build

Build and install a test package, from an elevated prompt the first time:

```
cargo build --release --workspace
pwsh packaging\pack-msix.ps1 -Sign -Install
```

Quit any GitHub copy first, unless the step says otherwise.

- [ ] **Identity.** About says "Updates come from the Microsoft Store" with **Open in Store**, which opens the Store's updates page. There's no update card and no **Check now**.
- [ ] **Updater off.** `seam-settings.exe --check-updates` and `--install 0.0.0`, run from the package folder, exit at once with nothing downloaded. The tray makes no check after two minutes.
- [ ] **Config survives.** Change a setting, quit Seam, start it again. The setting is kept.
- [ ] **Log and folder.** Turn on **Debug log**. `seam.log` is written. **Open** shows the folder under `%LOCALAPPDATA%\Packages\…\LocalCache\Roaming\Seam`, which holds `config.json` and `seam.log`. The path under the button is that one.
- [ ] **Ease setting.** Turn **Ease cursor movement between displays** off and on in Windows. Displays shows the right state each time.
- [ ] **Align arrangement.** **Align arrangement** changes the Windows arrangement from inside the package, or refuses with the same reason as the GitHub build.
- [ ] **Elevated windows.** Run an elevated window on a screen and move the pointer across a border with the remap on. The pointer crosses and nothing crashes. With zones on, a Shift-drag over the elevated window doesn't move it.
- [ ] **Shortcuts.** Ctrl+Shift+Alt+L and Ctrl+Shift+Alt+P work.
- [ ] **Start with Windows.** The switch turns the startup task on, and Windows Settings, Startup apps lists Seam as on. Sign out and in: Seam is in the tray with no window. Start it from the Start menu: the tray and the window both open.
- [ ] **Turned off in Windows.** Turn Seam off in Windows Settings, Startup apps. General shows "Turned off in Windows Settings, Startup apps" under the switch, and the switch can't be used. Turn it back on there and reopen the window.
- [ ] **Reboot.** Restart the PC with **Start with Windows** on. Seam is in the tray with no window.
- [ ] **Both installed.** With the test package installed, start the GitHub `seam.exe` from a folder. It hands over and exits, and the packaged Seam's window opens. Then quit Seam, start the GitHub copy, and start the packaged one from the Start menu. The GitHub copy quits, the packaged copy runs, and there's one tray icon.
- [ ] **Two-copies note.** While the GitHub copy's Run key or its exe is still there, the packaged window shows "Seam is installed twice. Uninstall the GitHub version to keep the Store one" on every page. Delete the GitHub copy and clear its Run key, and the note goes.
- [ ] **Import.** With a GitHub `config.json` in `%APPDATA%\Seam` and no package config (uninstall the package to clear it), start the packaged Seam. It starts with default settings, and the window shows "Import settings from your earlier Seam". **Import** brings the old settings in. Reopen the window: no offer. If the offer never shows, the package can't read `%APPDATA%\Seam`, which needs a decision.
- [ ] **Uninstall.** Remove the package. Seam leaves the tray, and the GitHub copy still starts.

## Stream Deck

Build the plugin with `pnpm install`, `pnpm build` and `pnpm run pack` in `integrations/streamdeck`. Run these with Seam running and **Let other apps control Seam** on, and the Stream Deck app open.

- [ ] **Install the packed file.** Double-click `dist\co.f3nn3x.seam.streamDeckPlugin`. Stream Deck installs it, and a Seam group with thirteen actions appears in the action list. General in Seam lists "streamdeck" among the connected apps.
- [ ] **Switch workspace.** Drag the action to a key and pick a workspace in the dropdown, which lists the current desk's workspaces. The key shows its name. Press it: the workspace is applied, and the key lights once it's in use. Switch to another workspace by hotkey. The key goes dark.
- [ ] **Drift dot.** With the key's workspace in use, drag one of its windows away. Within a second a small amber dot shows in the key's top-right corner. Press the key: the window goes back and the dot goes. Switch to another workspace: the key goes dark with no dot. Quit Seam while the dot shows: the key says "Seam isn't running" with no dot.
- [ ] **Snap back.** Drag the action to a key. Move a window of the workspace in use and press it: the window goes back. With no workspace in use the key shows an alert.
- [ ] **Renamed workspace.** Rename the workspace in Seam, then switch to it. The key's title shows the new name. Delete it and capture one with the same name. The key still switches to it.
- [ ] **Line up screens.** Press the key. Line Up opens. Press it while Line Up is open. The key shows an alert and Line Up stays as it is.
- [ ] **Pause or resume remap.** The key reads "Remap on" and is lit. Press it. Crossings stop remapping, and the key reads "Remap paused". Press it again to resume. Pause from the tray menu instead. The key follows. With no borders lined up, the key shows paused.
- [ ] **Zones on or off.** The key follows the Zones page. Press it to turn zones off and on. Change the setting on the Zones page. The key follows.
- [ ] **Next and previous zone.** Focus a window and press each key. The window moves one zone forward or back. Press one with the desktop focused. The key shows an alert.
- [ ] **Move window.** Drag the action to a key and pick each direction in its settings. The key's title names the direction. Press it: the focused window moves that way as Shift+Win+Alt+arrows does. With nothing that way the key shows an alert.
- [ ] **Focus window.** Drag the action to a key, pick a direction and press it while an app is focused. The window that way takes focus, even though the press came from the Stream Deck. With no window that way the key shows an alert.
- [ ] **Desk.** The key shows the active desk's name. Press it. The Displays page opens.
- [ ] **Desk change.** Unplug the U3477. The desk key and the workspace keys change to match, and a workspace key from the other desk goes dark. Replug it. They come back.
- [ ] **Seam isn't running.** Quit Seam from the tray. Every Seam key shows "Seam isn't running", and pressing one shows an alert. The workspace dropdown lists "Seam isn't running".
- [ ] **Reconnect.** Start Seam again. Within 30 s every key goes back to its own title and state without touching the Stream Deck app.
- [ ] **Off switch.** Set `api.enabled` to false in `config.json` and reload. The keys show "Other apps are off in Seam". Set it back. They recover.
- [ ] **Plugin log.** In the Stream Deck app's plugin logs for Seam, a failed press logs one line. The plugin keeps running.

## Reconnecting and sleep

- [ ] **Unplug a screen.** Unplug the U3477. The log shows `display change`, then `no saved layout for these screens`. The other screens behave as plain Windows.
- [ ] **Replug.** Replug it. The log shows `resolved 3 of 3 saved link(s)` and crossings work again without opening Line Up.
- [ ] **Sleep and resume.** Sleep the PC and wake it. The log shows `resumed from sleep`, then monitors, then `hook installed`. Crossings work.
- [ ] **Screens off.** Let the screens power off and wake them. Crossings still work.
- [ ] **Unplug with the overlay open.** Open Line Up, unplug the U3477. The log shows `overlay active: true`, every overlay closes, nothing is saved, and the app stays up.

## Scaling

- [ ] **Change scaling while running.** Set screen 1 from 125% to 150%, then back. Pixel rects stay the same, so links stay resolved and crossings land in the same place.
- [ ] **Change scaling with the overlay open.** The overlay stays full-screen on its monitor, or closes cleanly if Windows reports a display change. Note which.

## Shell

- [ ] **Explorer restart.** Run `taskkill /f /im explorer.exe`, then `start explorer`. The tray icon comes back, and crossings work throughout.
- [ ] **Second launch.** Run `seam.exe` again. No second process appears, and the Seam window opens or comes to the front.
- [ ] **Second launch, Line Up.** Run `seam.exe --line-up` while Seam is running. No second process appears, Line Up opens in the first, and the keyboard works immediately (try Tab). Press Esc.
- [ ] **`--line-up`.** Quit Seam, then run `seam.exe --line-up`. It starts with Line Up open.
- [ ] **Launch at login.** Tick it in the tray, sign out and in. Seam starts and crossings work. Untick it, sign out and in. Seam doesn't start.
- [ ] **Pause.** Untick **Remap pointer at borders**. The log shows `hook uninstalled` and the tooltip says paused. Tick it again and crossings work.

## Staying out of the way

- [ ] **Game confining the cursor.** While a game holds the cursor with `ClipCursor`, no `remap` lines appear and the pointer stays in the game.
- [ ] **Borderless fullscreen game.** With nothing confining the cursor, crossings still remap. Note any stutter or focus change. If the game does confine it, expect the case above.
- [ ] **RDP into this PC.** Moves are injected, so no `remap` lines appear for the session.
- [ ] **RDP out of this PC.** While the pointer is in the RDP window on one screen, no remaps fire inside it. Crossing to another screen behaves normally.

## High polling mouse

Set the mouse to its highest rate, up to 8 kHz.

- [ ] **Everyday use.** Circle quickly across all borders for a minute. There's no visible stutter.
- [ ] **Hook time.** Run this and note the maximum. Expected: well under 1000 µs. Earlier runs peaked at 634 µs.

```
powershell -c "(Select-String 'hook ([0-9.]+)us' $env:APPDATA\Seam\seam.log).Matches | % { [double]$_.Groups[1].Value } | Measure-Object -Maximum -Average"
```

- [ ] **Every event.** The log writes a `hook stats` line once a minute, covering all mouse events. `own max` is Seam and should stay well under 1000 µs. `next hooks max` is other apps' mouse hooks, which Seam can't control. `events` shows your polling rate reaching the hook.

## Ease cursor movement off

Turn off **Settings → System → Display → Multiple displays → Ease cursor movement between displays**.

- [ ] **Warning.** Restart Seam with the setting off. A balloon warns once, and the log shows `Ease cursor movement is off`. The settings panel shows it as off.
- [ ] **Describe what happens.** Expected: crossings where both screens have pixels at that row still remap. Elsewhere the pointer stops at the edge, because Seam only acts when a move leaves the screen across a calibrated border and lands on a physical screen. Try the top of screen 1 left of x=596, and the right edge of screen 1 below where screen 4 ends. Record what you see, then turn it back on.

## Align arrangement

Finishing Line Up with the setting on calls `SetDisplayConfig` only if the plan is accepted.

- [ ] **Refusal on this desk.** With **Also align Windows arrangement** ticked, finish Line Up. The balloon says the arrangement is unchanged, the log shows `arrangement result: Err("aligning would make screens overlap")`, and nothing moves. Moving screen 4 up for its border would take screen 3 with it, and screen 3's 7 px offset to the left would collide with screen 1.
- [ ] **Setting off.** Untick it and finish Line Up. The log shows `arrangement skipped: setting is off`.
- [ ] **Undo in Settings.** Only if an arrangement was actually applied: drag the screens back in Display settings, press Apply, and confirm the log shows `display change` and borders resolve again. If a screen no longer touches on its calibrated side, the log shows `dropped` for that border.

## Workspace app icons

Needs workspaces on this desk that hold the apps below.

- [ ] **Win32 apps.** A workspace with a few desktop apps (Notepad, Firefox, Explorer) shows each app's own icon in a 16 px row under the name, in the order of the rules. The "n windows" caption is gone. Hover an icon: the tooltip is the app name the detail table shows.
- [ ] **Store apps.** Add Windows Terminal and Calculator. Each shows its logo, not a blank or generic icon.
- [ ] **Exe gone.** Rename or uninstall an app a workspace holds. Its slot shows a grey rounded square, no error or toast, and the other icons are unaffected.
- [ ] **Many apps.** A workspace with 12 different apps shows eight icons and "+4". Two windows of the same app show one icon.
- [ ] **Light and dark.** Switch Windows between light and dark. The placeholder and the "+n" are readable in both, and icons with transparent corners show no black or white fringe.
- [ ] **150% scaling.** Set display scaling to 150%. Icons are sharp, not blurry, and still 16 logical px.
- [ ] **No stall.** Open the Workspaces page with a workspace of many apps. The page draws at once and the icons fill in a moment later.

## Report back

For each failure, give the step, what you saw, and the matching lines from `seam.log`.
