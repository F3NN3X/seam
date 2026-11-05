# Seam

Line up your screens on Windows 11 so the pointer crosses exactly where they physically meet.

This is a Windows take on [Shim](https://screenshim.app/) for macOS, with one addition Windows needs. macOS arranges screens in points, but Windows arranges them in **raw pixels**. When two monitors have different pixel densities (a 27" 4K next to a 27" 1440p, a laptop under an external), no arrangement can be right along the whole edge. Windows keeps the same pixel row when the pointer crosses, so the pointer jumps. Seam fixes this with a small low-level mouse hook. Whenever the pointer crosses a border you've lined up, the hook puts it where the screens physically meet.

## Use it

1. In **Settings → System → Display → Multiple displays**, turn on **Ease cursor movement between displays**. Seam relies on it so the pointer can always leave a screen toward its neighbour, and warns you if it's off.
2. Run `seam.exe`. It lives in the tray, and opens the Seam window when you start it yourself. At sign-in it starts in the tray only. On a new set of screens it asks once.
3. Choose **Line Up Screens…** in the tray menu, or press Ctrl+Shift+Alt+L. A full-screen overlay opens on every monitor, with big numbers so you know which screen is which.
4. For each border, the reference screen shows two coloured lines. On the neighbouring screen, drag the matching lines until each one runs straight across the bezel:
   - **Grab a line** to move just that line. This sets the scale.
   - **Drag anywhere else** to move both lines. This sets the height.
   - Arrow keys nudge the selected line (Shift moves 10 px, Ctrl moves both). Tab switches lines. R resets to the guess from your monitors' EDID.
   - The pointer already follows your lines while you adjust, so move it across the border to check.
5. Press **Enter** to go to the next border, or to finish after the last one. **Backspace** goes back, **Esc** cancels.

When you finish, Seam saves the layout for that exact set of screens and turns on the remap. If **Also align Windows arrangement** is checked (it is by default), Seam also moves your monitors in the Windows arrangement. Each border then lines up at the middle of its physical overlap, so things like dragging a window between screens look right even with Seam closed. You can undo this anytime in Display settings.

### Zones

Open **Zones** in the Seam window, pick a layout for each screen (Columns, Rows, Grid or Priority), and hold **Shift** while you drag a window. The zones show on the screen under the pointer, the one under it turns amber, and letting go snaps the window into it. Hold **Ctrl** as well to cover several zones with one window. Both keys, the gap between zones and whether a window gets its old size back when you drop it outside a zone are on the same page.

**Win+Alt+Right** and **Win+Alt+Left** move the focused window to the next or previous zone, carrying on to the next screen after the last zone. A window that isn't in a zone snaps to the one holding its centre. Layouts belong to the set of screens you have connected, so each desk keeps its own.

Windows 11 shows its own snap layouts when you drag a window to the top of the screen. Seam leaves that alone. To turn it off, go to **Settings → System → Multitasking** and switch off "Show snap layouts when I drag a window to the top of my screen".

Seam can't move windows from apps running as administrator. It tells you once when that happens.

To make your own layout, press **Edit layout** on the Zones page (or the **Custom** tile). Click a zone and split it side by side or top and bottom, drag a divider (it snaps to quarters and thirds), merge two neighbouring zones, then press **Done**. Esc throws the changes away and Ctrl+Z undoes the last one. A layout can have up to 128 zones.

Experimental: turn on **Keep windows in their zones** on the Zones page and Seam moves snapped windows back into their zones after a screen, resolution or scaling change, or after waking the PC. It's off by default, and windows you've moved since they snapped are left alone.

### Tray menu

- **Open Seam…**: opens the Seam window, which holds the other settings.
- **Line Up Screens…**: calibrate.
- **Remap pointer at borders**: pause or resume the hook.

Left-clicking the icon opens Seam on a new install. If you're upgrading from v0.1 it keeps opening Line Up. `tray_click` in `config.json` sets which.

Running `seam.exe` again while it's already running opens the Seam window. `seam.exe --line-up` opens Line Up, or starts with it open.

### Shortcuts

- **Ctrl+Shift+Alt+L**: Line Up.
- **Ctrl+Shift+Alt+P**: pause or resume the remap.
- **Win+Alt+Right** and **Win+Alt+Left**: move the focused window to the next or previous zone.

If another app already holds one, Seam keeps running without it.

## Install and update

Download `seam-X.Y.Z-windows-x64.zip` from [Releases](https://github.com/F3NN3X/seam/releases), unzip it and keep both exes together. It's unsigned, so Windows warns about an unknown publisher the first time. See [Troubleshooting](#troubleshooting). A Microsoft Store version is planned for later.

To update, download the new zip, quit Seam, and replace both exes. Your settings stay in `%APPDATA%\Seam`.

Seam has a built-in updater, but it only installs releases signed with Seam's key, and no release is signed yet. About shows the result of each check.

## How it works

For each shared border, Seam stores a linear map between the two edges, relative to each monitor's own origin:

```
b = s · a + o
```

`s` is the physical scale (pixels on B per pixel on A), and `o` is the offset. Two line pairs lined up by eye solve both, so you never measure anything. Because the map is relative to each monitor, it survives rearranging monitors in Windows. Monitors are identified by their device path, so layouts survive reboots and come back when you reconnect the same screens (home desk, office desk, and so on).

The hook runs on its own high-priority thread, so UI work can never delay mouse input. It only acts when the pointer leaves a screen across a calibrated border, and only if a physical screen is on the other side. Everywhere else it leaves Windows' behaviour untouched. It also stays out of the way when:

- An app or game confines the pointer (`ClipCursor`).
- Input is injected (remote desktop, automation tools).
- The screens no longer touch on the same side as when you calibrated.

## Debug log

Run `seam.exe --log`, or set `SEAM_LOG=1`, to append to `%APPDATA%\Seam\seam.log`: monitors, resolved borders, every remap with its time in the hook, a once-a-minute line timing all hook events, hook install and removal, display changes, and arrangement results. Quit any running Seam first, because a second launch only opens the window of the first one. Logging is off by default, and then adds only an atomic load to the hook.

## Build

On Windows with Rust installed:

```
cargo build --release
```

This builds the tray app, `target\release\seam.exe`. To include the settings panel, run `cargo build --release --workspace`, which also needs CMake and the Visual Studio C++ tools. Keep `seam-settings.exe` next to `seam.exe`.

To build the Store package, see `packaging/README.md`.

To cross-compile from Linux, use `./build-cross.sh`. It needs `mingw-w64` and Rust's library sources for `-Zbuild-std`.

`cargo test` runs the geometry and config tests on any OS.

## Troubleshooting

- **"Windows protected your PC".** The GitHub build isn't signed, so SmartScreen warns about it. Choose **More info**, then **Run anyway**.
- **Smart App Control blocks it.** Smart App Control can refuse unsigned apps outright, with no way to run them.
- **The zip won't run.** Windows marks a downloaded zip as untrusted. Unblock it before unzipping: right-click it, choose **Properties**, tick **Unblock**. Or run `Unblock-File seam-X.Y.Z-windows-x64.zip` in PowerShell.
- **A shortcut doesn't work.** NVIDIA, AMD and Intel overlays hold some combinations. General and Shortcuts show a conflict in red. Choose another one in Shortcuts.
- **No update arrives.** About says why when a check fails. The most common cause is a network that blocks `api.github.com`. Check by hand with **Check now**.

## Limits

- Borders are found from the Windows arrangement: two screens must touch there. Screens that touch on your desk but have a gap in the layout ("glue") aren't supported yet.
- Rotated screens work as long as Windows reports their rotated size, which it does. Mirrored screens have nothing to line up.
- "Align Windows arrangement" refuses layouts where moving one screen would break another border (some 2×2 grids). The pointer remap still works in those layouts.
- The tray icon is drawn at runtime.
