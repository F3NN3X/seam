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

Layouts belong to the set of screens you have connected, so each desk keeps its own.

Windows 11 shows its own snap layouts when you drag a window to the top of the screen. Seam leaves that alone. To turn it off, go to **Settings → System → Multitasking** and switch off "Show snap layouts when I drag a window to the top of my screen".

Seam can't move windows from apps running as administrator. It tells you once when that happens.

To make your own layout, press **Edit layout** on the Zones page (or the **Custom** tile). Click a zone and split it side by side or top and bottom, drag a divider (it snaps to quarters and thirds), merge two neighbouring zones, then press **Done**. Esc throws the changes away and Ctrl+Z undoes the last one. A layout can have up to 128 zones.

The editor opens on the screen itself, over its work area, so you line zones up at full size. The Seam window stays in front for Split, Merge and Done. If you'd rather edit in the preview, turn off **Edit layouts on the screen** and the screen shows a live view that clicks pass through.

Experimental: turn on **Keep windows in their zones** on the Zones page and Seam moves snapped windows back into their zones after a screen, resolution or scaling change, or after waking the PC. It's off by default, and windows you've moved since they snapped are left alone.

### Window control

"Left" here means left on your desk, as the screens physically stand, not in Windows' pixel layout. So left from the top of a tall screen beside two stacked ones goes to the upper one.

- **Shift+Win+Alt+arrows** move the focused window. A window in a zone goes to the next zone that way, and at the edge onto the nearest zone of the screen next to it. A window in no zone goes to the next screen, in the same place there.
- **Ctrl+Shift+Alt+arrows** focus the nearest window that way.
- **Swap** trades places with the nearest window that way, zones included. It has no key until you set one.

Seam leaves windows from apps running as administrator, windows that aren't responding and its own windows alone. Next and previous zone are still on the Shortcuts page and in the API. New installs give them no key, and an earlier `config.json` keeps the keys it has. Win+Alt+arrows aren't the move keys because some Windows builds hold them already.

- **Pin window on top** keeps the focused window above the others until you press it again.
- **Keep window out of workspace switches** makes the focused window sticky: a workspace switch never moves, minimizes or closes it, and capture leaves it out.
- **Focus border**, on the Zones page, colours the border of the window that has focus in your Windows accent colour, the one under Settings > Personalisation > Colours. It's off until you turn it on, needs Windows 11, and skips windows from apps running as administrator.
- **Pin border**: a window pinned on top or sticky gets a Seam amber border, which wins over the focus border until you unpin it. Both colours are picked from a row of swatches under the Windows card on the Zones page: Windows accent, Seam amber, blue, teal, green, red, pink or white. **None** leaves that border to the app and Windows, and **Custom** takes any `#RRGGBB` colour.
- **Release pinned windows**, in the tray menu, takes back every pin and border Seam set. Seam does the same when you quit it. An update or a restart that you didn't ask for keeps the pins: the next start keeps the pins it finds and resets the borders.

The three pin keys have no key until you set one on the Shortcuts page.

If komorebi, GlazeWM or LeopardWM is running, Seam notices within 30 seconds and leaves placing windows to it: drag snapping, the zone keys, move and swap, and keeping windows in their zones are off, and the Zones page says why. **Another window manager handles my windows** on the Zones page does the same for any other one. The pointer remap, Line Up, workspaces, focus keys and pins keep working.

### Workspaces

Arrange your windows the way you like them, then press **Capture current windows** on the Workspaces page. Seam saves which app is on which screen and in which zone (or maximized, or at its own size), for the desk you have connected. **Switch now**, the workspace's shortcut, or **Workspaces** in the tray menu then puts each app back. Apps that aren't running are started if they're ticked under Launch and **Start missing apps** is on, and Seam waits up to 15 seconds for each window. **Update from screen** saves the windows as they are now into a workspace.

**Other windows** says what a switch does with everything else on the desk's screens: leave it, minimize it, or close it. Close only asks. Each app gets a normal close request, so one with unsaved work shows its own prompt and stays open. Seam never ends a process.

Each workspace's **...** menu has **Export...** and **Import...**. An imported workspace starts with launching off, and every app has to be ticked again, because a file from someone else shouldn't be able to start programs for you. Windows are mapped to your screens by device path, then by number and model, then by number, and any that match nothing stay where they are.

Other apps, such as a Stream Deck, can switch workspaces through the local API.

### Tray menu

- **Open Seam…**: opens the Seam window, which holds the other settings.
- **Line Up Screens…**: calibrate.
- **Workspaces**: switch to a workspace on this desk. Only shown when the desk has some.
- **Remap pointer at borders**: pause or resume the hook.
- **Release pinned windows**: take back every pin and focus border Seam set.

Left-clicking the icon opens Seam on a new install. If you're upgrading from v0.1 it keeps opening Line Up. `tray_click` in `config.json` sets which.

Running `seam.exe` again while it's already running opens the Seam window. `seam.exe --line-up` opens Line Up, or starts with it open.

### Shortcuts

- **Ctrl+Shift+Alt+L**: Line Up.
- **Ctrl+Shift+Alt+P**: pause or resume the remap.
- **Shift+Win+Alt+arrows**: move the focused window that way on the desk.
- **Ctrl+Shift+Alt+arrows**: focus the window that way.
- Swap, next or previous zone, and the three pin keys have no key until you set one.
- One per workspace, and one to capture the current windows, all set on the Shortcuts page. Ctrl+Shift+Alt plus a number is a good choice, since Ctrl+Alt alone types characters on many keyboards.

If another app already holds one, Seam keeps running without it.

## Stream Deck

A Stream Deck plugin in `integrations/streamdeck` puts Seam on keys. It talks to the running Seam over the local API, so **Let other apps control Seam** has to stay on in General. These are its actions:

- **Switch workspace**: pick a workspace on the current desk. The key lights while it's in use.
- **Line up screens**: opens Line Up.
- **Pause or resume remap** and **Zones on or off**: two-state keys that follow Seam, so they stay right when you change a setting elsewhere.
- **Next zone** and **Previous zone**: move the focused window.
- **Move window** and **Focus window**: pick a direction in the key's settings. They work as the shortcuts do.
- **Pin on top**: pins the focused window on top, or unpins it.
- **Release pinned windows**: takes back every pin and focus border Seam set.
- **Desk**: shows the active desk's name. Pressing it opens the Displays page.

When Seam isn't running, every key says so and reconnects by itself once it is.

On a PC shared with other Windows accounts, the plugin can't check that the program answering on Seam's connection is really Seam. Seam's own apps do check. The plugin only sends commands like "switch workspace" and reads status, so the most another account could do is see that status or ignore the keys.

To install it, download `co.f3nn3x.seam.streamDeckPlugin` and open it. The Stream Deck app installs it and adds a Seam group to the action list. To build it yourself, run `pnpm install`, `pnpm build` and `pnpm run pack` in `integrations/streamdeck`. The packed file lands in its `dist` folder.

## Install and update

There are three ways to get Seam:

- **Microsoft Store**, once it's live. The Store updates it, and winget finds it through the `msstore` source.
- **The installer**, recommended on GitHub. Download `Seam-Setup-X.Y.Z.exe` from [Releases](https://github.com/F3NN3X/seam/releases) and run it. It installs for you only, with no admin prompt, and adds Seam to the Start menu and to Installed apps.
- **The portable zip.** Download `seam-X.Y.Z-windows-x64.zip` from the same page, unzip it and keep both exes together.

The installer and the zip are unsigned, so Windows SmartScreen warns about an unknown publisher the first time. Choose **More info**, then **Run anyway**. See [Troubleshooting](#troubleshooting).

To update the zip by hand, download the new zip, quit Seam, and replace both exes.

Uninstalling the Store version removes its settings. The installer's uninstaller asks whether to remove them. The portable zip leaves them in `%APPDATA%\Seam`.

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
- Windows of apps running as administrator are skipped by workspaces and zones, because Windows doesn't let Seam move them.
- Apps that restore their own positions when they start may move after Seam places them. Seam places a window once and doesn't fight the app.
- Workspaces don't restore browser tabs or open documents.
- Close only asks. Apps with unsaved work will prompt, and those windows stay open.
- The tray icon is drawn at runtime.
