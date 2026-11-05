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

- [ ] **Menu.** Right-click the tray icon: status line, **Open Seam…**, **Line Up Screens…**, **Remap pointer at borders**, **Quit Seam**.
- [ ] **Left-click, migrated.** With `"tray_click": "line_up"`, left-click opens Line Up. Press Esc.
- [ ] **Left-click, new install.** With `"tray_click": "open"`, left-click opens the Seam window.
- [ ] **Line up shortcut.** Press Ctrl+Shift+Alt+L. Line Up opens. Press Esc.
- [ ] **Pause shortcut.** Press Ctrl+Shift+Alt+P. Crossings stop remapping and the tooltip says paused. Press it again and they work.
- [ ] **Shortcut taken.** Have another app hold Ctrl+Shift+Alt+L, then start Seam. Seam runs, and `state.get` lists it in `hotkey_errors`.
- [ ] **Zone shortcuts.** Win+Alt+Right and Win+Alt+Left are registered by default. If another app holds one, `state.get` lists `next_zone` or `prev_zone` in `hotkey_errors`.
- [ ] **Ease balloon.** With Ease cursor movement off, click the balloon. The Seam window opens.

## Local API

Use a small client that opens `\\.\pipe\seam-<your username>` and writes one JSON line at a time.

- [ ] **Own account only.** Run the client as another Windows user. It's refused.
- [ ] **State.** `{"jsonrpc":"2.0","id":1,"method":"state.get"}` returns the 4 screens and 3 lined-up borders.
- [ ] **Remap.** `remap.set` with `{"on":false}`. Crossings stop remapping, and a subscribed client gets `state.changed`. Turn it back on.
- [ ] **Events.** Subscribe with `events.subscribe`, unplug the U3477. `desk.changed` arrives. Open and cancel Line Up. `lineup.finished` arrives with `saved` false.
- [ ] **Identify.** `screen.identify` with `{"number":4}`. The number shows on the U3477 for 1.5 s.
- [ ] **Off switch.** Set `api.enabled` to false in `config.json` and reload. Calls from your client get error 5.
- [ ] **Bad input.** Send a line that isn't JSON, an unknown method, and a line over 1 MB. You get errors, or the connection closes, and Seam keeps running.
- [ ] **Slow client.** Subscribe and stop reading while a second client toggles the remap for 30 s. Events go out at most 10 times a second, so the first client is dropped after about 26 s, and crossings stay smooth.
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
- [ ] **Next and previous zone.** Press Win+Alt+Right with a window focused. An unsnapped window snaps to the zone holding its centre. Press again to step through the zones, and past the last one it lands in the first zone of the next screen. Win+Alt+Left goes back, wrapping the other way.
- [ ] **Zone hotkeys in the panel.** On Shortcuts, Zones shows both shortcuts. Record new keys and they work from another app. Record one another app holds and the row says so in red.
- [ ] **API.** `zone.move` with `{"dir":"next"}` moves the focused window as the hotkey does. With zones off or no layout set it answers error 3 with the reason.
- [ ] **Excluded app.** Add an exe name such as `notepad.exe` to `zones.exclude` in `config.json` and reload. Dragging it with Shift held shows no overlay.
- [ ] **Elevated window.** Run Task Manager as administrator and Shift-drag it onto a zone. It doesn't move, and you get one balloon: "Seam can't move windows from apps running as administrator." A second try in the same session shows no second balloon.
- [ ] **Game with ClipCursor.** With zones on, run a game that confines the pointer. No overlay appears and the game isn't disturbed.
- [ ] **Windows snap flyout.** Dragging a window to the top edge still shows Windows' own layouts. Seam leaves that setting alone.
- [ ] **Hook timing.** Note `hook stats`, then drag windows with Shift held for 10 s. `own max` and `avg` match a run with nothing dragged, and the pointer stays smooth.
- [ ] **Overlay clean-up.** After every drag no overlay stays on any screen, including when you press Esc mid-drag or close the dragged window.

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
- [ ] **Rollback.** Make `seam-settings.exe` read-only and locked by another program, then install. Seam is back on the old version, the error is shown, and the tray is running.
- [ ] **On their own.** Turn on **Install updates on their own**. While you use the PC nothing happens. Leave it alone for two minutes with an update ready, and Seam restarts on the new version with the balloon. With Line Up open, it waits.
- [ ] **No network.** Turn Wi-Fi off and press **Check now**. About shows one line saying what happened, with **Try again**, and the remap keeps working.
- [ ] **Hook timing.** Compare `hook stats` across a check and an install with a run without one. Own max stays well under 1000 µs.

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

## Report back

For each failure, give the step, what you saw, and the matching lines from `seam.log`.
