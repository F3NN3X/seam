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

## Tile this screen's windows

Set a key for **Tile this screen's windows** on Shortcuts (it has no key until you set one). Start with screen 1 set to None and the other windows closed.

- [ ] **Two windows, no layout.** Open Notepad and Calculator on screen 1 and press the key with the pointer on screen 1. They go side by side in two columns, the focused one in the first. A notification says "Tiled 2 windows on screen 1" and offers Undo. The Zones page shows Changed for screen 1.
- [ ] **One window.** Close one: the key shows "Nothing to tile" and no layout is saved.
- [ ] **Three windows.** With a third window and screen 1 set to None again, the key picks Main + stack: the focused window takes the large zone on the left, the others the stacked zones.
- [ ] **Five windows.** With five windows and None, the key picks 3x2, and with ten windows nine are tiled and the notification says one was left where it was.
- [ ] **Existing layout.** Pick Grid for screen 1. Snap one window into the top-left zone, then open three more and press the key. The snapped window stays, the others take the free zones nearest to them, and the layout is still Grid. With more windows than zones, the notification says how many were left.
- [ ] **Left alone.** Pin one window on top, make another sticky, minimize a third, and run Task Manager as administrator. None of them moves. A window on another screen doesn't move either.
- [ ] **Undo.** Click the notification within 8 seconds: every window goes back to where it was, and a layout the tile picked is gone from the Zones page. After 8 seconds a click does nothing, and the log says it was too late.
- [ ] **Tray, one screen.** With one screen connected, the tray menu has **Tile windows** and it tiles that screen.
- [ ] **Tray, two screens.** With two screens, **Tile windows** is a submenu with Screen 1 and Screen 2, numbered as in Line Up. Each tiles its own screen whatever screen the pointer was on.
- [ ] **API.** `screen.tile` with no params tiles the screen under the pointer and answers `{ok, tiled, left}`. With `{"number":2}` it tiles screen 2, and `{"number":9}` answers error 1. With zones off it answers error 3.
- [ ] **Other window manager.** While komorebi runs, the key does nothing and the notification says why.
- [ ] **Stream Deck.** The **Tile screen** action does what the key does.

## Quick layout

Set a key for **Quick layout** on Shortcuts (it has no key until you set one). Make one custom layout for screen 1 only and one for any screen.

- [ ] **Open on each screen.** Put the pointer on each screen in turn and press the key. A panel opens in the middle of that screen, titled with its number, and takes focus. It shows the built-in layouts first, then the custom ones, then None, numbered from 1. The layout the screen has is marked with an amber ring and says "In use", and no thumbnail is tinted amber.
- [ ] **Keys 1 to 9.** Open windows on the screen and press 2. The screen takes the second layout and its windows move into the zones, one to a zone. The panel stays open, the mark moves, and no Undo notification appears. Press 4 to try another.
- [ ] **Arrows and Enter.** The arrows move a white ring over the thumbnails, left and right by one and up and down by a row. Enter picks the one it is on.
- [ ] **Ctrl+Z.** After two picks, Ctrl+Z puts the screen back to the first pick's layout, and again to the one it had when the panel opened. With nothing left to undo it does nothing.
- [ ] **Esc and click.** Esc closes the panel. So does a click outside it, which also goes through to what was under it. A click on a thumbnail picks it and the panel stays open.
- [ ] **More than nine.** With more than nine layouts on the screen, the arrows scroll the grid a row at a time, the numbers follow what is shown, and the header says which ones are shown.
- [ ] **Used on.** A custom layout tied to screen 1 shows on screen 1's panel but not on screen 2's, unless screen 2 is using it. One for any screen shows on both.
- [ ] **Zones off.** Turn zones off and open the panel. It says zones are off. A pick is saved, and the Zones page shows it, but no window moves.
- [ ] **Other window manager.** While komorebi runs, the panel opens with a line saying Seam leaves the windows to it, and a pick only saves.
- [ ] **API.** `zones.quick` with no params opens it on the screen under the pointer and takes focus. `{"number":9}` answers error 1.
- [ ] **Stream Deck.** The **Quick layout** action opens the panel.

## Tray and shortcuts

- [ ] **Menu.** Right-click the tray icon: status line, **Open Seam…**, **Line Up Screens…**, the **Workspaces** menu when there are workspaces, **Tile windows**, **Remap pointer at borders**, **Release pinned windows**, **Quit Seam**.
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

- [ ] **Open.** Choose **Line Up Screens…** in the tray menu. Every screen shows its number. Border 1 of 3 is active, and the keyboard works straight away.
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

## Glue

Two steps move a screen in Display settings. Ask before moving a screen, and note where it was so it can go back.

- [ ] **Make a gap (ask before moving a screen).** In Display settings, drag screen 4 about 100 px right of screen 1 and press Apply. The log shows `display change`, and the 1 to 4 border is either kept or dropped with `dropped`.
- [ ] **Line Up offers it.** Open Line Up. A pair found only from the pixel layout starts off, as under **G turns it off**, and a pair the screens' sizes put side by side starts glued. After the touching borders comes "Glue screen 4 to screen 1". Both facing edges are dashed amber with "Gap in Windows, 100 px" beside them, and the lines are bright, because the screens' sizes put them side by side. The step says which way to move each line, such as "straight below" or "level with".
- [ ] **G turns it off.** Press G. Screen 4 reads "Glue screen 4 to screen 1?" with "Press G, then line them up as they stand on your desk" and "Enter: skip", and screen 1 reads "Screen 4, to the right, may touch this one on your desk. Look at screen 4 to glue them". The lines and edges dim. Dragging, arrows, Tab and R do nothing, and the pointer is a plain arrow on screen 4. Press G again and it's back as it was, with "G unglues them" under the tips.
- [ ] **Crossing the gap.** While it's glued, move right off screen 1 at the height of the lines. The pointer rests at the edge, then enters screen 4 where the lines meet once you've pushed about 60 px (the **Glued edge push** default). Move left off screen 4 and it comes back the same way. A quick flick crosses at once. The log shows `remap` lines for `border 0->3`.
- [ ] **Resting resets.** Push a little way past the edge, move back onto screen 1 or slide along the edge, then push again. It takes the full push again to cross.
- [ ] **Auto-hide taskbar on a glued edge.** On my desk, with the 4K to panel glue saved and the 4K's taskbar set to hide on its bottom edge, push slowly down off the 4K. The taskbar slides up and the pointer stays on it. Keep pushing and the pointer crosses to the panel. Pull back up before that and nothing crosses.
- [ ] **Off means Windows.** Turn it off with G and cross again. The pointer stops at the edge of screen 1, as it does without Seam.
- [ ] **Saving.** Glue it, finish Line Up. In `config.json` the 1 to 4 border has `"glued": true`, and no other border has a `glued` key. Restart Seam: the log shows the link resolved, and crossing still works.
- [ ] **Displays.** The border row reads "Glued to the right edge of 1" next to "Lined up today". On the map, screen 4 sits against screen 1, not off to the side.
- [ ] **Align.** With **Also align Windows arrangement** ticked, finish Line Up. On this desk the plan is still refused for screen 3, and the gap is untouched. If a move would take screen 4 out from right of screen 1, the balloon names it: "Moving screen N would break its glue with screen M".
- [ ] **Skipping.** Open Line Up, press G to turn the glue off, and finish. The 1 to 4 border is gone from `config.json`.
- [ ] **Close the gap (ask before moving a screen).** Glue it again and finish. Then drag screen 4 back against screen 1 in Display settings. The border still resolves, crossings land where the lines meet, and after another Line Up the border keeps `"glued": true`.
- [ ] **Put it back.** Restore the arrangement you noted, and check the original three borders resolve.

## Line Up aids

On my desk the HDMI panel is screen 3, the U3477 is screen 4 and the PG32UCDM is screen 1.

- [ ] **Named steps.** Open Line Up. The step titles read like "Line up screen 3 (HDMI) with screen 4 (U3477)" and "Glue screen 3 (HDMI) to screen 1 (PG32UCDM)", and the reference screen's text names both screens the same way.
- [ ] **Stripes.** On both screens of a step a band of faint diagonal stripes, about 6 cm deep, runs along the shared edge. Line the two screens up well: the stripes run on straight across the bezel. Drag both lines by 20 px on screen B: they step by about that much. Drag only one line: the spacing on B changes. They follow the lines as you drag, and the blue and pink lines stay the clear ones.
- [ ] **Gap.** On the glue step the stripes carry on across the gap when the lines are right.
- [ ] **Dimmed steps.** With N on, or an unglued gap, no stripes show.
- [ ] **No size.** A screen with no EDID size, if you have one: the step text says "No stripe check: a screen on this pair reports no size", and the band is missing on both screens.
- [ ] **M off.** With **Type measurements in Line Up** off, nothing mentions M and pressing it does nothing.
- [ ] **M on.** Turn **Type measurements in Line Up** on and open Line Up. Screen B's text adds "M: type the offset in cm". Press M: "Screen 4's top edge is [ _ ] cm below screen 3's top edge". Tab flips below and above. Type "1,5" or "1.5": one separator only, no letters. Backspace deletes a digit. Esc closes the field and leaves the lines alone, and Enter and Backspace still mean next and back with the field closed.
- [ ] **M applies.** Measure the offset between two screens' top edges with a ruler, type it and press Enter. The lines move, the stripes run on straight across, and dragging afterwards still works. A negative offset (above) moves B's lines the other way.
- [ ] **M and no size.** With a screen that reports no size, B's text says typing isn't offered and M does nothing.

## Not on your desk

On my desk the panel (screen 3) touches the U3477 (screen 4) in Windows, 18 px below the 4K, but sits under the 4K on the desk, glued to it.

- [ ] **Line Up offers N.** Open Line Up and go to the screen 4 to screen 3 border. Under the tips, screen 3 reads "Screen 3 doesn't touch screen 4 on your desk? Press N, and Seam stops the pointer crossing here".
- [ ] **N turns it on.** Press N. Screen 3 reads "Screen 3 doesn't touch screen 4 on your desk", and screen 4 says Seam stops the pointer crossing here. The lines and edges dim, and dragging, arrows, Tab and R do nothing. Press N again and it's back as it was.
- [ ] **Saving.** Press N and finish. In `config.json` the 4 to 3 border has `"not_on_desk": true`, and no other border has that key. Open Line Up again: the step starts with N on.
- [ ] **Up off the panel.** Push slowly up off the panel where the 4K is above it on the desk. The pointer rests at the edge, never lands on the U3477, and after about 60 px enters the 4K where the glue lines meet. A quick flick crosses at once.
- [ ] **Blocked where no glue maps.** Push up off the right part of the panel, past where the 4K ends on the desk. The pointer stays at the edge and never reaches the U3477. Push down off the bottom of the U3477 over the panel: it stays on the U3477.
- [ ] **Off means Windows.** Press N to turn it off and finish. Going up off the panel under the U3477 lands on the U3477 again, as Windows has it.
- [ ] **Displays.** With it on, the 4 to 3 row reads "Not on your desk" and "The pointer stops at this edge". On the map the panel sits under the 4K, not under the U3477.
- [ ] **Align.** With it on, Align arrangement doesn't move the panel to match the 4 to 3 border.
- [ ] **Games and remote input.** In a game that confines the pointer, or over Remote Desktop, the pointer moves between screen 4 and the panel as Windows has it.

## Edge resistance

Both rows are in the Pointer card on Displays.

- [ ] **Off is as before.** With **Edge resistance** at 0 (Off), cross every border slowly and quickly. Lined-up borders remap as before, borders that aren't lined up cross where Windows puts the pointer, and the log shows no `hold` lines except at glued edges and borders not on your desk.
- [ ] **Grab an edge at a border.** Set **Edge resistance** to 10. Put a window's right edge on the right edge of screen 1, next to screen 4. Move slowly onto that edge: the pointer stops on the last pixel of screen 1, the resize arrow shows, and dragging resizes the window. The log shows `hold` lines while it waits.
- [ ] **Then it crosses.** Keep pushing: after about 10 px the pointer crosses, where the lines meet on a lined-up border and where Windows puts it on one that isn't. Pushing, moving back and pushing again takes the full 10 px again. A quick flick crosses at once.
- [ ] **Corners.** Do the same at a window's corner on a border with a screen above or below.
- [ ] **Drags aren't slowed.** With **Edge resistance** at 100, drag a window by its title bar from screen 1 to screen 4, and drag a file between Explorer windows on two screens. Neither stops at the border. Right-, middle- and side-button drags cross at once too.
- [ ] **Glued edge push.** On my desk, with the 4K's taskbar set to hide on its bottom edge and the 4K to panel glue saved, set **Glued edge push** to 0, 60 and 150 in turn and push slowly down off the 4K. At 0 the pointer crosses at once and the taskbar may not show. At 60 and 150 the taskbar slides up while you push and the pointer crosses after about that many pixels. With **Edge resistance** at 100 and the push at 60, it takes about 100.
- [ ] **Saving.** The sliders send their value when you let go. `config.json` shows `"pointer": { "edge_resistance_px": 10, "glue_push_px": 60 }`, and both rows show their value under the title.
- [ ] **Dead edge.** Where a shorter screen sits beside a taller one, push slowly down off the shorter one's bottom near the taller one. With Ease cursor movement on, the pointer waits at the edge, then jumps to the taller screen after the push.
- [ ] **Line Up.** With **Edge resistance** at 100, open Line Up and cross the borders while you line them up: nothing holds the pointer except glued edges. After Line Up the resistance is back.
- [ ] **Paused.** With **Remap pointer at borders** paused and **Edge resistance** above 0, the pointer crosses every border at once, as in Windows.
- [ ] **Search.** Search for "grab", "corner" and "taskbar" in the Seam window: they find these two rows.

## Zones

Set up first: on Zones, pick **Priority** for screen 1 and **Grid** for screen 2, with **Shift** to show and **Ctrl** to span.

- [ ] **Page.** Zones is in the sidebar with no preview note. The header says "Hold Shift while you drag a window to drop it into a zone", the tabs list the 4 screens, and the preview shows each zone's number and width in px. The chosen tile has an amber ring.
- [ ] **Layouts.** Pick each of None, Columns, Rows, Grid, Priority, Main + stack and Stack + main. The tiles sit in two columns, and Main + stack has one zone on the left taking two thirds with two stacked zones beside it, Stack + main the other way round. The preview changes, and `config.json` holds the layout under `desks[...].zones` for that screen's device path. **None** removes the entry. **Custom** opens the editor.
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
- [ ] **Elevated window.** Run Task Manager as administrator and Shift-drag it onto a zone. It doesn't move, and you get one balloon that names Task Manager. A second try in the same session shows no second balloon.
- [ ] **Elevated Notepad names itself.** Run Notepad as administrator and Shift-drag it onto a zone. The balloon says "Notepad runs as administrator, so Windows won't let Seam move it. Click for how to manage it". Try again: no second balloon for Notepad. Run Task Manager as administrator and do the same: it gets its own balloon. Click a balloon: the Seam window opens on Help. `window.move` on the elevated Notepad answers error 3 with "Notepad runs as administrator".
- [ ] **Game with ClipCursor.** With zones on, run a game that confines the pointer. No overlay appears and the game isn't disturbed.
- [ ] **Windows snap flyout.** Dragging a window to the top edge still shows Windows' own layouts. Seam leaves that setting alone.
- [ ] **Hook timing.** Note `hook stats`, then drag windows with Shift held for 10 s. `own max` and `avg` match a run with nothing dragged, and the pointer stays smooth.
- [ ] **Overlay timing.** With `--log`, drag windows with Shift held across every screen of the test desk. `zone overlay: main` lines are the tray's time per change (expected well under 2000 us). `zone overlay: worker` lines give the overlay thread's own time: drawing a base once per screen after start-up or a screen change, `worker show` per change. The first drag after start-up shows zones on a screen only once its base is drawn, and later drags find them ready. Compare `hook stats` `own max` in the same minute with and without drags.
- [ ] **Changing screens.** Change a screen's resolution, then drag with Shift held. The zones match the new size, with no leftover overlay at the old one. Turn Zones off, and Task Manager's memory for `seam.exe` drops back.
- [ ] **Overlay clean-up.** After every drag no overlay stays on any screen, including when you press Esc mid-drag or close the dragged window.

### Where apps open

Turn the debug log on. **Where apps open** is the last card on Zones, with the caption "Seam puts these apps in place whenever they open, outside workspaces too".

- [ ] **Add a rule.** **Add an app**, pick Notepad from **Open now**, and it gets a row with its icon and a **Goes to** choice. Set it to screen 1, zone 2. `app_rules` in `config.json` holds one entry, and Undo takes the change back. Picking Notepad again says "Already has a place" and adds nothing.
- [ ] **Opens in place.** With **Put apps in place when they open** off and no workspace in use, close Notepad and open it: it goes to zone 2 of screen 1 once, and the log has `app rule notepad: placed`. Drag it elsewhere: it stays there.
- [ ] **The workspace owns the app.** Switch to a workspace that has a row for Notepad (placed on another screen), close Notepad and open it, with **Put apps in place when they open** on and then off: the rule never moves it. Open a second Notepad window after the row is taken: the rule still doesn't move it.
- [ ] **Left alone.** Add Notepad to **Left alone** (**Never include this app** on a workspace row) and open it: it opens where Windows puts it, though the rule is still listed. **Remove** under **Left alone** brings the rule back into effect.
- [ ] **Screen unplugged.** Make the rule's screen go away (unplug it or turn it off) and open Notepad: nothing moves, and there's no `app rule` line.
- [ ] **A dialog isn't moved.** Press Ctrl+O in Notepad: the Open dialog stays where Windows puts it, while a new Notepad window still goes to the rule's zone.
- [ ] **During a switch.** Start a workspace switch and open Notepad by hand while it runs: the switch owns it, and the rule doesn't move it afterwards.
- [ ] **Full screen and Remove.** Change **Goes to** to "Screen 1, full": the next Notepad opens maximized on it. **...**, **Remove** deletes the rule with Undo, and the next Notepad opens untouched.
- [ ] **Search and Help.** Searching "where apps open", "app rules" or "always open" finds the card, and Help has a **Where apps open** topic.

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
- [ ] **Grow and shrink.** Record keys for grow and shrink on Shortcuts. Pick Columns on screen 1 and snap Notepad into the middle zone and a browser into the right one. Press grow on Notepad: the divider towards the bigger neighbour moves by about 5% of the screen, the browser follows, and the left zone doesn't move. Press shrink: the divider moves back. The Zones page now shows Custom. Restart the tray and press grow again: the windows are still found from their stamps.
- [ ] **Two thirds.** Record a key for **Give window two thirds of its group**. With the same windows, press it on Notepad: Notepad gets about two thirds of the width and the others share the rest. Press it again: the zones and windows are back as they were, and the Zones page shows Columns again. Press it, then grow, then it again: this time it gives two thirds again instead of putting the zones back.
- [ ] **Minimum size.** Snap Steam (about 1263×750 at 100%) into the right zone of Columns and Notepad in the middle. Press grow on the left window until it stops: Steam is never squeezed below its minimum, the middle divider is pushed along as Notepad reaches its own minimum, and at the end the key does nothing and the log says there's no room. `window.resize` with `{"by":"grow","step":2000}` stops at the same place.
- [ ] **Neighbours follow an edge.** Pick Columns on screen 1 and snap a window into each of the three zones. Drag the middle window's right border to the right and back: the right window follows live with no gap or overlap, and the left one doesn't move. Let go: the middle window fits its zone exactly, the Zones page shows Custom, and `config.json` holds the new tree once. Drag the right border again: all three still count as snapped and follow.
- [ ] **Edge cascade.** Snap Steam into the right zone of Columns and Notepad in the middle. Drag the left window's right border far to the right: Notepad shrinks to its minimum, then the middle divider is pushed and Steam keeps its own minimum. Past that the neighbours stop, and when you let go the left window comes back to its zone, with no overlap.
- [ ] **Esc on an edge drag.** Start dragging the middle window's border, move it, then press Esc before letting go. Windows puts the window back, the neighbours go back to where they were, and `config.json` doesn't change.
- [ ] **Resize neighbours off.** Turn off **Resize neighbours with a window** on Zones and drag the middle window's border: only that window changes size, as before, and it's no longer snapped. Turn it back on.
- [ ] **Corner drag in Grid.** Pick Grid and snap four windows. Drag the top-left window's bottom-right corner: the window to its right, the one below and the one diagonally across all follow, and the two dividers end where you let go.
- [ ] **Slow app follows.** Snap VS Code, Discord or another Electron app beside Notepad and drag Notepad's border quickly: the app follows, perhaps lagging a little, and lands exactly on its zone when you let go. With `--log`, the `zone edges:` line gives the longest tick on the tray's main thread (expected well under 1000 us).
- [ ] **Screen edge.** Drag the outer edge of a window in the left zone (the side along the screen's edge): nothing else moves, and the window is resized by hand as before.
- [ ] **Group refusals.** Press grow on a window in no zone: nothing moves and the log says it isn't in a zone. Put Task Manager running as administrator in the zone beside: grow on its neighbour moves nothing and the log says it runs as administrator. `window.resize` and `window.priority` answer error 3 with the same reasons, and with zones off.
- [ ] **Left alone.** Run Task Manager as administrator and press Shift+Win+Alt+Right on it. Nothing moves and the log says it runs as administrator. With the Seam window focused, nothing moves either. Swap towards Task Manager doesn't move it or the other window.
- [ ] **Hung window.** Hang a test app (a debugger break, or a test app that sleeps on its UI thread) and press Shift+Win+Alt+Right on it. Nothing moves and the log says it isn't responding. Focus and swap skip it.
- [ ] **Rapid presses.** Hold Shift+Win+Alt and tap Right five times quickly. The window ends up where the last press sends it from the place it had reached, with no window left half-moved.
- [ ] **Panel.** On Shortcuts, the Windows section lists move, focus and swap in each direction with their keys. Record a new key and it works from another app. Search "swap" or "focus" and the rows are found.
- [ ] **API.** `window.move`, `window.focus` and `window.swap` with `{"dir":"left"}` act on the focused window and answer `{"ok":true}`. With nothing that way, on an administrator window, or on a hung one they answer error 3 with the reason. `{"dir":"next"}` answers error -32602.
- [ ] **API focus.** From a test client run from a terminal, send `window.focus` with `{"dir":"right"}` while another app is focused. The window to the right takes focus, its taskbar button doesn't just flash, and the pointer doesn't move.
- [ ] **Focus on this window.** Record a key for it. With four windows on screen 1 and one on screen 2, press it on one of screen 1: the other three minimize and screen 2 is untouched. Press it again: they come back in their old stacking order, and the window you pressed on has focus. Restore one by hand, press again: only the others come back. Close all the minimized ones, press again: it minimizes again. A window pinned on top and a sticky one stay. An app on the zones' exclude list, an administrator app and a hung app stay.
- [ ] **Focus on this window, workspace.** With a workspace in use, `window.focus_only` with `{"scope":"workspace"}` minimizes every window on every screen that isn't one of its apps or another window of those apps; calling it again restores them. With no workspace in use it answers error 3. With a window manager like komorebi running it answers error 3 as well, and the key does nothing.
- [ ] **Spotlight.** Record a key for it. Press it: every screen dims to about half, except the focused window, which stays bright. Click another window, and it becomes the bright one at once. Click on a window on screen 2: screen 1 stays dimmed above all its windows, screen 2 dims below the window. Minimize and restore windows, and Alt+Tab: the dim keeps following. Click through the dim: clicks reach the windows and nothing takes focus. Click the desktop or the taskbar: the dim hides, and comes back with the next window. Move the focused window to another screen by key: the dim follows within a moment. Unplug a screen or change resolution: the dim fits the screens. Press the key again, or quit Seam: the dim is gone. A topmost window (a pinned one) stays bright above the dim. With komorebi running it still works.
- [ ] **Focus and spotlight API and Stream Deck.** `window.spotlight` with `{}` toggles and answers `{"on":true}`, then `{"on":false}`; `state.get` shows `spotlight`. A Stream Deck "Spotlight" key follows it and "Focus on this window" minimizes and restores.
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
- [ ] **Border swatches.** On the Zones page, Windows card, each row has eight colour swatches between the None and Custom colour ones. Pick each colour in the pin row with a pinned window on screen: the border changes at once. Do the same in the focus row with the focus border on: the focused window's border changes at once, and the pinned one doesn't. Pick **Windows accent** and change the accent in Settings: both follow. Turn the focus border off: the focus row dims, its swatches don't respond and the tooltip says why. Tab to a swatch: it shows a ring, and Enter and Space pick it. The chosen swatch always has a ring. Hover each swatch: a tooltip names it (Windows accent, Seam amber, Blue, Teal, Green, Red, Pink, White). Set `windows.pin_color` to `"nope"` in `config.json`: the pin row shows amber and pins are amber. Search for "pin border" and "focus border colour": both rows are found. `state.get` shows `wm.focus_color` and `wm.pin_color`.
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

- [ ] **Open.** On Zones, press **Edit layout** under the preview. The preview becomes the editor, with a toolbar under it: Split side by side, Split top and bottom, Merge with neighbour, Undo, Reset, Done. Press the **New layout** tile: it opens the editor too, on a single zone.
- [ ] **No layout yet.** On a screen set to None, open the editor. It starts as one zone covering the screen, and splitting it works.
- [ ] **Select.** Click a zone: it gets an amber outline and a faint amber fill. Tab to the preview, then use the arrow keys to move the selection from zone to zone. Tab again reaches every toolbar button, in order.
- [ ] **Split.** Select a zone and press **Split side by side**: it becomes two zones, with the old number kept on the first. Do the same with **Split top and bottom**. The zones renumber left subtree first.
- [ ] **Too small or too deep.** Keep splitting one zone. When its halves would be under 120 px the split buttons turn grey and a tooltip says the zone is too small to split further. Nothing crashes at any depth, and at 128 zones the tooltip says a layout can have 128 zones at most.
- [ ] **Drag a divider.** Drag a divider. The zones follow the pointer, a tag next to it shows the two widths in px, and the divider pulls to 1/4, 1/3, 1/2, 2/3 and 3/4 when it comes within 2% of them. It never goes below 10% or above 90%.
- [ ] **Merge.** Select one of two sibling zones and press **Merge with neighbour**: they become one. Select a zone whose neighbour is split into more zones: the button is grey and its tooltip says merging needs the zone next to it to be a single zone.
- [ ] **Undo.** Press Ctrl+Z after a split, a merge, a Reset and a divider drag. Each undoes exactly one step, and a drag is one step. The Undo button's tooltip names the last step, for example "Undo split (Ctrl+Z)". With focus anywhere on the page outside the editor, Ctrl+Z does nothing.
- [ ] **Reset.** Press **Reset**. The tree goes back to the template the layout started from, or to how it was when the editor opened for a custom one. It can be undone.
- [ ] **Done.** On a screen set to Columns, edit and press **Done**. The editor closes, a new "Custom N" tile is highlighted, the preview shows the tree, and `config.json` holds the tree in `custom_layouts` and `"template": "custom"`, its `custom` id and the tree for that screen. Shift-drag a window: it snaps to the new zones, with no restart.
- [ ] **Esc.** Edit, then press Esc with focus on a toolbar button, then again with focus on the preview. Both times the edits are thrown away with no dialog and `config.json` is unchanged.
- [ ] **Leaving with edits.** Edit, then press another page in the sidebar, another screen tab or another layout tile. One dialog asks "Discard layout changes?" with Discard and Keep editing. Keep editing leaves everything as it was, Discard goes where you were headed. Without any edits there is no dialog.
- [ ] **Only Done writes.** Watch `config.json` while editing. It doesn't change until Done.
- [ ] **Every screen.** Edit the layout of the portrait U3477 and the 1920x550 panel, which are the tricky shapes. Dividers, px widths and the saved layout follow the real screen size.
- [ ] **Layout changed while open.** Open the editor on screen 1 and make no edits. Drag the border of a window snapped on screen 1: when you let go the editor shows the new dividers, and an edit then Done builds on them. With an edit made first, the editor keeps your edits and Done saves them over the drag's layout.
- [ ] **Screens change.** With the editor open, unplug a screen. The editor closes without saving.

## Custom layouts

Start with no `custom_layouts` in `config.json`, and screens 1 and 4 set to Columns.

- [ ] **Old Custom layouts.** With a config from before this change where a screen has `"template": "custom"`, open Zones on that screen. The tiles are the templates, a selected **Changed** tile showing its zones, and **New layout**. **Save as layout** is beside **Save from windows**, and `custom_layouts` is still empty.
- [ ] **Create two.** Press **New layout**, split the zone side by side, press **Done**. A "Custom 1" tile appears, selected, and screen 1 uses it. Do the same with a top and bottom split: "Custom 2". `custom_layouts` has both.
- [ ] **Edit layout on a template.** On screen 4 with Columns, press **Edit layout**, drag a divider and press **Done**. A new "Custom 3" tile appears and screen 4 uses it. Pick Columns again: it is still the plain three columns.
- [ ] **Apply each to two screens.** Use **Used on...** to set Custom 1 to any screen and Custom 2 to any screen. Click Custom 1 on screen 1 and on screen 4, then Custom 2 on screen 2 and screen 3. Each screen's tile is highlighted, and Shift-dragging a window snaps to that layout on every one of them.
- [ ] **Picking moves the screen's windows.** On screen 1 with Grid, snap two terminal windows into the two left zones and leave Notepad loose on the right. Pick Columns: the two terminals each get a zone of their own (one goes to the first column, the other to the free column nearest it), Notepad takes the remaining column, no two windows share a zone, and a notification "Screen 1 has a new layout" says 3 windows moved. Click it within 8 seconds: Grid is back on the Zones page, both terminals are in their old zones and Notepad is back where it was. After 8 seconds a click does nothing.
- [ ] **All snapped, crowded.** On screen 2 with Grid, snap the two terminals into the top-left and bottom-left zones and Notepad into the top-right zone. Pick Columns, then Rows, then Main + stack: each time the log says "3 moved" (or 2 moved and 1 left for Rows), and no two windows ever end up in the same zone.
- [ ] **More windows than zones.** With four windows on screen 1, pick Rows: two windows get the two zones, snapped ones first, the other two stay where they are, and the notification says 2 left where they were.
- [ ] **Picking with zones off.** Turn zones off and pick a layout: it is saved, nothing moves and no notification shows. Pick None with zones on: the zones go and no notification shows, because no window moved. Pick a layout on a screen with no windows on it: no notification either.
- [ ] **Picking the same layout.** Click the tile that is already selected: nothing moves and no notification shows.
- [ ] **Edit one.** Open the **...** of Custom 1 and choose **Edit**. Split a zone and press **Done**. Screens 1 and 4 both get the new zones, and their entries in `config.json` have the new tree with `custom` still set. Esc instead of Done changes nothing.
- [ ] **Rename and duplicate.** **Rename** Custom 2 to "Desk work": the tile and the highlight on screens 2 and 3 follow. **Duplicate** it: "Desk work copy" appears with the same zones, and no screen uses it.
- [ ] **Delete one.** **Delete** "Desk work". The dialog says "Delete Desk work?" and "Screens using it keep their zones". Delete: the tile goes, screens 2 and 3 keep their zones and show **Changed**, and their entries have no `custom`.
- [ ] **Changed after a divider drag.** On screen 1 with Custom 1, drag the edge of a snapped window so its neighbour follows. The page shows **Changed** for screen 1, selected, with **Save as layout**, and screen 4 still has Custom 1's zones. **Save as layout** adds a new tile, which screen 1 then uses.
- [ ] **Edit layout on Changed.** On a Changed screen, **Edit layout** and **Done** change that screen alone: no new tile, and still Changed.
- [ ] **Many layouts.** Add layouts until there are a dozen. Under **Your layouts** the tiles wrap across the full width and push the page down, ending with **New layout**, and the preview above stays full width.
- [ ] **Save from windows, two columns.** On screen 1, put Notepad on the left half and Calculator on the right half with Win+Left and Win+Right. Press **Save from windows**. A new tile appears with two side-by-side zones split at the half, screen 1 uses it, and both windows snap into their zones. The API's `zones.from_windows` with `{"number":1}` does the same and answers `{ok, id}` with the new layout's id.
- [ ] **Save from windows, main and stack.** One window on the left two thirds, two stacked on the right third: the new layout is a main zone of two thirds and two stacked zones.
- [ ] **Save from windows, unsplittable.** Arrange four windows in a pinwheel around the middle of the screen, or let two overlap. **Save from windows** says "These windows don't line up into zones", and nothing changes.
- [ ] **Older Seam.** Open the config in an older build: screens with a library layout read as Custom with the same zones.

## Zones page arrangement and Used on

- [ ] **At 1160x780.** Open Zones at the default size. Screen tabs, then the preview across the full width, its caption with **Save from windows** and **Edit layout** on the right, then **Built in** with the seven templates in one row, then **Your layouts**. The preview is no taller than about 320 px, and zone numbers and px labels still show.
- [ ] **At the minimum size.** Shrink the window to 900x640. Nothing overlaps or is cut off: the preview narrows, the caption wraps beside the buttons, and the built-in tiles may wrap to a second row. Long names such as "Main + stack" are not cut short.
- [ ] **No layouts of my own.** With `custom_layouts` empty, **Your layouts** has only **New layout** and "Make your own with New layout, or Save from windows". No **Show all screens** link.
- [ ] **Keyboard.** Tab moves through the built-in tiles, then Your layouts, with a focus ring. Enter or Space picks one. The **...** on a tile opens its menu without picking the tile.
- [ ] **New layouts are for their screen.** On screen 1, **New layout**, split, **Done**. Its entry in `config.json` has `"screens"` with screen 1's device path. Screen 2's tab doesn't show it. Do the same with **Save from windows**, **Save as layout** and **Edit layout** on a template: each entry lists the screen it was made on.
- [ ] **Used on with two screens.** Open the **...** of that layout, **Used on...**. The dialog lists "Any screen" and a checkbox per screen, named as the tabs are, with screen 1 ticked. Tick screen 2 and **Save**: screens 1 and 2 show it, and 3 and 4 don't. Open it again and tick **Any screen**, **Save**: every tab shows it and the entry has no `screens`. **Cancel** changes nothing.
- [ ] **Not connected.** Tie a layout to screens 1 and 4, unplug screen 4, set it to screens 1 and 2 with **Used on...**. Plug screen 4 back in: the layout still shows on screen 4, because its device path was kept.
- [ ] **Show all screens.** On screen 2, press **Show all screens**: every layout shows, and those not for screen 2 say "For screen 1" under their name. The link now reads "Only screen 2". Switch tabs: it stays on. Close and reopen the panel: it is off again, and `config.json` has nothing new.
- [ ] **In use elsewhere still shows.** Give screen 3 a layout tied to screen 1 (pick it from Show all screens on screen 3), then turn Show all screens off. Screen 3's tab still shows that tile, selected, with "For screen 1" under it.
- [ ] **Old entries.** A `custom_layouts` entry from before this change, with no `screens`, shows on every tab.

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
- [ ] **Switch from closed.** Close all three apps and press **Switch now**. Each starts (Terminal and Discord through their AUMID), lands in its zone, and the card shows a green dot and "Active". `workspace.switched` has `launched` 3.
- [ ] **Two Firefox windows.** Capture two Firefox windows in different zones, close them, switch. Each lands in its own zone.
- [ ] **Maximized.** Capture a maximized window, switch. It comes back maximized on its screen, and un-maximizing gives its saved normal size.
- [ ] **Other windows.** Set **Leave**: nothing else changes. Set **Minimize**: other windows on this desk's screens minimize, and the Seam window doesn't. Set **Close** with an unsaved Notepad document: Notepad asks to save and nothing else is closed.
- [ ] **Unticked app.** Untick Launch on an app, close it, switch. It isn't started, and nothing is reported as an error.
- [ ] **Hung app.** Switch while an app is hung. Seam stays responsive, the other windows still land, and the log's hook timing line doesn't move.
- [ ] **Newer switch wins.** Switch to one workspace, then another straight away. The second one finishes and only its card is marked active.
- [ ] **Active after a restart.** Turn the debug log on, switch to a workspace, then quit Seam and start it again. Nothing moves, and the card shows "Active" again, or "Active, moved" if a window moved meanwhile. The log has `workspace in use at start: <id> (remembered)`, and `config.json` has `active_workspace` under the desk. With **Put apps in place when they open** on, an app of that workspace you open after the restart goes to its place, as intended. A last session restore offered at sign-in works as before.
- [ ] **Default.** On a workspace's "..." menu pick **Set as default**: its card shows "Default" in grey after the name, next to the Active dot when both apply, and the menu now says **Stop being the default**. `workspaces.list` has `default: true` for it only. Remove `active_workspace` from `config.json` with Seam stopped and start it: the default is active and the log says `(default)`. **Stop being the default** removes the word. A test client that isn't the panel gets error 4 from `workspace.set_default`, and an id not on this desk gets error 1.
- [ ] **Detected at start.** With no `active_workspace` and no default, arrange the windows so every running app of exactly one workspace is in place, and start Seam: that one is active, with `(detected)` in the log, and nothing moves. With two workspaces that both fit, or none, none is active and the log says `none detected`.
- [ ] **Desk change and delete.** Plug a screen in or out: the other desk's remembered workspace becomes active the same way. Delete the active and default workspace: nothing is active, no card says Default, and both ids are gone from `config.json`.
- [ ] **Active card.** In dark and in light mode, the card of the workspace in use shows a small filled green dot and "Active" in grey text right after its name, the same dot as "In place" in the editor, and its card looks like the others: no amber anywhere. Picking a card still gives it the raised fill and the stronger grey border, whether it's the active one or not. Move one of its windows: within a second it reads "Active, moved", and after **Put windows back** "Active" again. With no workspace in use (plug a screen in or out), no card shows it.
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
- [ ] **Where it is now.** With the app open, move its window to another zone and pick **Where it is now**. The row says the new zone, and Undo gives the old one back. With the app closed it says "Where <app> goes stays as it was" and "That app isn't open, so where it goes stays as it was".
- [ ] **Show as.** Open a row with a click on its name. Pick Minimized, switch: the window is minimized and restores into its place. Maximized sets Goes to "Screen N, full". Normal from Maximized puts the window back at the size it had before it was maximized.
- [ ] **Launch.** Untick an app: it doesn't start on a switch with it closed. Tick it again.
- [ ] **Reorder by drag.** Drag a row by its grip below the next one. The notice says "Moved <app> to place 2", and `windows` in `config.json` is in the new order.
- [ ] **Reorder by keyboard.** Tab to a row, press Alt+Down then Alt+Up. The row moves each time, the focus ring stays on it, and Alt+Up on the first row or Alt+Down on the last does nothing.
- [ ] **Remove.** "..." on a row, **Remove**. It's gone at once, and Undo brings it back in the same place.
- [ ] **Rename and switches.** Rename, the shortcut on Shortcuts, **Start missing apps** and **Other windows** still save, without a notice.
- [ ] **Refused.** With the tray stopped, change anything: "Seam isn't running" shows and nothing changes. In `config.json`, give a rule a zone the screen's layout doesn't have, then rename the workspace: that still saves, and the rule keeps its zone.
- [ ] **Not the panel.** A test client that isn't the panel gets error 4 from `workspace.update`.
- [ ] **Where it is now stamps.** With **Put apps in place when they open** off, add an app with **Add an app** to the workspace in use, open it by hand, put it where you want it and pick **Where it is now** on its row. Drag it away: its row shows "Moved" within a second and the log has `workspace <id>: drifted`, without a switch first.
- [ ] **Narrow panel.** At the panel's default size (1160 by 780, on a panel with no window open before) and at its least width (900), with a workspace whose apps have long names and places, every row shows the grip, icon, name, Now, Goes to, Launch and the "..." at the right, with nothing cut off. Long names, places and Now words end in "..." and hovering shows them in full. At the default size, "Visual Studio Code", "Running in background" and "Screen 2, zone 1" show in full with no "...". Widen the window: the name, Now and Goes to columns all grow, the name and Goes to about twice as fast as Now, until "Running in background" and the places show in full. The column heads stay over their columns as the window narrows and widens. The detail header keeps **Update from screen**, **Switch now** and its "...", with a long name ending in "...". A card with a long name, "Default" and "Active, moved" ends the name in "..." and keeps both words.
- [ ] **Reorder with windows open.** Turn the debug log on, switch to the workspace so its apps are placed, then drag the first row below the second. The log has `workspace rows changed: 2 stamps moved, 0 cleared`. Turn **Put apps in place when they open** on, close the app that is now first and open it again: it goes to its own place, not the other app's. Remove a row whose app is running: the log has `1 cleared`, and dragging that app's window no longer marks the workspace as drifted. Rename the workspace: no `workspace rows changed` line.

### Zones in a workspace

Use the 4-screen desk. Screen 1 is Priority, screen 2 Columns, and the library has "Custom 1".

- [ ] **Section.** A workspace's **Zones** row, under **Add an app**, is closed and says "Every screen keeps its layout" for a workspace from before this change. Click it, or Tab to it and press Enter: one row per screen, named as on the Zones tabs ("1  U2720Q"), each with a thumbnail and **Keep as is**. The menu lists Keep as is, No zones, the six built-in layouts, then "Custom 1".
- [ ] **Capture records the layouts.** Capture. The new workspace's `zones` in `config.json` has all four screens: Priority on 1, Columns on 2, and `{"template": "none", "tree": "leaf"}` on the screens with no zones. **Update from screen** after changing screen 2 to Rows records Rows, and **Save windows to the active workspace** does the same.
- [ ] **Two workspaces, two layouts.** Workspace A: screen 1 Priority, screen 2 Columns, with an app in zone 3 of each. Workspace B: screen 1 Grid, screen 2 "Custom 1", with apps in zone 4 of screen 1 and zone 2 of screen 2. Switch to B: both screens change layout before the apps move, the Zones page shows Grid and Custom 1, and each app lands in its zone of the new layout. Switch to A: the layouts and apps go back. A Shift-drag after each switch shows the new zones.
- [ ] **Missing zone.** Give an app zone 4 on a screen, set that screen to Rows in the workspace (two zones), and switch. The app fills the screen's work area, not maximized.
- [ ] **Keep as is.** Set screen 3 to **Keep as is** in workspace B, change screen 3 by hand on the Zones page, then switch to B. Screen 3 keeps the layout you gave it.
- [ ] **No zones.** Set screen 4 to **No zones** and switch: screen 4 has no zones on the Zones page, and the entry is in `config.json`.
- [ ] **Library layout edited.** With workspace B using "Custom 1" on screen 2, edit "Custom 1" on the Zones page and press Done. Workspace B's thumbnail for screen 2 shows the new tree, and its `zones` entry has the new tree with the same `custom` id. Delete "Custom 1": the entry keeps its tree, has no `custom`, and the menu shows "Custom".
- [ ] **Disconnected screen.** In `config.json`, add a `zones` entry to a workspace under a device path that isn't connected. The section shows it greyed, named by its model, with "Not connected" and **Remove**. A switch leaves the connected screens' other entries alone and nothing fails. **Remove** takes it out, and Undo puts it back.
- [ ] **Snap back restores a layout.** Switch to workspace B, then change screen 1 to Columns on the Zones page. The editor shows screen 1 "Moved" and **Put windows back** appears. Press it: screen 1 is Grid again, its apps go back into its zones, and "Moved" goes away.
- [ ] **Open alongside.** Use **Open alongside** for A: the layouts change as with **Switch now**, and other windows stay.
- [ ] **Last session.** Restore a last session after changing a layout by hand: no layout changes.
- [ ] **Zones off, other window manager.** Turn zones off and switch: the layouts are saved (the Zones page shows them), and the windows move as before. The same with **Another window manager handles the windows** on.
- [ ] **Undo.** Each pick in the Zones section shows "Screen 2 gets Columns" (or "keeps its layout", "gets no zones") with Undo, and Undo puts the entry back.
- [ ] **Import.** Export workspace B and import it on "Laptop only": the layouts are mapped to the screens found by number and model, an entry for a screen with no match is gone, and no entry has a `custom` id.
- [ ] **API.** `workspaces.list` has `zones` for each workspace, `{}` for one with none.

### Adding an app

- [ ] **Open now.** **Add an app** opens a popover with a search field and the tabs **Open now** and **Installed**. Open now lists the app windows with a taskbar button, one per window, with each window's title under the app's name, and nothing from Seam. Click one: the popover closes, the app is the last row with "Keep where it is" and Launch ticked, and "Added <app>" has Undo.
- [ ] **Installed.** The Installed tab lists the Start menu's apps by name with their icons, and no services, uninstallers or links. Search "note": only matching apps stay. Search for something that isn't there: "Nothing matches ...".
- [ ] **A Store app.** Add Calculator from Installed with it closed. The row says "Packaged app". Switch: Calculator starts through its AUMID and isn't listed as `refused`.
- [ ] **Already there.** An app that's already a row shows "In this workspace" in both tabs. Clicking it adds a second row, for a second window of that app.
- [ ] **Never include.** "..." on a row, **Never include this app**. The row goes, its exe is under **Left alone** on Zones and in `zones.exclude`, and a capture leaves it out. Undo brings the row back and takes it off the list. Under **Left alone** on Zones, **Remove** takes an app off the list with Undo. A Store app added from Installed has no Never include until a capture or Where it is now fills in its exe.

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
- [ ] **Snaps and maximize.** Press Win+Left on a placed window: within a second its row shows "Moved" and the log has `workspace <id>: drifted`. Press **Put windows back**, then click a placed window's maximize button: "Moved" again. Click it again to restore: "In place". A window the workspace shows maximized that you restore with the button shows "Moved".
- [ ] **Drag timings.** With the focus border off and nothing pinned, drag a placed window around for 10 s without letting go. The next `wm stats` line has `max` well under 1000 µs and an `avg` of a few µs, the log has at most one `drifted` line for the whole drag, and dragging a window of no workspace adds no `drifted` or `in place` line.
- [ ] **Minimized.** Minimize a window the workspace shows normally: "Minimized" with a hollow ring. A window set to Show as Minimized that is minimized shows "In place".
- [ ] **Administrator.** Start an app of the workspace as administrator: its row shows a red ring and "Can't move: runs as administrator", and a switch skips it.
- [ ] **Closed to the tray.** Put Stream Deck and Spotify in the workspace, then close both windows so they keep running in the tray. Their rows show a hollow grey ring and "Running in background", not "Not running", and no **Put windows back** shows for them. Quit Spotify from its tray icon: within the next status check it shows "Not running". `workspace.status` answers `background` for the one still running. Untick Launch for Stream Deck and switch: `workspace.switched` has it in `skipped` with "Running in background". Tick it and switch: it's started again and shows its window, and a single-instance app goes to its place. With **Put apps in place when they open** on, open Stream Deck's window from its tray icon: it goes to its place if its row is free. Restart Seam with only a workspace's tray apps running and nothing else of it: it isn't detected as the one in use.
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
- [ ] **Opened during a switch.** Untick Launch for one app, close it, and switch with Start missing apps on so the switch starts the others. Open the unticked app by hand while the switch is still starting them. It isn't moved until the switch's `switched in` line, then it goes to its place once, with an auto-apply line after that line. Close it before the switch ends: nothing is logged for it. Open a second window of an app the switch is waiting for: once the switch has placed the first, the second is left alone.
- [ ] **Gate B, hook timings.** With auto-apply on, open and close apps and drag windows for 60 s while crossing the borders. `hook stats` `own max` and `avg` match a run with auto-apply off, and the `wm stats` lines every 30 s have `max` well under 1000 µs.
- [ ] **Drag while it settles.** Open an app and start dragging its window at once. Auto-apply leaves it where you drop it.
- [ ] **Restore isn't an open.** Minimize a window you dragged away and restore it. It isn't put back in its zone. Close an app to the tray and show it again: it isn't moved either.
- [ ] **Reopened.** Close one of the three apps and open it again. It goes back into its zone, because its row is free again.
- [ ] **Never manage.** Add an app of the workspace to **Left alone** on Zones, then open it: it's left where it opens.
- [ ] **Administrator.** Open an app of the workspace as administrator: it's left alone without a notice.
- [ ] **Off.** Turn the switch off and open an app of the workspace. It stays where it opens, and the log's `wm: workspace in use true, auto-apply false` line shows the hooks changed.
- [ ] **No workspace.** Plug a screen in or out, so no workspace is in use, and open an app: nothing happens.
- [ ] **Another window manager.** With komorebi running, auto-apply does nothing.
- [ ] **Release all and quit.** Release pinned windows, then open an already placed app's second window: it's still left alone. Quit Seam and start it again: the windows placed before keep their stamp, so opening the workspace's apps again only places the ones with a free row.

### Window size on another screen

Turn the debug log on. Use two screens with different scaling side by side, for example the 4K 27" at 150% and a 1440p 27" at 100%, plus the 1920×550 HDMI panel, which reports no size. Keep a ruler at hand, and note Notepad's width and height in pixels (from the log or a window spy) before each move. Every resize Seam makes logs `window size (<Mode>): <app> screen A → B, W×H px → W×H px`.

- [ ] **Zones page.** In the Windows card, **When a window moves to another screen** shows Let Windows scale it, Keep its pixel size and Keep its real size, with Let Windows scale it chosen on a new install. **Also when I drag a window to another screen** is off, dimmed, and a click on it changes nothing while Let Windows scale it is chosen. Pick Keep its real size: with the HDMI panel connected the row says "Screen N doesn't report its size, so windows there are scaled by Windows" with its number, and with it unplugged it says nothing. The other two choices never show that line. Search finds the row by "size", "pixels", "scale" and "DPI", and the toggle by "drag". `config.get` has `window_size_on_move` (`"windows"`, `"pixels"` or `"real"`) and `window_size_on_drag`.
- [ ] **Let Windows scale it, key move.** Turn zones off on the 1440p screen. Put Notepad at its own size on the 4K screen and press Shift+Win+Alt+Right until it moves to the 1440p screen. It's sized by Windows' scaling, exactly as before this version, and no `window size` line is logged.
- [ ] **Keep its pixel size, key move.** Pick it and move Notepad across again. It has the same width and height in pixels as before the move, centred where the move put it, with the log line. Back again: still the same pixels.
- [ ] **Keep its real size, key move.** Pick it and move Notepad across. It measures the same with the ruler on both screens, centred where the move put it, with the log line.
- [ ] **Too big.** In each of the two Keep modes, make Notepad nearly as wide as the 4K screen and move it. It's clamped to the 1440p work area, with no part off screen.
- [ ] **Into a zone.** With zones on the 1440p screen, move a window in a zone across in each mode. It takes the zone's size, and nothing is logged.
- [ ] **Swap.** Set a Swap key. Swap a free window on the 4K screen with a free one on the 1440p screen in each Keep mode. Each keeps its own pixels or its own real size, centred where the other was.
- [ ] **API.** `window.move {dir:"right"}` from a script does what the key does in each mode, with the log line.
- [ ] **No EDID size.** With Keep its real size, move Notepad onto the HDMI panel: Windows scales it, and the log has one line saying the screen doesn't report its size. With Keep its pixel size it keeps its pixels there.
- [ ] **Maximized and full screen.** Move a maximized window across in each mode: it's maximized on the other screen and nothing is logged.
- [ ] **Workspace, same screen.** Capture a workspace with Notepad at its own size. Move the taskbar to the left or turn auto-hide on, then switch in each mode: Notepad uses the saved fractions as before, with no `window size` line.
- [ ] **Workspace, other screen.** Capture with Notepad at its own size on the 4K screen and note its pixels and its size with the ruler. The rule has `captured_on` with the 4K screen's device path. Unplug the 4K screen and switch. Let Windows scale it: Notepad stays where it is, as before. Keep its pixel size: it goes to the same place on the screen it's on now with the captured pixels. Keep its real size: the same place at the captured real size. The log has `window size (<Mode>): notepad screen at capture → N`.
- [ ] **Workspace, same screen at another resolution.** Capture on the 4K screen, change its resolution or scaling, and switch in each Keep mode. Notepad is on the screen it was captured on, so it uses the saved fractions with no `window size` line, even where they no longer give its old size.
- [ ] **Workspace, retargeted in the editor.** Capture on the 4K screen, then in the editor send Notepad's place to the 1440p screen with Goes to. `captured_on` still has the 4K path. Switch in each Keep mode: the window is resized to the captured pixels or real size there, with the log line.
- [ ] **Workspace, import from another PC.** Export the workspace, import it on another PC (or edit `captured_on` to a path no screen has). The path matches no screen, so in each Keep mode the window is resized to the captured pixels or real size, with the log line. The exported file still has `captured_on`.
- [ ] **Workspace, old workspace.** A workspace with no `captured_on` in config.json (captured before this version, or an app added from the picker) uses the 10% size check as before: on its own screen it keeps the saved fractions while they give the captured size, and after a resolution change or with its screen gone the Keep modes resize it. A recapture or Where it is now writes `captured_on`. A workspace with neither `size_px` nor `size_mm` leaves the window where it is in every mode.
- [ ] **Workspace on a screen with no size.** Capture on the HDMI panel: the rule has `size_px` and no `size_mm`. Keep its real size always uses the fractions for it, and Keep its pixel size uses the captured pixels once it goes to another screen.
- [ ] **Auto-apply.** With the 4K screen unplugged and **Put apps in place when they open** on, open Notepad from that workspace by hand in each Keep mode. It's placed once at its captured pixels or real size, with the log line.
- [ ] **Drag, toggle off.** With the toggle off, drag Notepad from the 4K screen to the 1440p screen with the mouse in each mode. Windows' own scaling decides, and nothing is logged.
- [ ] **Drag, toggle on.** Turn the toggle on with Keep its pixel size and drag again. About 120 ms after you let go it goes back to the pixels it had before the drag, centred where you dropped it, with the log line. With Keep its real size it goes to the same real size instead. A drag within one screen, a drag into a zone with the show key, and a drag of a maximized window off the top don't resize. A snapped window dragged out with **Restore size when dragged out** on gets its old size, not this one. Pick Let Windows scale it with the toggle still on: drags aren't resized.
- [ ] **Drag with zones off.** Turn zones off altogether: the toggle still works. With komorebi running, or **Another window manager handles my windows** on, nothing resizes after a drag.
- [ ] **Hook timings.** With the toggle on, drag windows across the borders for 60 s. `hook stats` `own max` and `avg` match a run with it off.

### Quick save

Turn the debug log on. Set **Save windows to the active workspace** on Shortcuts (it has no key until you set one).

- [ ] **Into the workspace in use.** Switch to a workspace, move two of its windows and open an app it doesn't have. Press the key. A notification says "Saved <name>" and "N windows saved. Click to undo". The editor shows the moved rows at their new places, the new app at the end, and the launch details and order of the rest as they were, the same as **Update from screen**. The log has `quick save: N windows into <name>`.
- [ ] **Undo.** Do it again and click the notification within 8 s. The workspace is as it was before the save, and the log has `quick save: undone`. Click a quick save's notification after 8 s (from the Action Center): nothing changes and the log says `quick save: too late to undo`.
- [ ] **No workspace in use.** Plug a screen in or out, so no workspace is in use, and press the key. A new workspace "Workspace N", with the next free number, appears on this desk, the notification says it's now in use, and the Stream Deck workspace key for it lights. Click the notification within 8 s: the workspace goes, and none is in use again.
- [ ] **Stamped at once.** With **Put apps in place when they open** on and no workspace in use, press the key, then drag one of the saved windows to another zone without switching: its row shows "Moved" within a second. Close one of the saved apps and open it again: it goes back to where it was saved. The same after **Update from screen** on the workspace in use: a window the update added at the end counts at once.
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
- [ ] **Two views.** On my desk, Displays opens on **Seam**. The 4 to 3 border is a muted dotted line with "Not on your desk" if it fits, the glued ones are dashed amber, the lined-up ones solid amber, and a border never lined up is dashed grey. The caption reads "Real sizes from each screen's EDID, joined where you lined them up". Choose **Windows**: the screens sit as in Display settings, in pixels, touching edges are plain lines, the panel and the U3477 show "18 px gap", and the caption reads "How Windows arranges them, in pixels". Switch to another page and back: the choice stays. Close and reopen the panel: it is back on Seam. Click a screen in each view: its number flashes on it.
- [ ] **Show on screens.** Press **Show on screens** on each view. Every screen goes dim with its number and model large in a corner. Seam: each edge with a link has a band, labelled "to screen 1", "to screen 1, glued" or "Not on your desk". Windows: neighbours are labelled "to screen N", and gaps "to screen N, 18 px gap". Clicks go through the overlay to the app below, and nothing takes focus (type into another app while it's up). Esc, a click, or waiting 8 seconds closes it. Press the button twice quickly: one show, not two. `layout.show` from any client other than the Seam window is refused.
- [ ] **Not on my desk.** Open the **...** of the 4 to 3 border (touching in Windows): **Not on my desk**. The row reads "Not on your desk", the map shows the dotted line, and `config.json` has `"not_on_desk": true` on it. The menu now says **On my desk**: choose it, and the border is back as before. The glued border has no **...** button. On a touching border that was never lined up, **Not on my desk** adds it with the scale 1 and offset 0, and **On my desk** removes it again.
- [ ] **Type measurements.** On Displays, **Type measurements in Line Up** is off, and `lineup_measure` is false or missing in `config.json`. Turn it on and `"lineup_measure": true` is saved. Search "cm" and "offset" finds it, and Line Up then offers M (see [Line Up aids](#line-up-aids)).
- [ ] **Remap switch.** Turn it off. Crossings stop remapping and the sidebar says paused. Turn it on and they work again.
- [ ] **Align switch.** Turn it off, then open the tray menu. **Also align Windows arrangement** is unticked. Turn it back on.
- [ ] **Rename.** Rename a desk from its **…** menu. The sidebar and tray menu show the new name.
- [ ] **Desk dates.** Finish Line Up: the border rows on Displays say "Lined up today" and **Saved desks** says "N screens, lined up today". A desk you haven't used since says "last connected" with a date in local time (check one connected just after local midnight, and one just before UTC midnight if your zone is not UTC). Set `lined_up_at` in `config.json` to a time one, three and ten days ago, and restart Seam: the text reads "yesterday", "3 days ago" and the date. Remove the field: both say plain "Lined up". Seam doesn't write `lined_up_at` for a desk that was never lined up.
- [ ] **Forget and Undo.** Forget the desk in use. The borders go to "Not lined up", the page keeps a sensible desk name, and **Saved desks** says there are none. Press **Undo** within 8 seconds and everything comes back. Forget it again, wait, and the notification goes on its own. Line up to save it again.
- [ ] **Record a shortcut.** On Shortcuts, press **Change**, then Ctrl+Alt+Shift+K. The new keys show and work from another app.
- [ ] **Recorder keys.** Press **Change** then Esc: nothing changes. Press **Change** then Backspace: the row says "Not set" and stays that way after quitting and restarting Seam. **Record** a shortcut again.
- [ ] **Shortcut problems.** Record Ctrl+Alt+K: the row warns that it types characters. Record a shortcut another app holds: the row says so and the keycaps turn red.
- [ ] **Keyboard.** Ctrl+F jumps to search and Esc clears it. Tab walks the sidebar then the page, with a ring on each control, and no ring appears after a mouse click. Space flips a switch. Left and Right change the Tray icon choice.
- [ ] **Search.** Type "log" and press a result. It opens that page.
- [ ] **Search scrolls to the row.** Make the panel window short enough that every page scrolls. On each page, open a search result for a row near the bottom and check the row is in view with some room above it, starting from the top and from the bottom of another page: Displays "Also align Windows arrangement", Zones "Pin border colour" and "Another window manager handles my windows", Workspaces "Other windows" (with a workspace selected), Shortcuts "Capture current windows", General "Debug log", About "Licenses" (shows the Support section) and "Include pre-releases". A result at the top of its page, such as Displays "Line up screens", opens the page at the top. Scrolling by hand afterwards works as before, and picking a page in the sidebar keeps working as it did.
- [ ] **Left-click.** Choose **Open Seam** under Tray icon, then left-click the tray icon. The panel opens. Set it back.
- [ ] **Start with Windows.** Turn it on, sign out and in. Seam starts, and the panel doesn't. Turn it off.
- [ ] **Debug log.** Turn it on, cross a border, and check `seam.log` grows. Turn it off.
- [ ] **Files.** **Open** shows the folder holding `config.json`. **Export…** then **Import…** round-trips your settings, and importing a broken file is refused with nothing changed.
- [ ] **About.** **Copy diagnostics** puts your version, Windows build, screens and borders on the clipboard, with no window titles or file paths.
- [ ] **Panel missing.** Rename `seam-settings.exe`, choose **Open Seam…**. A balloon says it's unavailable and Seam keeps running. Rename it back.

## Sidebar and Help

- [ ] **Sidebar.** Displays, Zones and Workspaces are at the top. The App heading with Shortcuts, General, Help and About sits at the bottom, right above "Seam" and the version. Make the window as short as it goes: the two groups don't overlap. The update dot still shows on About.
- [ ] **Open Help.** Click Help. The topics are on the left, in this order: Getting started, Pointer and borders, Line Up aids, Glue and Not on my desk, Layout views, Zones, Moving windows, Pins and borders, Workspaces, Last session, Shortcuts, Stream Deck, Local API, Updates. Getting started is picked the first time.
- [ ] **Accurate.** Read each topic against the app. Glue, 60 px, G, N, M, **Show on screens** and the **...** menu all behave as the text says, and nothing in it is planned.
- [ ] **Every topic.** Click each one. The picked card is raised, its text shows on the right with the labels in bold, nothing is cut off at the panel's smallest size, and it reads correctly in light and dark.
- [ ] **Getting started buttons.** **Open Displays**, **Open Zones** and **Open Workspaces** on the three steps each open that page.
- [ ] **Topic buttons.** The button under each other topic opens its page: Pointer and borders, Line Up aids, Glue and Not on my desk and Layout views all open Displays, Zones Zones, Moving windows Shortcuts, Pins and borders Zones, Workspaces Workspaces, Last session General, Shortcuts Shortcuts, Stream Deck General, Local API General, Updates About.
- [ ] **Search.** Type "snap back": the results include Help's Workspaces topic, and pressing it opens Help with that topic picked. Type "stream deck", "sign-in" and "pre-releases": each lists its Help topic. Type "glue", "not on my desk", "stripes", "offset" and "show on screens": each lists its Help topic. Type "help": every topic is listed.
- [ ] **Remembered page.** Close the panel on Help and open it again: it opens on Help. Run `seam-settings.exe --page help`: it opens on Help.
- [ ] **Page on an open panel.** With the panel open on Displays, run `seam-settings.exe --page help` and send `panel.open` with `{"page":"about"}`: each switches the open window to that page and brings it to the front, and no second window starts. On Zones, start a layout edit and run it again with another page: the "Discard layout changes?" dialog asks first, and Keep editing leaves the page as it was. An unknown page leaves the open window where it is.
- [ ] **About.** About's last section is called Support and still has **Copy diagnostics**, **Report a problem**, **Source on GitHub** and **Licenses**, each working as before.

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

### Switch back and Open alongside

Set **Switch back to the previous workspace** on Shortcuts (it has no key until you set one). Have two workspaces, A and B, with **Other windows** on Minimize in both, and a Notepad window open that neither has.

- [ ] **Nothing yet.** Restart Seam with a desk that has no previous workspace. The tray's Workspaces menu has no "Switch back" item, and the key shows the balloon "No workspace to switch back to".
- [ ] **Toggle.** Switch to A, then B. The tray's Workspaces menu starts with "Switch back to A" and a separator. Press the key: A is applied. Press it again: B. `config.json` has `previous_workspace` on the desk, and Seam keeps it across a restart.
- [ ] **Deleted.** Delete A while B is in use. The item goes from the tray menu, and the key shows the balloon again.
- [ ] **API.** `workspace.switch_back` answers `{ok: true}` and does what the key does, or error 1 with no previous workspace.
- [ ] **Stream Deck.** Put the **Switch back** action on a key and press it: the same as the hotkey.
- [ ] **Open alongside.** On A's **...** menu choose **Open alongside**. A is applied but Notepad stays where it is, though A's **Other windows** is Minimize. A's setting is still Minimize, and **Switch now** minimizes Notepad again. A is shown as Active after the alongside switch.
- [ ] **API.** `workspace.switch` with `{"name":"A","alongside":true}` leaves Notepad alone, and without `alongside` minimizes it.

## Stream Deck

Build the plugin with `pnpm install`, `pnpm build` and `pnpm run pack` in `integrations/streamdeck`. Run these with Seam running and **Let other apps control Seam** on, and the Stream Deck app open.

- [ ] **Install the packed file.** Double-click `dist\co.f3nn3x.seam.streamDeckPlugin`. Stream Deck installs it, and a Seam group with fifteen actions appears in the action list. General in Seam lists "streamdeck" among the connected apps.
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
- [ ] **Start with Windows.** Turn it on in General, sign out and in. Seam starts and crossings work. Turn it off, sign out and in. Seam doesn't start.
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
- [ ] **Spike source.** Run `seam.exe --log` for 12 minutes of normal use, then read `own_hist`, `next_hist`, `own_over_1ms` and `next_over_1ms` on the `hook stats` lines in `%APPDATA%\Seam\seam.log`. Each histogram counts events per bucket in microseconds: bucket 0 is under 1, then 1, 2-3, 4-7 and so on doubling, and the last is 2048 and up. Spikes in `own_hist` are Seam, spikes in `next_hist` are the hooks after it, and spikes in neither mean scheduling.

## Ease cursor movement off

Turn off **Settings → System → Display → Multiple displays → Ease cursor movement between displays**.

- [ ] **Warning.** Close the Seam window and restart Seam with the setting off. A balloon warns once, and the log shows `Ease cursor movement is off`. Clicking it opens Displays, which shows the alert with **Turn it on**.
- [ ] **No double warning.** With the Seam window open, pause and resume the remap. No balloon comes, because the window shows the setting itself.
- [ ] **First-run ask.** With `cursor_easing_asked` false or missing in `config.json`, open the Seam window on any page but Displays. A dashed note at the top says "Seam needs Windows' Ease cursor movement between displays…" with **Not now** and **Turn it on**. On Displays the note doesn't show, and the alert does.
- [ ] **Not now.** Press **Not now**. The note goes, `cursor_easing_asked` is true, and it doesn't come back after restarting Seam or turning the remap off and on. The Displays alert stays while the setting is off.
- [ ] **Turn it on.** Set it off again and `cursor_easing_asked` back to false. Press **Turn it on** in the note. Without signing out: the Windows Settings toggle shows on (reopen the page), the pointer crosses where it used to stop at an edge, and the log shows `cursor easing: Some(false) to Some(true), Ok(())` plus the `SPI_SETCURSORS` and `WM_SETTINGCHANGE` lines. "Turned on Ease cursor movement" shows, the alert goes, and the note never comes back. Record whether the crossing works straight away or only after a sign-out.
- [ ] **Didn't stick.** Can't be forced by hand. If **Turn it on** ever shows "Ease cursor movement left as it was" with "Windows didn't keep Ease cursor movement on", the log's `cursor easing:` line shows what the registry held afterwards, and `cursor_easing_asked` stays false.
- [ ] **Alert button.** Turn it off in Windows Settings, come back to Displays: the alert shows **Open Display settings** and **Turn it on**. **Turn it on** works as above. With it on, there's no alert.
- [ ] **State.** `state.get` has `windows_cursor_easing` false while off and true after. `cursor_easing.set` from a client without the panel's token is refused, and `{"on":false}` is refused with nothing changed.
- [ ] **Search.** Type "deadzone", "easing" and "jump": each finds "Ease cursor movement between displays", which opens Displays at the top.
- [ ] **Describe what happens.** Expected: crossings where both screens have pixels at that row still remap. Elsewhere the pointer stops at the edge, because Seam only acts when a move leaves the screen across a calibrated border and lands on a physical screen. Try the top of screen 1 left of x=596, and the right edge of screen 1 below where screen 4 ends. Record what you see, then turn it back on.

## Align arrangement

Finishing Line Up with the setting on calls `SetDisplayConfig` only if the plan is accepted.

- [ ] **Refusal on this desk.** With **Also align Windows arrangement** ticked, finish Line Up. The balloon reads "Pointer remap is on. Windows arrangement unchanged. Moving screen 3 would overlap screen 1.", the log shows `arrangement result: Err("Moving screen 3 would overlap screen 1")`, and nothing moves. Moving screen 4 up for its border would take screen 3 with it, and screen 3's 7 px offset to the left would collide with screen 1.
- [ ] **Setting off.** Untick it and finish Line Up. The log shows `arrangement skipped: setting is off`.
- [ ] **Undo in Settings.** Only if an arrangement was actually applied: drag the screens back in Display settings, press Apply, and confirm the log shows `display change` and borders resolve again. If a screen no longer touches on its calibrated side, the log shows `dropped` for that border.

## Workspace app icons

Needs workspaces on this desk that hold the apps below.

- [ ] **Win32 apps.** A workspace with a few desktop apps (Notepad, Firefox, Explorer) shows each app's own icon in a 16 px row under the name, in the order of the rules. The "n windows" caption is gone. Hover an icon: the tooltip is the app name the detail table shows.
- [ ] **Store apps.** Add Windows Terminal and Calculator. Each shows its logo, not a blank or generic icon.
- [ ] **Exe gone.** Rename or uninstall an app a workspace holds. Its slot shows a grey rounded square, no error or toast, and the other icons are unaffected.
- [ ] **Exe back.** With the panel still open, rename the app back. Go to another page and back to Workspaces within a minute: still the grey square. After a minute, go away and back again: its icon shows. Staying on the page doesn't retry it.
- [ ] **Many apps.** A workspace with 12 different apps shows six icons and "+6" on one line. Two windows of the same app show one icon.
- [ ] **Light and dark.** Switch Windows between light and dark. The placeholder and the "+n" are readable in both, and icons with transparent corners show no black or white fringe.
- [ ] **150% scaling.** Set display scaling to 150%. Icons are sharp, not blurry, and still 16 logical px.
- [ ] **No stall.** Open the Workspaces page with a workspace of many apps. The page draws at once and the icons fill in a moment later.

## Screen reader names

Use Narrator (Win+Ctrl+Enter) or Accessibility Insights for Windows, and open the Seam window.

- [ ] **Sidebar.** Each row is read as its page title with "tab", and the open page as selected.
- [ ] **Pause and resume.** The button beside the remap status is read as "Pause remap" while on and "Resume remap" while paused, not as an unnamed button.
- [ ] **More menus.** On Displays, each desk's "..." is read as "More for" and the desk's name. On Workspaces, the list card and the editor's "..." read "More for" and the workspace name, or the app's name in the editor.
- [ ] **Switches.** On General, Displays, Workspaces, Zones and Updates, each switch is read as its row title with "switch" and "on" or "off", and the state changes after you press it.
- [ ] **Choices.** Each segmented control (Show zones, Left-click, Other windows, Show as and the rest) is read as a group named for its row, and each option as its own text with selected or not.
- [ ] **Nothing unnamed.** In Accessibility Insights' live tree, no button, switch or tab on any page has an empty Name.

## Window opens under the pointer

- [ ] **Each screen.** With two or more screens, put the pointer on each one in turn and open Seam from the tray (and from `seam-settings.exe --page general`). The window opens centred in that screen's work area, never partly under the taskbar, and never flashes on the primary screen first.
- [ ] **Different scaling.** With screens at different scaling (say 100% and 150%), open it on each. It's the same apparent size on both, about 1160 x 780 logical, and fully on the screen.
- [ ] **Screen left of the primary.** Place a screen left of or above the primary, so its origin is negative, and open it there. It's centred on that screen.
- [ ] **Small screen.** On a screen smaller than the window (or with a very large taskbar), the window fits inside the work area.
- [ ] **Already open.** With the window open on one screen, open Seam from another screen. The existing window comes forward where it is; it doesn't move.

## Report back

For each failure, give the step, what you saw, and the matching lines from `seam.log`.
