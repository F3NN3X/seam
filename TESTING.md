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
- [ ] **Second launch.** Run `seam.exe` again. No second process appears, Line Up opens in the first, and the keyboard works immediately (try Tab). Press Esc.
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

Only remaps are timed. A move that stays on one screen isn't logged, so this figure doesn't cover that path.

## Ease cursor movement off

Turn off **Settings → System → Display → Multiple displays → Ease cursor movement between displays**.

- [ ] **Describe what happens.** Expected: crossings where both screens have pixels at that row still remap. Elsewhere the pointer stops at the edge, because Seam only acts when a move leaves the screen across a calibrated border and lands on a physical screen. Try the top of screen 1 left of x=596, and the right edge of screen 1 below where screen 4 ends. Record what you see, then turn it back on.

## Align arrangement

Finishing Line Up with the setting on calls `SetDisplayConfig` only if the plan is accepted.

- [ ] **Refusal on this desk.** With **Also align Windows arrangement** ticked, finish Line Up. The balloon says the arrangement is unchanged, the log shows `arrangement result: Err("aligning would make screens overlap")`, and nothing moves. Moving screen 4 up for its border would take screen 3 with it, and screen 3's 7 px offset to the left would collide with screen 1.
- [ ] **Setting off.** Untick it and finish Line Up. The log shows `arrangement skipped: setting is off`.
- [ ] **Undo in Settings.** Only if an arrangement was actually applied: drag the screens back in Display settings, press Apply, and confirm the log shows `display change` and borders resolve again. If a screen no longer touches on its calibrated side, the log shows `dropped` for that border.

## Report back

For each failure, give the step, what you saw, and the matching lines from `seam.log`.
