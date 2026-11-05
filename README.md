# Seam

Line up your screens on Windows 11 so the pointer crosses exactly where they physically meet.

This is a Windows take on [Shim](https://screenshim.app/) for macOS, with one addition Windows needs. macOS arranges screens in points, but Windows arranges them in **raw pixels**. When two monitors have different pixel densities (a 27" 4K next to a 27" 1440p, a laptop under an external), no arrangement can be right along the whole edge. Windows keeps the same pixel row when the pointer crosses, so the pointer jumps. Seam fixes this with a small low-level mouse hook. Whenever the pointer crosses a border you've lined up, the hook puts it where the screens physically meet.

## Use it

1. In **Settings → System → Display → Multiple displays**, turn on **Ease cursor movement between displays**. Seam relies on it so the pointer can always leave a screen toward its neighbour.
2. Run `seam.exe`. It lives in the tray. On a new set of screens it asks once.
3. Click the tray icon, or choose **Line Up Screens…** in its menu. A full-screen overlay opens on every monitor, with big numbers so you know which screen is which.
4. For each border, the reference screen shows two coloured lines. On the neighbouring screen, drag the matching lines until each one runs straight across the bezel:
   - **Grab a line** to move just that line. This sets the scale.
   - **Drag anywhere else** to move both lines. This sets the height.
   - Arrow keys nudge the selected line (Shift moves 10 px, Ctrl moves both). Tab switches lines. R resets to the guess from your monitors' EDID.
   - The pointer already follows your lines while you adjust, so move it across the border to check.
5. Press **Enter** to go to the next border, or to finish after the last one. **Backspace** goes back, **Esc** cancels.

When you finish, Seam saves the layout for that exact set of screens and turns on the remap. If **Also align Windows arrangement** is checked (it is by default), Seam also moves your monitors in the Windows arrangement. Each border then lines up at the middle of its physical overlap, so things like dragging a window between screens look right even with Seam closed. You can undo this anytime in Display settings.

### Tray menu

- **Line Up Screens…**: calibrate (left-clicking the icon does the same).
- **Remap pointer at borders**: pause or resume the hook.
- **Also align Windows arrangement**: whether finishing a line-up also moves monitors in Display settings.
- **Launch at login**: per-user `Run` key, no admin needed.
- **Open settings folder**: `%APPDATA%\Seam\config.json`.

Running `seam.exe` again while it's already running opens Line Up. `seam.exe --line-up` starts with it open.

## How it works

For each shared border, Seam stores a linear map between the two edges, relative to each monitor's own origin:

```
b = s · a + o
```

`s` is the physical scale (pixels on B per pixel on A), and `o` is the offset. Two line pairs lined up by eye solve both, so you never measure anything. Because the map is relative to each monitor, it survives rearranging monitors in Windows. Monitors are identified by their device path, so layouts survive reboots and come back when you reconnect the same screens (home desk, office desk, and so on).

The hook only acts when the pointer leaves a screen across a calibrated border, and only if a physical screen is on the other side. Everywhere else it leaves Windows' behaviour untouched. It also stays out of the way when:

- An app or game confines the pointer (`ClipCursor`).
- Input is injected (remote desktop, automation tools).
- The screens no longer touch on the same side as when you calibrated.

## Build

On Windows with Rust installed:

```
cargo build --release
```

The exe is `target\release\seam.exe`.

To cross-compile from Linux, use `./build-cross.sh`. It needs `mingw-w64` and Rust's library sources for `-Zbuild-std`.

`cargo test` runs the geometry and config tests on any OS.

## Limits (v0.1)

- Borders are found from the Windows arrangement: two screens must touch there. Screens that touch on your desk but have a gap in the layout ("glue") aren't supported yet.
- Rotated screens work as long as Windows reports their rotated size, which it does. Mirrored screens have nothing to line up.
- "Align Windows arrangement" refuses layouts where moving one screen would break another border (some 2×2 grids). The pointer remap still works in those layouts.
- The tray icon is drawn at runtime, and the exe itself has no icon yet.
