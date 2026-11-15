# Seam local API

The local API is how other programs on the same PC talk to a running Seam. The Stream Deck plugin uses it, and so do Seam's own settings window and updater. You can use it from a script, a launcher, a hotkey tool or your own integration: read what Seam is doing, switch workspaces, move windows, line up screens.

It is local only. Seam opens one named pipe for the signed-in user, and nothing listens on the network. No other user on the PC can open it.

- [Turning it on and off](#turning-it-on-and-off)
- [Transport](#transport)
- [Security model](#security-model)
- [Lifecycle](#lifecycle)
- [Methods](#methods)
- [Events](#events)
- [The state object](#the-state-object)
- [Error codes](#error-codes)
- [Examples](#examples)
- [MCP](#mcp)
- [Versioning](#versioning)

## Turning it on and off

**Let other apps control Seam** on the General page of the Seam window switches the API. It is on by default, and General shows how many apps are connected. Assistants through `seam-mcp.exe` have two more switches under it, off by default, and are counted apart. See [MCP](#mcp).

When it is off, every other client gets error `5` from `hello` and from everything else. The Seam window and the updater keep working, because they identify themselves as Seam (see [Security model](#security-model)).

## Transport

- Pipe: `\\.\pipe\seam-<username>`, with `<username>` in lowercase. It is the `USERNAME` environment variable, or `os.userInfo().username` in Node. Clients derive the name themselves, so there is no discovery file to read.
- The pipe is a byte stream, and the server refuses remote clients.
- Framing: UTF-8 JSON Lines. Every request, reply and event is one JSON object on one line, ending in `\n`. A line can be at most 1 MiB. A client that lets more than that pile up without a newline is disconnected.
- Protocol: JSON-RPC 2.0. A request has `jsonrpc`, `id`, `method` and optional `params`. Without `"jsonrpc":"2.0"` the request is rejected with `-32600`.

```
-> {"jsonrpc":"2.0","id":1,"method":"state.get"}
<- {"jsonrpc":"2.0","id":1,"result":{ ... }}
<- {"jsonrpc":"2.0","id":2,"error":{"code":3,"message":"No window has focus"}}
<- {"jsonrpc":"2.0","method":"event","params":{"type":"workspace.switched","data":{ ... }}}
```

Replies carry the `id` of the request. Events have no `id` and the method `event`. They can arrive between a request and its reply on a subscribed connection, so match replies by `id`.

Empty lines are ignored. A line that isn't JSON gets a reply with `id` `null` and error `-32700`.

## Security model

**Same user only.** The pipe is created with an ACL that grants access to the current user's SID and nobody else, and remote clients are refused. Other accounts on the PC can't connect.

**Checking the server.** Any process can create a pipe under a predictable name, so a client that sends commands should check who it is talking to. Seam's own programs do two things:

1. Open the pipe with SQOS set to identification (`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`), so the server can learn who the client is but can never act as the client.
2. Call `GetNamedPipeServerProcessId` on the handle and check that the process runs as the same user (compare its SID with yours).

The Stream Deck plugin does neither. Node's `net` module opens the pipe with default quality of service and has no way to read the server's process. On a PC shared with other Windows accounts, that means the plugin can't tell that the program answering is Seam. The plugin only sends commands like "switch workspace" and reads status, which limits what another account could do. If your client sends anything sensitive, or runs somewhere other accounts can reach, do both checks. A script that only reads status is the same as the plugin.

**Who may call what.** The server decides for each connection, from two facts:

- **Panel.** The Seam window proves it is the panel by sending the session's token in `hello`. The tray makes a fresh random token every time it starts and passes it to `seam-settings.exe` on its command line. No other client has it.
- **Seam image.** The client process is `seam.exe` or `seam-settings.exe` from the same folder as the running tray. The server reads that from the pipe's client process id.

Most methods are open to any client while **Let other apps control Seam** is on. A few are not, and answer error `4` ("Only Seam can do that") to everyone else. The methods table has a **Who** column:

| Who | Meaning |
|---|---|
| Any | Any client, while the API is on |
| Panel | Only the settings window, with the token |
| Panel or Seam | The settings window, or a Seam exe |
| Seam | Only a Seam exe |
| Panel or Assistant | The settings window, or `seam-mcp.exe` while **Let assistants change workspaces** is on |

**Assistants.** A client whose process is a `seam-mcp.exe`, from any folder, is held to a short list of methods whatever it calls itself: `app.info`, `state.get`, `desks.list`, `workspaces.list`, `workspace.get`, `workspace.status`, `workspace.switch`, `workspace.snap_back`, `workspace.switch_back`, `events.subscribe`, and, while **Let assistants change workspaces** is on, `workspace.quick_save` (it adds a row for every window on screen) and the edits above. Everything else answers `4` ("Assistants can't do that"). Its `workspace.switch`, `workspace.snap_back` and `workspace.switch_back` always leave the other windows alone, whatever `alongside` says. Both switches are checked on every call, not only at `hello`. While **Let assistants use Seam (MCP)** is off, its `hello` and every call answer `5` with "Assistant access is off in Seam, General". This changes at once, with no restart. **Let other apps control Seam** has to be on as well. See [MCP](#mcp).

This is a courtesy limit, not a security boundary. Seam tells an assistant's server by its process name, so a copy of `seam-mcp.exe` renamed to anything else, or any other program on the PC, is an ordinary client of the same user, with everything **Any** allows (`workspace.switch` without `alongside` included), and none of what is Panel only. The pipe has no credential beyond the account, so the switches keep an honest `seam-mcp.exe` within bounds and keep an assistant from getting further by itself. They do not stop software you run.

Error `4` is therefore not a bug in your call. It means the method isn't open to third parties. Settings are changed through the Seam window, and everything else a client needs is in the open methods.

## Lifecycle

1. Connect to the pipe. If it fails with "pipe busy" (Windows error 231), wait a moment and try again. Seam serves at most 8 clients at once. Seam's own client retries up to 20 times, 50 ms apart.
2. Send `hello` first. Calls before it work too, but the connection is then listed as `anonymous` in General. The reply tells you the API and Seam versions.
3. Make calls, and read events if you subscribed.
4. **Idle timeout.** Seam closes a connection that sends no request for 30 seconds, unless it has subscribed to events with `events.subscribe` (at least one topic). So a client that calls now and then should connect again on demand: Seam's own client reconnects when it has been quiet for 25 seconds. A client that subscribes can sit idle for as long as it likes. The Stream Deck plugin does this, and reconnects with a delay that grows from 1 s to 30 s when the connection drops.
5. A subscribed client that stops reading and falls more than 256 events behind is disconnected. A write that blocks for 2 seconds also ends the connection.

A command that has to run on Seam's main thread waits up to 60 seconds for an answer, then fails with error `2`.

## Methods

Every method takes `params` as an object, or none. Unknown params are ignored. A wrong or missing one gives `-32602`. Error codes are listed [below](#error-codes), and `-32601` (no such method) can happen for any name. With the API off, every method but `hello` answers `5` to a client that isn't the panel or a Seam exe, and `hello` itself answers `5`.

In the examples, `->` is what you send, with the `jsonrpc` and `id` fields left out to keep each line short.

### Session

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `hello` | `client?` name, `version?`, `token?` | `{api_version, seam_version, packaged}` | Any | 5 | `{"method":"hello","params":{"client":"my-script","version":"1.0"}}` |
| `app.info` | none | `{seam_version, build, packaged, pid}` | Any | | `{"method":"app.info"}` |
| `events.subscribe` | `topics`: list of strings | `{ok: true}` | Any | | `{"method":"events.subscribe","params":{"topics":["state","workspace"]}}` |
| `app.quit` | none | `{ok: true}` | Panel or Seam | 4 | `{"method":"app.quit"}` |

`client` is the name General shows. `packaged` is true for the Microsoft Store build. `events.subscribe` replaces the connection's earlier subscription, and an empty list turns events off. Topic names aren't validated.

### Status and config

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `state.get` | none | [State](#the-state-object) | Any | | `{"method":"state.get"}` |
| `config.get` | none | The saved `config.json`, as JSON | Any | | `{"method":"config.get"}` |
| `config.patch` | `patch`: object, a JSON merge patch | `{ok: true}` | Panel | 3, 4 | `{"method":"config.patch","params":{"patch":{"enabled":false}}}` |
| `remap.set` | `on`: bool | `{on}` | Any | `-32602` | `{"method":"remap.set","params":{"on":false}}` |
| `zones.set` | `on`: bool | `{on}` | Any | `-32602` | `{"method":"zones.set","params":{"on":true}}` |
| `autoapply.set` | `on`: bool | `{on}` | Any | `-32602` | `{"method":"autoapply.set","params":{"on":true}}` |
| `cursor_easing.set` | `on`: bool | `{on}`, the setting as Windows reports it | Panel | 3, 4 | `{"method":"cursor_easing.set","params":{"on":true}}` |
| `seam.pause` | `on?`: bool | `{paused}` | Any | `-32602` | `{"method":"seam.pause","params":{"on":true}}` |

`state.get` and `config.get` read a snapshot, so they answer at once even while Seam is busy. `config.patch` is validated: a patch that makes an invalid config, or comes from a different config version, answers `3` and changes nothing. `autoapply.set` is "Put apps in place when they open". `config.patch` can also set `app_rules`, the list behind "Where apps open": each entry is a workspace row with `exe` (or `aumid`), `monitor` (the screen's device path) and `target` (`{zone}`, `{state: "max"}` for the full screen). An entry can also carry `pin` (`"top"`, `"sticky"` or `"both"`), which pins the app's windows when they open, or when the entry is made or changed while the app is open, and once only: Release pinned windows frees a window until it's opened again. An entry with `pin` and `target` `{state: "keep"}` only pins, and needs no `monitor`. An entry with neither a place nor a pin is ignored. A patch replaces the whole list, and an entry this build can't read is kept and ignored. A screen's layout on a desk or in a workspace's `zones` with that entry's `custom` id takes the entry's new tree, and keeps its tree without the id when the entry is deleted. `config.patch` also sets the pointer's push past an edge: `pointer.edge_resistance_px` (0 to 100, 0 is off) holds the pointer at the edge to any other screen until it has pushed that many pixels past it, and `pointer.glue_push_px` (0 to 200, default 60) does the same at a glued edge. Larger values are read as the top of the range. `cursor_easing.set` only accepts `true`: Seam never turns Ease cursor movement off, and `false` answers `3`. `seam.pause` pauses or resumes everything Seam does, and toggles when `on` is left out. While paused, Seam's shortcuts (except the one that resumes it), drag snapping, Spotlight, the focus border, the divider handles, the pointer remap and edge resistance, app rules and auto-apply are off. The API, the tray, the Seam window and switching workspaces still work. A pause is not saved, so a restart resumes. `config.patch` can set `step_aside_fullscreen` (default `true`), which does the same for the shortcuts, snapping and Spotlight while a game or other full-screen app is in front, and `hotkeys.pause_seam`, the shortcut for `seam.pause` (no key by default). `config.patch` also sets "When I sign in" per desk, as `desks.<sig>.sign_in`: `{mode, workspace?, ask}`. `mode` is `"nothing"` (default), `"last_session"` (offer the last session, and `restore_session` follows the desks) or `"workspace"`, which opens the workspace with the id `workspace` when Seam starts at sign-in. `ask` (default `true`) shows "Open <workspace>? Click to open" and waits for the click, and `false` switches once Windows is ready. A partial `sign_in` merges onto the desk's own value, so `{"ask": false}` keeps the mode. The first start after an update gives every desk a `sign_in` from the old `restore_session`, and a desk seen later starts at `nothing`. `restore_session` is still written, as "some desk uses `last_session`", but nothing reads it back; patching it sets every desk's `mode` to `last_session` (`true`) or `nothing` (`false`). With `ask` `false`, Seam switches about 10 s after the taskbar is up. Nothing opens with no `workspace`. If the workspace is gone or isn't on the desk, Seam retries, and gives up with one log line 3 minutes after it started. It waits while a full-screen app is in front, and a pause by hand cancels it. It acts on the desk that is current when it fires. `config.patch` also sets "Move and resize from anywhere in a window" as `windows.alt_drag`: `{on, key}`, where `on` defaults to `false` and `key` is `"alt"` (default), `"win"` or `"ctrl_alt"`; anything else reads as `"alt"`. Hold the key and drag with the left button to move a window, or with the right to resize it. It skips apps in `zones.exclude`, Seam's own windows and apps running as administrator, and is off while Seam is paused or standing aside for a full-screen app.

### Desks and Line Up

A desk is one set of connected screens, with its own borders, zones and workspaces.

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `desks.list` | none | `[{sig, name, screens, active}]` | Any | | `{"method":"desks.list"}` |
| `lineup.start` | none | `{ok: true}` | Any | 2 | `{"method":"lineup.start"}` |
| `arrangement.align` | none | `{ok: true}` | Any | 2, 3 | `{"method":"arrangement.align"}` |
| `screen.identify` | `number`: Line Up number, from 1 | `{ok: true}` | Any | 1 | `{"method":"screen.identify","params":{"number":2}}` |
| `layout.show` | `view`: `"seam"` or `"windows"` | `{ok: true}` | Panel | 4 | `{"method":"layout.show","params":{"view":"windows"}}` |

`lineup.start` answers at once and opens the overlay. Watch for `lineup.finished`. `arrangement.align` moves the screens in the Windows arrangement to match the saved borders. It answers when done, and its refusal text (for example "Moving screen 3 would overlap screen 1", or that moving a screen would break its border or glue with another) is in `error.message` and in the state's `last_align`. `screen.identify` shows the screen's number on it for 1.5 seconds. `layout.show` draws the layout on every screen: each screen's number and model, and a band along each edge that has a neighbour, labelled with it. `"seam"` marks the lined-up borders, and `"windows"` the neighbours and gaps in the Windows arrangement. It answers at once, and Esc, a click or 8 seconds closes it.

### Zones and windows

These act on the window in the foreground, using the screens as they stand on your desk. Seam leaves alone windows of apps running as administrator, windows that aren't responding, and its own windows.

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `zone.move` | `dir`: `"next"` or `"prev"` | `{ok: true}` | Any | 3 | `{"method":"zone.move","params":{"dir":"next"}}` |
| `window.move` | `dir`: `"left"`, `"right"`, `"up"` or `"down"` | `{ok: true}` | Any | 2, 3 | `{"method":"window.move","params":{"dir":"left"}}` |
| `window.focus` | `dir`, as above | `{ok: true}` | Any | 2, 3 | `{"method":"window.focus","params":{"dir":"up"}}` |
| `window.swap` | `dir`, as above | `{ok: true}` | Any | 2, 3 | `{"method":"window.swap","params":{"dir":"right"}}` |
| `window.resize` | `by`: `"grow"` or `"shrink"`; `step`: optional, 1 to 10000 pixels, default 5% of the screen and at least 32 | `{ok: true}` | Any | 2, 3 | `{"method":"window.resize","params":{"by":"grow"}}` |
| `window.priority` | none | `{ok: true}` | Any | 2, 3 | `{"method":"window.priority","params":{}}` |
| `window.share` | `hwnd`: optional, the focused window by default; `axis`: `"width"` or `"height"`; `fraction`: 0.1 to 0.9 | `{ok: true}` | Any | 2, 3 | `{"method":"window.share","params":{"axis":"width","fraction":0.6667}}` |
| `window.ungroup` | `hwnd`: optional, the focused window by default | `{ok: true}` | Any | 3 | `{"method":"window.ungroup","params":{}}` |
| `window.fit` | `hwnd`: optional, the focused window by default | `{ok: true}` | Any | 3 | `{"method":"window.fit","params":{}}` |
| `window.stack` | `on`: `true` or `false`; `hwnd`: optional, the focused window by default | `{on, windows}` | Any | 3 | `{"method":"window.stack","params":{"on":true}}` |
| `window.stack_cycle` | `dir`: `"next"` or `"prev"` | `{hwnd, position, windows}` | Any | 3 | `{"method":"window.stack_cycle","params":{"dir":"next"}}` |
| `tiling.set` | `on`: `true` or `false`, `family`: `"smart"`, `"columns"`, `"main_stack"` or `"grid"`, at least one; `number`: optional, the screen under the pointer by default | `{on, family}` | Any | 1, 3 | `{"method":"tiling.set","params":{"number":1,"on":true}}` |
| `tiling.free` | `number`: optional, the screen under the pointer by default | `{ok, freed}` | Any | 1, 3 | `{"method":"tiling.free","params":{"number":1}}` |
| `screen.tile` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true, tiled, left}` | Any | 1, 2, 3 | `{"method":"screen.tile","params":{"number":2}}` |
| `screen.rotate` | `number?`: Line Up number, from 1; the screen under the pointer when left out. `dir?`: `"next"` (default) or `"back"` | `{ok: true, moved, left}` | Any | 1, 2, 3, -32602 | `{"method":"screen.rotate","params":{"number":2,"dir":"back"}}` |
| `screen.reset_sizes` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true, moved, left}` | Any | 1, 2, 3 | `{"method":"screen.reset_sizes","params":{"number":2}}` |
| `zones.from_windows` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true, id}` | Any | 1, 2, 3 | `{"method":"zones.from_windows","params":{"number":1}}` |
| `zones.quick` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true}` | Any | 1, 3 | `{"method":"zones.quick","params":{"number":2}}` |
| `zones.layouts` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{number, current, layouts: [{id, name, kind}]}` | Any | 1, 3 | `{"method":"zones.layouts","params":{"number":2}}` |
| `overview.show` | none | `{ok: true}` | Any | | `{"method":"overview.show"}` |
| `zones.apply` | `number?`: as above; `layout`: the id or name of a layout the screen is offered, or null for no zones | `{ok: true}` | Any | 1, 3, `-32602` | `{"method":"zones.apply","params":{"number":1,"layout":"Main + stack"}}` |
| `zones.pick` | `number`: Line Up number, from 1; `layout`: a screen layout, or null for no zones | `{ok: true}` | Panel | 1, 2, 3, 4, `-32602` | `{"method":"zones.pick","params":{"number":1,"layout":null}}` |

Error `3` carries the reason in `message`: no window has focus, nothing in that direction, the window can't be moved, runs as administrator or isn't responding. `window.resize` and `window.priority` act on the zones of the focused window's screen and every window in them, and also answer `3` when the window isn't in a zone, when no divider beside it can move, or when one of the windows that would move can't be. A second `window.priority` on the same layout puts it back as it was. A changed layout is saved, as Custom unless it is back to its template. `window.move`, `window.swap`, `window.resize` and `window.priority` also answer `3` while komorebi, GlazeWM, LeopardWM or "Another window manager handles my windows" has Seam leaving placement alone. `window.focus` still works then. Error `2` means a newer window command replaced this one before it ran.

`window.share` moves the divider beside a window in a zone so the window gets `fraction` of the span of itself and the zones beside it along `axis`, as the right-click menu of a divider handle does. Of the dividers on its two sides along that axis, it takes the one towards the bigger neighbour. The windows on both sides follow, none goes below the smallest size its app allows, and the layout is saved as for `window.resize`, with the same errors. `window.ungroup` takes a window out of its zones: it gets its size from before it was snapped, centred on those zones, and the layout and the other windows stay. A window snapped before the tray started, or maximized before it was snapped, gets two thirds of its zones. It answers `3` when the window isn't in a zone, with zones off, for the same windows Seam can't move, and while another window manager has Seam leaving placement alone. Both act on `hwnd` when it is given, and on the focused window otherwise.

`window.fit` is **Fit group to screen**: it stretches a window's group over its screen's work area, keeping each window's share and order, and offers Undo for 8 seconds. A group is the ad-hoc group a drop beside a window made on a screen with no zones. A window in no group is given the work area itself, not maximized. It answers `3` with "Already fills its screen" for a window in zones, whose layout covers the work area already, and for a group or window that fills it already, and for the same windows Seam can't move, one of a group's included, and while another window manager has Seam leaving placement alone. It acts on `hwnd` when it is given, and on the focused window otherwise. `config.patch` sets its shortcut as `hotkeys.fit_group` (no key by default), and `zones.snap_beside` (default `true`): while zones show in a drag, the pointer in the band along the edge of another window (15% of its size, 24 to 120 px) puts the dragged window beside it instead of in a zone, splitting the zone that window holds on a screen with zones, or making an ad-hoc group on a screen with none. Ad-hoc groups are kept in memory only.

`tiling.set` turns smart tiling on or off on a screen for the virtual desktop in front, or sets how it arranges the windows there, and answers what the screen has now. It answers `3` when turning it on with zones off, and `1` for a screen that isn't connected. Off by default on every screen and desktop. While it is on, each window that opens on that screen, settles for 150 ms and isn't claimed by a workspace row or an app rule takes a share of the screen: `smart` splits the largest window's space along its longer side, and `columns`, `main_stack` and `grid` share the screen evenly in that shape. A window that can't shrink to its share joins a stack in that space and takes a share of its own once there is room; one that closes leaves a gap the others fill. A minimized one's space waits 600 ms after its minimize and after the last restore, then closes; windows minimized within 250 ms of each other (Win+D) keep their spaces until they are restored or closed, or until 30 s pass with none restored. A tiled window a workspace switch left minimized rests once the switch ends, as one minimized by hand: its space closes 600 ms later, and a restore takes it back while it is free. A restored one takes back its space when it is still free, or a share. While Seam is paused or stands aside for a full-screen app, while zones are off and during a workspace switch, nothing moves; gaps close afterwards. A workspace switch's minimizes, and those up to 500 ms after it, never reach tiling as they happen; opens and restores during it are kept and taken once it ends, except the windows it placed, and a window a workspace placed is never tiled again on restore. Dialogs, owned and tool windows, windows that can't be resized, maximized windows, apps in `zones.exclude`, administrator windows, windows a workspace or an app rule placed, and windows of an app rule with `float: true` float. A tiled screen's zones are its tree for that desktop, kept in memory, so drags, S, splits and handles work on it as on any zones and its own layout stays saved under it. A tiled window dragged out of its space floats where it is dropped, and the others fill the space. One moved or resized without a drag, by Win+arrow keys, its app or a workspace switch, floats where it is and is never moved back, and the others fill its space. A capture, Update or Quick save only stamps windows and floats none. One rule runs before every move tiling makes, so this holds for opens, closes, gaps, family changes and drags alike. A window that closes or goes to another desktop unseen leaves its tree. Main + stack keeps a dragged main share; Columns and Grid are even again at each open or close. A capture saves a tiled screen's windows by their place, not by zone, since the tree changes with each open and close. Each window keeps a `Seam_tile` property with the order it was tiled in, so a restart tiles them again, and one floated by hand a `Seam_float` property. `tiling.free` is **Free all windows**: every window tiling took on the screen goes back to where it was before, and tiling turns off there for the desktop in front; it answers `3` when tiling is off there. The settings are kept per desk as `desks.<sig>.tiling.<device path>`: `{family, desktops}`, `desktops` being the ids of the virtual desktops it is on for. `config.patch` sets the shortcuts as `hotkeys.float_tile` (**Float or tile this window**) and `hotkeys.free_tiling` (no key by default). An older Seam drops `tiling` and `float`, and tiles nothing. Assistants can't use either method.

`config.patch` sets `zones.tiling_free_on_quit` (default `false`), "Put windows back when Seam quits". With it on, a quit the user asked for, from the tray or `app.quit` from the Seam window, frees every screen tiled on every virtual desktop, as `tiling.free` does, and turns tiling off there, and waits up to 1 s (shared with releasing pins) for the moves. An update restart, the Store copy taking over, a crash and a sign-out or shutdown leave the arrangement as it is.

`window.stack` with `on: true` makes a stack of a window in a zone and the other windows Seam has in the same zones, with the window on top, and answers how many windows the stack has. With `on: false` it takes the window out of its stack, where it stays in its zone, and answers `windows: 0`. Only windows on the current virtual desktop join. `window.swap` takes a stack's top out of its stack, as `window.move` does; a swap by dragging carries the whole stack. A stack keeps every window sized to its zones and hides the others behind the top one by stacking order only, so Alt+Tab and the taskbar still reach each, and the window that takes the focus is the top. `window.stack_cycle` puts the next or previous window of the focused window's stack on top and gives it the focus, and answers its handle, its place in the stack from 1 (`position`) and how many windows the stack has. Both answer `3` when the window isn't in a zone, shares it with no other window, or isn't in a stack, with zones off, and while another window manager has Seam leaving placement alone. A drop makes a stack too: press S while zones show in a drag, and the window goes on top of the one in the zone rather than swapping with it, with Undo for 8 seconds. Each window keeps a `Seam_stack` property with its stack and place in it, so a restart of Seam finds its stacks again. `config.patch` sets the shortcuts as `hotkeys.stack_next` and `hotkeys.stack_prev` (no key by default). A workspace row has `stack`, its window's place in a stack from the top at capture (0 is the top), left out when the window wasn't in one. The rows in one zone with a `stack` are a stack again after a switch. An older Seam drops the field, and those windows only overlap in their zone.

`screen.tile` puts the loose windows of one screen into its zones, up to nine, and answers how many it placed (`tiled`) and how many found no zone or were past the ninth (`left`). A window already in a zone keeps it. A screen with no layout is given one for the number of windows and keeps it. It answers `3` with "Nothing to tile" when there is nothing to place, and while zones are off or another window manager has the windows, and `1` for a number that isn't a screen. Pinned, sticky, minimized and administrator windows are left alone.

`screen.rotate` moves every window snapped in the zones of one screen to the next zone in zone order, the zone keys' order, or with `dir` `"back"` the previous one, the last wrapping to the first, and answers how many moved (`moved`) and how many snapped windows stayed (`left`). A stack goes as one. Floating windows, pinned and sticky windows and windows of apps running as administrator stay where they are, and a zone one of them holds is stepped over. A minimized window, or one on another virtual desktop, keeps its zone for when it is back, so a zone it holds is stepped over too; an empty zone is an open step. A tiled screen rotates its tiles, after the same check as any tiling move: a window moved by hand floats first. `screen.reset_sizes` puts a screen whose layout the Zones page shows as **Changed** back to the layout it came from, with that layout's proportions, and its windows follow; a tiled screen's splits even out instead, and Main + stack goes back to its default main share. A layout records what it was cut from as `from`, the template's name or the library id, when it first changes, and keeps it through further changes; picking a layout or saving one clears it. A layout without it, from an older Seam, goes to the built-in or library layout that cuts the screen the same way, only when no other is anywhere near in its ratios. Both answer `3` when zones are off, another window manager has Seam leaving placement alone, a workspace switch is running, nothing can move, or, for a reset, the layout hasn't changed no layout cuts it the same way, or two layouts are about as near that Seam won't guess, and `1` for a number that isn't a screen. Each offers Undo for 8 seconds in a notification, as `screen.tile` does. `config.patch` sets the shortcuts as `hotkeys.rotate` and `hotkeys.reset_sizes` (no key by default); Shift added to the rotate key goes back. Assistants can't call either.

`zones.from_windows` is **Save from windows** on the Zones page. It turns where the windows on one screen are now into a layout with a zone for each: it looks for a straight cut across the screen with windows on both sides and none across it, side by side first, then does the same on each side. A cut within a few pixels of a quarter, a third, a half, two thirds or three quarters goes there. The layout is added to the library of custom layouts as "Custom N", the screen is given it, and each window snaps into its zone. It answers with the new layout's `id`. Windows that overlap, or that no straight cut separates, answer `3` with "These windows don't line up into zones", and nothing changes. It also answers `3` while zones are off or another window manager has the windows, and `1` for a number that isn't a screen. The same windows as for `screen.tile` are taken into account. Windows may overlap by up to 16 px, and each window's centre has to fall in a zone of its own.

`zones.quick` opens the quick layout panel on one screen, the one a Quick layout shortcut opens. The panel shows that screen's layouts as numbered thumbnails and takes focus, and each pick it makes is a `zones.pick`. It answers once the panel is open, and `1` for a number that isn't a screen and `3` when the desk isn't saved yet. With zones off, or another window manager in charge, it still opens and says so, since a pick is only saved.

`overview.show` opens the overview on every screen, the one the **Show overview** shortcut and the tray menu's **Overview** open, and answers at once; one already open stays as it is. Each screen shows the real app windows on it, the ones a capture would take, with a live DWM thumbnail, or the app's icon for a minimized window, one on another virtual desktop or one Windows has no thumbnail for. Windows on other virtual desktops are grouped under their desktop's name. Typing searches titles and app names, the arrows, Tab, Home and End pick a window, and Enter or a click goes to it, through the same focus path as a stack's cycle; a window on another desktop takes Windows to that desktop. Dragging a window onto a zone snaps it there, and onto another screen with no zones centres it there, both as a placement by hand: a window smart tiling arranged floats and its neighbours close the gap, and one dropped on a tiled screen floats over that zone. Windows on other desktops can't be dragged, since Windows' public desktop interface moves only an app's own windows, and neither can windows running as administrator; the overview says so in place of its hint. A drop the snapping can't make shows a notification. A window that isn't responding is listed with its icon and "Not responding", can be gone to but not dragged, and is asked nothing; every title is read from Windows' own copy, and app names and icons are read in short slices after the first paint, so no app can hold the overview or its Esc up. Esc always closes it, giving the focus back to the window that was in front while that still exists, and so does focus going to another app. Enter on a window that closed meanwhile closes it too. Its windows are tool windows of Seam's own, so they never show in Alt+Tab, and stacks, smart tiling and Step aside don't count them as the window in front. `config.patch` sets its shortcut as `hotkeys.overview` (no key by default). Assistants can't use it.

`config.patch` sets "Open the overview when the pointer rests in a corner" as `windows.corner_overview`: `{on, corner}`, where `on` defaults to `false` and `corner` is `"top_left"` (default), `"top_right"`, `"bottom_left"` or `"bottom_right"`; anything else reads as `"top_left"`. With it on, the pointer resting within a few pixels (4 at 100% scale) of that corner for 400 ms with no mouse button down opens the overview, as `overview.show` does, once until the pointer leaves the corner. Only outer corners of the desk count: a screen's corner where no other screen holds the pixel beside it or diagonally past it, so a corner two screens share, or where the pointer crosses to another screen, never does. Each screen whose corner is outer has its own. Nothing opens during a drag, while Seam is paused or steps aside, with a full-screen app in front, or while the overview is open. It uses a low-level mouse hook of its own on its own thread, in only while the setting is on and Seam isn't standing by, which never swallows input.

`zones.layouts` lists what one screen can be given, and answers from a snapshot, so it answers at once. `layouts` are the built-in layouts (`kind` `"template"`, with ids such as `grid` and `mainstack`), then the library layouts offered on that screen (`"custom"`, with their library id), then `{id: "none", name: "None", kind: "none"}`. `current` is the id of the layout the screen has now, `"none"` for no zones, and null for a tree of its own that isn't in the library. It answers `1` for a number that isn't a screen and `3` when the desk isn't saved yet.

`zones.apply` gives a screen one of those layouts, the way picking its tile on the Zones page does, so it takes the same time, moves the same windows and offers the same Undo. `layout` is matched against ids first and then names, ignoring case, among what `zones.layouts` lists for that screen; null is no zones. It takes no geometry, so a client can only choose from what the user already has. It answers `1` for a number that isn't a screen and `3` for a layout the screen isn't offered or a desk that isn't saved yet. `layout` has to be given, as a string or null, or it answers `-32602`. With zones off or another window manager in charge it only saves the layout, as a pick does. A pause doesn't stop it, since a pick is something you ask for.

`zones.pick` is what a layout tile on the Zones page does. It saves the screen's layout, and with zones on the windows `screen.tile` would take on that screen each get a zone of their own. Windows snapped in the old zones go first: the closest pair of a window's old zones and a free new zone is matched first, then the closest of the rest, so no two share a zone. Then the others, the focused one first, each take the free zone nearest their own middle. Windows past the last zone stay where they are. A notification offers Undo for 8 seconds, which gives the screen its old layout back and puts each moved window back where it was, in its old zones if it had them. Picking no zones (`layout` null) moves nothing. With zones off or another window manager in charge it only saves the layout. Picking the layout the screen already has does nothing.

### Custom layouts

`config.patch` can also set `custom_layouts`, the library of custom layouts shared by every desk and screen. Each entry is `{id, name, tree, screens}`: a short id, the name on its tile ("Custom 1" unless renamed), a split tree in the same form as a screen's layout, and optionally the device paths of the screens whose tabs offer it. With no `screens`, or an empty list, it's offered on any screen. A layout made in the panel or with `zones.from_windows` starts with the screen it was made on. A patch replaces the whole list, and an entry this build can't read is kept and ignored.

A screen's layout, `desks[sig].zones[screen]`, is `{template, tree}`. One given a library layout is `{template: "custom", custom: id, tree}`. When a patch changes a library layout's tree, Seam gives the new tree to every screen whose `custom` is that id, on every desk. When a patch deletes a library layout, those screens keep their tree and lose `custom`. A screen whose layout is changed on its own, by dragging a window's edge, `window.resize` or `window.priority`, keeps its tree and loses `custom` too. A second `window.priority` that puts the zones back links the screen again, if the library layout still has that tree. A patch that gives a screen a layout with no link has to say `"custom": null`, because a merge patch keeps keys it doesn't mention. A changed layout also has `from`, the id of the template (`"columns"`, `"grid"` and so on) or library layout it was cut from, which Seam sets when it first changes and keeps through further changes; a patch that gives a screen a picked layout has to say `"from": null`. Reset sizes reads it. An older Seam ignores `custom` and `from`, and reads the screen's tree as Custom.

### Focus and spotlight

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `window.focus_only` | `scope?`: `"screen"` (default) or `"workspace"` | `{ok: true, minimized}` or `{ok: true, restored}` | Any | 2, 3, -32602 | `{"method":"window.focus_only","params":{"scope":"screen"}}` |
| `window.spotlight` | `on?`: bool | `{on, resumes}` | Any | -32602 | `{"method":"window.spotlight","params":{"on":true}}` |

`window.focus_only` minimizes every other window on the focused window's screen. With `scope` `"workspace"` it minimizes every window outside the workspace in use, on all of the desk's screens, using the matching a workspace switch uses. Seam's own, administrator, hung, excluded, sticky and pinned-on-top windows are left alone. Calling it again while any window it minimized still exists restores those that are still minimized, bottom of the stack first, and gives focus back to the window that had it. Error `3` carries the reason: no window has focus, no other window to minimize, or no workspace in use. It also answers `3` while another window manager has Seam leaving placement alone. `window.spotlight` toggles when `on` is left out, and answers the new state. While Seam is paused or stepped aside for a full-screen app it stays off and answers `{on: false, resumes: true}` if it will come on when Seam resumes. It dims every screen except the focused window, and works next to another window manager. The state's `spotlight` says whether it is on.

### Pins

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `window.pin` | `kind`: `"top"` or `"sticky"`, `on?`: bool | `{on}` | Any | 2, 3 | `{"method":"window.pin","params":{"kind":"top"}}` |
| `wm.release` | none | `{ok: true, released}` | Any | 2 | `{"method":"wm.release"}` |

`window.pin` toggles when `on` is left out. `top` keeps the window above the others, `sticky` keeps it out of workspace switches. Error `3` covers no window, a Seam or administrator window, or a window that is already on top by itself. `wm.release` takes back every pin and border Seam set, and `released` counts the windows.

### Workspaces

Workspaces belong to a desk, and these methods act on the desk that is connected unless `desk` says otherwise.

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `workspaces.list` | `desk?`: a desk's `sig` | `[{id, name, hotkey, windows, zones, active, default}]` | Any | 1 | `{"method":"workspaces.list"}` |
| `workspace.switch` | `id` or `name`, `alongside?`: bool | `{ok: true}` | Any | 1, 3, `-32602` | `{"method":"workspace.switch","params":{"name":"Coding"}}` |
| `workspace.capture` | `name?`, `into?`: workspace id, `row?`: number, `hwnd?`: window handle with `exe` and `class` | `{id}` | Panel or Seam | 1, 2, 3, 4 | `{"method":"workspace.capture","params":{"name":"Review"}}` |
| `workspace.quick_save` | none | `{id, created}` | Any | 1, 2, 3 | `{"method":"workspace.quick_save"}` |
| `workspace.add_window` | `id` or `number`: the workspace's id, or its number from 1 in the desk's list; `hwnd?`: window handle | `{id, row, new}` | Any | 1, 2, 3, `-32602` | `{"method":"workspace.add_window","params":{"number":2}}` |
| `workspace.snap_back` | none | `{ok: true}` | Any | 1, 3 | `{"method":"workspace.snap_back"}` |
| `workspace.switch_back` | none | `{ok: true}` | Any | 1, 3 | `{"method":"workspace.switch_back"}` |
| `workspace.status` | `id` | `[{rule_index, state}]` | Any | 1 | `{"method":"workspace.status","params":{"id":"ws_7f3a"}}` |
| `workspace.update` | `id`, `workspace`: the whole workspace object | `{ok: true}` | Panel | 1, 3, 4 | `{"method":"workspace.update","params":{"id":"ws_7f3a","workspace":{...}}}` |
| `workspace.get` | `id` | `{id, name, show_all, other_windows, rows: [{row, app, title, screen, target, shown, launch}]}` | Any | 1 | `{"method":"workspace.get","params":{"id":"ws_7f3a"}}` |
| `workspace.create` | `name` | `{id, name}` | Panel or Assistant | 3, 4, 5 | `{"method":"workspace.create","params":{"name":"Review"}}` |
| `workspace.rename` | `id`, `name` | `{ok: true}` | Panel or Assistant | 1, 3, 4, 5 | `{"method":"workspace.rename","params":{"id":"ws_7f3a","name":"Mail"}}` |
| `workspace.edit_row` | `id`, `row`, and at least one of `target`, `shown`, `launch` (only `false`) | `{ok: true}` | Panel or Assistant | 1, 3, 4, 5 | `{"method":"workspace.edit_row","params":{"id":"ws_7f3a","row":0,"shown":"minimized"}}` |
| `workspace.remove_row` | `id`, `row` | `{ok: true}` | Panel or Assistant | 1, 3, 4, 5 | `{"method":"workspace.remove_row","params":{"id":"ws_7f3a","row":2}}` |
| `workspace.set_show_all` | `id`, `on` | `{ok: true}` | Panel or Assistant | 1, 3, 4, 5 | `{"method":"workspace.set_show_all","params":{"id":"ws_7f3a","on":true}}` |
| `workspace.set_default` | `id`: a workspace id, or `null` to clear | `{id}` | Panel or Seam | 1, 3, 4 | `{"method":"workspace.set_default","params":{"id":"ws_7f3a"}}` |
| `session.restore` | none | `{ok: true}` | Panel or Seam | 1, 3, 4 | `{"method":"session.restore"}` |

- `workspaces.list`: `hotkey` is the workspace's shortcut, or `null`. `windows` is how many apps it holds. `zones` maps a screen's device path to the layout a switch gives it, in the same form as a screen's layout on a desk, with `{"template": "none", "tree": "leaf"}` for no zones. A screen with no entry keeps its layout, and so does one whose entry this build can't read, or that isn't connected. `default` marks the one taken as in use at start when none is remembered. Names are matched case-insensitively by `workspace.switch`, and `id` wins when both are sent.
- `workspace.switch` answers as soon as the switch has started. When it ends you get a `workspace.switched` event, so wait for that if you need the result. Error `3` is "Couldn't start the switch". With `alongside` true (default false) the switch leaves the other windows alone this once, whatever the workspace's **Other windows** says. It doesn't change the saved setting. Before it places anything, a switch (Snap back and `alongside` too) saves the workspace's layouts for the connected screens, as a pick on Zones does but without moving snapped windows, and `state.changed` follows. A row whose zone the new layout doesn't have fills the screen's work area. `session.restore` changes no layout.
- `workspace.capture` saves the windows on screen into a new workspace, or into `into` (like **Update from screen**). With `into` alone and `workspaces_update_removes_closed` on in the config (off by default), the rows the capture has no window for are removed, and the tray shows a notification that undoes it; with `row` it never removes anything. Both record every screen of the desk in `zones`: its layout now, or no zones. With `row`, `zones` stays as it was. With `row` and `into`, only that row of the workspace is updated, from its open window, like **Where it is now**. `row` without `into` is `-32602`. Error `2` is "Already capturing", and `3` includes a row whose app isn't open. With `workspaces_remember_places` on in the config (off by default), a captured row of an Explorer window gets `place` `{"folder": "C:\\..."}` and one of a browser `{"url": "https://..."}`: only a folder on a fixed or removable drive that exists (no mapped drives), or an http or https page with no user name in it, and never for the last session. A page's whole query is dropped when any parameter's name, percent-decoded and lowercased, contains `token`, `secret`, `key`, `pass`, `sig`, `auth`, `code`, `sess`, `cred`, `otp`, `jwt`, `sid` or `state`, and its fragment when it holds `=`. An address bar showing no scheme gets `http://` for `localhost`, an IPv4 address or a dotless name with a port, `https://` for a name with a dot, and other dotless text isn't kept. The last session never gets `place`, and a last session restore opens none. Private windows are skipped on a best-effort basis, by words in the title, so one whose title doesn't say so may be saved. A capture that reads none keeps the row's saved `place`. With `hwnd`, `exe` and `class` as well as `into` and `row`, only that one window is read and the row takes nothing but its `place`, when the row still has that `exe` and `class`, has no `place` yet, and the window is of that app and class: this is what **Add an app** does for a window picked from Open now. It answers `3` when the setting is off or the window has no folder or page. While the setting is on, Seam remembers, in memory only, the window each row with a `place` was captured or added from, or that a switch started or first matched for it. A switch, Snap back, `workspace.status`, the drift check, auto-apply and `window.focus_only` then give such a row that window only, wherever it has browsed since, and no other row of the workspace takes it. A row not bound yet (after Seam restarts) matches by app once, but not a window another workspace's row holds. Once its window is closed the row gets none, so a switch starts the app there: a folder only for an Explorer row, a page only for a browser row. `config.get` shows `place` on rows. A workspace's Export and the settings export on General leave it out, and both kinds of import drop it. A capture never sets `open_with`, and a row it updates keeps the one it has.
- `workspace.quick_save` saves into the workspace in use, or into a new "Workspace N" that becomes the one in use (`created` is then true). Seam shows a notification that undoes it for 8 seconds.
- `workspace.add_window` adds the window in front, or `hwnd`, to one workspace of the desk, as **Add window to workspace 1** to **9** on Shortcuts does. Give `id` or `number`, not both. A window with no row there gets one at the end, with where it is now and **Launch** off; it never sets `open_with` and never turns launch on. A window that already has a row there updates that row (`new` is false): the row bound to that very window first, then the app's only row in the workspace (unless another open window is bound to it), then, when the app has several rows, the one on the same screen and in the same place. The row keeps its launch details, delay and order, and no row is added twice. It never removes the window from another workspace. It saves through the same checks as `workspace.update` and shows a notification that undoes it for 8 seconds. Error `1` is no such workspace or no workspace of that number, `2` a capture already running, and `3` a window Seam won't save, with the reason in `message`: Seam's own window, an app under **Left alone**, an app running as administrator, a window kept out of workspace switches, one with no taskbar button, or one that isn't on a screen of this desk. Any client may call it, the panel and Stream Deck included, but an assistant (`seam-mcp.exe`) can't, so it isn't on the list above.
- `workspace.snap_back` switches to the workspace in use again, so windows that moved or were minimized go back. Error `1` means no workspace is in use.
- `workspace.switch_back` switches to the workspace that was in use on this desk before the current one, as `workspace.switch` does, and a second call goes back again. Error `1` ("No workspace to switch back to") when there is none, or it was deleted.
- **Open with** is `open_with` on a workspace row: a path or link I typed for the app to open when a switch starts it. Only the panel can set it, through `workspace.update`, and it is never captured, never exported (a workspace export and the settings export leave it out) and dropped by both imports. `config.get` leaves it out for every client but the panel. It is checked again when a switch starts the app, and a refusal stops the app starting and shows in `workspace.switched` as `skipped` with the reason `open with refused`. A **path** is a full path on a drive letter, with no UNC, `..`, `/`, quote or control character, and it can't start with `-`. It must exist on a fixed or removable drive, and goes to the row's program (`safe_exe`, not on a network drive) as one quoted argument. A **link** has a scheme of `http`, `https`, `obsidian`, `vscode`, `vscode-insiders` or `spotify`, no space, quote, backslash or control character, a host for http and https, and loses any user name or password. It goes to the shell, which hands it to the app that handles the scheme, and that app may act on it (an `obsidian` or `vscode` link can open or change what it names). A path has no `%`, and a link has one only as a percent escape (`%` and two hex digits). A link has no invisible or direction-changing character (U+00AD, U+200B to U+200F, U+202A to U+202E, U+2060 to U+2064, U+2066 to U+2069, U+FEFF) and no Unicode whitespace. Neither is over 1024 characters. A path also has no `:` after the drive, no name ending in `.` or a space, and no DOS device name (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, `CONIN$`, `CONOUT$`) in any part. A path is refused when the row's program is a script host or launcher (cmd, powershell, powershell_ise, wscript, cscript, mshta, rundll32, regsvr32, msiexec, wsl, bash, conhost, wt, schtasks, forfiles, py, node, java, javaw, cmstp, msbuild, installutil, certutil, bitsadmin, ssh, perl, ruby, php, deno, bun, npx, and any name starting with python or pwsh; this blocks the common script hosts and is no guarantee that no program runs a path), when the row has an `aumid`, and for explorer.exe when it is a file; a link still works for all of them. It wins over a saved `place`, and an app that already has a window is never sent anywhere.
- `workspace.status` gives each app's state, by its row number from 0: `in_place`, `moved`, `minimized`, `not_running`, `background` (the app runs with no window, as one closed to the tray does) or `cant_move` (it runs as administrator). Error `1` when the id isn't on this desk.
- `workspace.get` reads one workspace of this desk from the snapshot. `app` is the program's file name, or its AUMID for a Store app, and `target`, `shown` and `launch` are as the workspace stores them. The program's path, its AUMID when there is a path, and a row's folder or page aren't included.
- `workspace.create`, `rename`, `edit_row`, `remove_row` and `set_show_all` are the small edits an assistant may make. The Seam window can send them too, but it has `workspace.update`. Any other client gets `4`, and an assistant gets `4` while **Let assistants change workspaces** is off and `5` while **Let assistants use Seam (MCP)** is off. Each is saved as `workspace.update` saves, with the same checks, so the zone of a `target` must be on its screen (`3`), and shows a notification that undoes it for 8 seconds, like a quick save. `create` makes an empty workspace named `name` (kept apart from the others by adding " 2"), not in use, and answers `{id, name}` with the name it got; `rename` answers `{ok, name}` the same way. A name is 1 to 64 characters, without control, bidi or zero-width characters. From an assistant, `create` answers `3` once the desk has 50 workspaces. Only the latest change can be undone from a notice, so a click on an older one undoes the newest. `edit_row` changes the row `row`, counted from 0, and nothing else about it: `target` is `{zone, span_to?}`, `{rect: [x, y, w, h]}` as fractions of the screen, or `{state: "max"}` or `{state: "keep"}`; `shown` is `normal`, `minimized`, `maximized` or `fullscreen`; `launch` can only be `false`, to stop Seam starting that app, and `true` is `-32602`. None of these can set a program, an AUMID, a shortcut, Other windows or a folder or page, and `workspace.update` is not open to an assistant. `remove_row` moves the rows after it up, and the windows Seam placed for them follow.
- `workspace.update` saves one edited workspace of the current desk, the way the editor does. The workspace's `show_all` (default false, "Show every window") makes a switch, Snap back and auto-apply restore a window saved minimized, onto its rect or maximized, and count it as moved while it's minimized. An older Seam ignores the field, and drops it when it saves the workspace. Without a `zones` key the workspace keeps the layouts it has, and with one, `zones` replaces them. A new or changed entry must be a layout of at most 128 zones nested at most 12 deep, and is stored as read, without fields Seam doesn't know. It answers `3` when a new program isn't safe to launch, a chosen zone isn't on its screen, or a layout can't be used. A row's `place` can be kept or removed, but one that no row of the same app (same `exe`, and same `class` when set) already had is dropped. A row's `open_with` (a path or link I typed, see **Open with** below) is checked by `safe_open_with` and stored as it is cleaned, and the call answers `3` with the reason when it is refused.
- `session.restore` restores the desk's last session through the workspace switch, with other windows left alone. Its result arrives as `workspace.switched` with `id` `last_session`. Error `1` when no session is saved for the desk.

### Updates and the app window

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `update.ready` | none | `{ok: true}` | Seam | 3, 4 | `{"method":"update.ready"}` |
| `panel.open` | `page?`: a page name | `{ok: true}` | Any | 3 | `{"method":"panel.open","params":{"page":"workspaces"}}` |

`update.ready` is how `seam-settings.exe` tells the tray that the updater has an update waiting, and it is not for other clients. The Store build answers `3`, since the Store updates Seam. `panel.open` opens the Seam window on one of `displays`, `zones`, `workspaces`, `shortcuts`, `general`, `help` or `about`. A window that is already open switches to that page and comes to the front. A new one opens centred on the screen the pointer is on. An unknown page opens a new window on its last page, and leaves an open one where it is. Error `3` means `seam-settings.exe` isn't next to `seam.exe`.

## Events

Subscribe with `events.subscribe` and a list of topics. An event belongs to the topic before the dot in its type, so `"workspace"` covers `workspace.switched` and `workspace.moved`. Events are sent on the connection that subscribed, as lines like the one shown under [Transport](#transport): `{"jsonrpc":"2.0","method":"event","params":{"type":...,"data":...}}`.

| Topic | `type` | `data` | When |
|---|---|---|---|
| `state` | `state.changed` | The full [State](#the-state-object) | Any change. At most about one per 100 ms |
| `workspace` | `workspace.switched` | `{id, name, launched, placed, skipped: [{app, reason}]}` | A switch ended. `launched` apps were started, `placed` windows were put in place, and `skipped` lists apps that couldn't be |
| `workspace` | `workspace.moved` | `{id}` | A window of that workspace was moved or minimized, or a window was put in place when it opened |
| `desk` | `desk.changed` | `{sig, name}` | Screens were connected or disconnected, so the desk changed |
| `update` | `update.ready` | `{version}` | An update is downloaded and ready. Not in the Store build |
| `lineup` | `lineup.finished` | `{saved: bool}` | Line Up closed, and whether the borders were saved |

`state.changed` is the easy way to stay current: take the first State from `state.get`, then replace it with each event's `data`.

## The state object

`state.get` returns this, and `state.changed` sends it whole. It is what Seam is doing now, not settings, and it isn't saved. Clients ignore fields they don't know.

```json
{
  "seam_version": "0.6.0",
  "packaged": false,
  "remap": "on",
  "zones": "on",
  "spotlight": false,
  "paused": false,
  "stepped_aside": false,
  "wm": {
    "other_wm": null,
    "manual_other_wm": false,
    "focus_border": true,
    "focus_color": "accent",
    "pin_color": "amber"
  },
  "desk": { "sig": "<signature>", "name": "Home desk" },
  "screens": [{ "number": 1, "id": "<device path>", "name": "PG32UCDM", "primary": true, "rect": { "x": 0, "y": 0, "w": 3840, "h": 2160 } }],
  "borders": [{ "a": 2, "b": 1, "side": "Bottom", "state": "lined_up", "glued": false, "not_on_desk": false }],
  "workspace": { "id": "ws_7f3a", "name": "Coding", "drifted": false },
  "session": { "saved_at": 1793890000, "windows": 6 },
  "restart_apps": true,
  "windows_cursor_easing": true,
  "hotkey_errors": [{ "action": "workspace:ws_9c21", "combo": "Ctrl+Shift+Alt+2" }],
  "last_align": { "ok": false, "message": "..." },
  "update": { "available": "0.6.1", "ready": true },
  "clients": ["panel", "streamdeck"],
  "stacks": [{ "screen": 1, "zone": 2, "windows": [264412, 198230], "top": 198230 }],
  "tiling": [{ "screen": 1, "on": true, "family": "smart", "windows": 3 }]
}
```

| Field | Meaning |
|---|---|
| `seam_version` | Seam's version, such as `0.6.0` |
| `packaged` | True for the Microsoft Store build |
| `remap` | The pointer remap: `on`, `paused`, or `off_no_borders` when no border is lined up on this desk |
| `zones` | `on` or `off`: whether drag snapping is enabled |
| `spotlight` | Spotlight is dimming everything but the focused window |
| `paused` | Seam is paused with `seam.pause`, the tray or its shortcut. It isn't saved |
| `stepped_aside` | A game or other full-screen app is in front, so Seam waits. Stays `false` while `paused` |
| `wm.other_wm` | `komorebi`, `glazewm` or `leopardwm` while that window manager is running, else `null`. Seam leaves placing windows to it |
| `wm.manual_other_wm` | "Another window manager handles my windows" is on |
| `wm.focus_border` | "Focus border" is on |
| `wm.focus_color`, `wm.pin_color` | The border colours as stored: `none`, a swatch name, or `#RRGGBB` |
| `desk` | The connected set of screens. `sig` is its stable signature, and `name` is what you called it |
| `screens` | One entry per connected screen. `number` is the Line Up number from 1, `id` the device path, `name` the model name, and `primary` whether it is the Windows main screen. `rect` is its place in the Windows arrangement, in pixels: `x` and `y` of its top left corner, then `w` and `h`. It is zero in an older Seam. `layout` is the id of the layout the screen has on this desk, as `zones.layouts` names it, so a client sees a change of layout as a `state.changed` |
| `borders` | The borders between screens, by Line Up number. `a` and `b` are the two screens, and `side` is which side of `a` the screen `b` is on (`Left`, `Right`, `Top` or `Bottom`). `state` is `lined_up` or `not_lined_up`. `glued` is true when the two screens touch on the desk across a gap in the Windows arrangement, and false when an older Seam leaves it out. `not_on_desk` is true when the two screens touch in Windows but not on the desk, so the pointer stops at that border, and false when an older Seam leaves it out |
| `workspace` | The workspace in use on this desk, or `null`. `drifted` is true once one of its windows was moved or minimized, so Snap back has work to do |
| `session` | The last session saved for this desk, or `null`. `saved_at` is Unix seconds and `windows` how many windows it holds |
| `restart_apps` | Windows' "Restart apps" sign-in option was on when Seam last looked |
| `windows_cursor_easing` | Windows' "Ease cursor movement between displays": `true`, `false`, or `null` on a Windows without the setting. Read from `CursorDeadzoneJumpingSetting` under `HKCU\Control Panel\Cursors` (see the README FAQ) |
| `hotkey_errors` | Shortcuts Seam couldn't register because another app holds them. `action` names the shortcut and `combo` is its keys |
| `last_align` | The result of the last change to Windows' arrangement, `{ok, message}`, or `null` |
| `update` | An update waiting to install, `{available, ready}`, or `null`. `ready` means it is downloaded and checked |
| `clients` | The names of the connected API clients. The settings window shows as `panel`, and an assistant's `seam-mcp.exe` as `seam-mcp (<assistant>)` |
| `tiling` | Smart tiling on each screen, for the virtual desktop in front. `screen` is the Line Up number, `on` whether it is on there, `family` how it arranges windows, and `windows` how many it tiles now, stacked ones included. Empty in an older Seam |
| `stacks` | Every stack of windows in zones on this desk. `screen` is the Line Up number, `zone` the zone from 1, and `span_to` the last zone when the stack spans several, left out otherwise. `windows` are the window handles in the order Next steps through them, and `top` the one on top. Empty in an older Seam |

## Error codes

| Code | Name | Meaning |
|---|---|---|
| `-32700` | Parse error | The line isn't JSON. `id` is `null` |
| `-32600` | Invalid request | Not a JSON-RPC 2.0 request, or `method` is missing |
| `-32601` | Method not found | No such method |
| `-32602` | Invalid params | A param is missing or the wrong type. `message` says which |
| `1` | not found | No such workspace, desk, screen, row or saved session |
| `2` | busy | Line Up or a capture is already running, a newer window command replaced this one, or Seam didn't answer within 60 seconds |
| `3` | refused | Seam understood and said no. `message` is a human reason, such as "No window in that direction" |
| `4` | forbidden | The method is only open to the Seam window or a Seam exe. See [Security model](#security-model) |
| `5` | disabled | **Let other apps control Seam** is off, or for `seam-mcp.exe`, **Let assistants use Seam (MCP)** is |

## Examples

Each one connects to the pipe, says hello, and then calls. They assume Seam is running. None of them checks the server's process, which the [security section](#security-model) explains.

### PowerShell

Connect, `hello`, `state.get`, and `workspace.switch`. It opens the pipe with identification-level security, which is the part of the server check that .NET offers.

```powershell
$name   = "seam-" + $env:USERNAME.ToLower()
$utf8   = New-Object System.Text.UTF8Encoding($false)
$pipe   = New-Object System.IO.Pipes.NamedPipeClientStream(
  ".", $name,
  [System.IO.Pipes.PipeDirection]::InOut,
  [System.IO.Pipes.PipeOptions]::None,
  [System.Security.Principal.TokenImpersonationLevel]::Identification)
$pipe.Connect(2000)   # throws if Seam isn't running or the pipe stays busy

$reader = New-Object System.IO.StreamReader($pipe, $utf8)
$writer = New-Object System.IO.StreamWriter($pipe, $utf8)
$writer.NewLine   = "`n"
$writer.AutoFlush = $true

$script:id = 0
function Invoke-Seam($method, $params = @{}) {
  $script:id++
  $request = @{ jsonrpc = "2.0"; id = $script:id; method = $method; params = $params }
  $writer.WriteLine(($request | ConvertTo-Json -Compress -Depth 10))
  while ($true) {
    $message = $reader.ReadLine() | ConvertFrom-Json
    if ($message.id -ne $script:id) { continue }   # an event, or a late answer
    if ($message.error) { throw "Seam error $($message.error.code): $($message.error.message)" }
    return $message.result
  }
}

Invoke-Seam "hello" @{ client = "powershell"; version = "1.0" }
$state = Invoke-Seam "state.get"
"Remap is $($state.remap) on desk '$($state.desk.name)'"
Invoke-Seam "workspace.switch" @{ name = "Coding" }

$pipe.Dispose()
```

`ReadLine` returns `$null` when Seam closes the connection, so a long-running script should stop when it sees that. A script that calls less often than every 30 seconds should connect again for each batch of calls.

### Node

This is the Stream Deck plugin's approach, simplified: one socket, a buffer split on newlines, and a map of calls waiting for their reply. The plugin's version, with reconnecting and timeouts, is `integrations/streamdeck/src/seam.ts`.

```js
import net from "node:net";
import os from "node:os";

const pipe = `\\\\.\\pipe\\seam-${(process.env.USERNAME ?? os.userInfo().username).toLowerCase()}`;
const socket = net.connect(pipe);
socket.setEncoding("utf8");

let nextId = 1;
const waiting = new Map();
let buffer = "";

socket.on("data", (chunk) => {
  buffer += chunk;
  let end;
  while ((end = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, end).trim();
    buffer = buffer.slice(end + 1);
    if (!line) continue;
    const message = JSON.parse(line);
    if (message.method === "event") {
      console.log("event", message.params.type, message.params.data);
    } else if (waiting.has(message.id)) {
      const { resolve, reject } = waiting.get(message.id);
      waiting.delete(message.id);
      message.error ? reject(new Error(`${message.error.code}: ${message.error.message}`)) : resolve(message.result);
    }
  }
});

function call(method, params = {}) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    socket.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
}

socket.once("connect", async () => {
  console.log(await call("hello", { client: "node-example", version: "1.0" }));
  const state = await call("state.get");
  console.log(`Remap is ${state.remap} on desk '${state.desk.name}'`);
  await call("workspace.switch", { name: "Coding" });
  socket.end();
});
socket.on("error", (e) => console.error("Seam isn't reachable:", e.message));
```

### Python

Python opens the pipe like a file. Use `buffering=0`, so each write goes out at once and nothing is held back. This blocking style suits request and reply. For events, use a connection of its own (see below), because a synchronous pipe handle can't write while another thread is reading it.

```python
import getpass, json, os

path = r"\\.\pipe\seam-" + os.environ.get("USERNAME", getpass.getuser()).lower()
pipe = open(path, "r+b", buffering=0)
next_id = 0

def call(method, **params):
    global next_id
    next_id += 1
    request = {"jsonrpc": "2.0", "id": next_id, "method": method, "params": params}
    pipe.write((json.dumps(request) + "\n").encode("utf-8"))
    while True:
        line = pipe.readline()
        if not line:
            raise ConnectionError("Seam closed the connection")
        message = json.loads(line)
        if message.get("id") != next_id:
            continue  # an event
        if "error" in message:
            raise RuntimeError(f"{message['error']['code']}: {message['error']['message']}")
        return message["result"]

print(call("hello", client="python-example", version="1.0"))
state = call("state.get")
print("Remap is", state["remap"], "on desk", state["desk"]["name"])
call("workspace.switch", name="Coding")
pipe.close()
```

`open` fails with `FileNotFoundError` when Seam isn't running. Python opens the pipe without identification-level security, so it has the same limit as the Node client.

### Subscribing to events

Open a connection, say hello, subscribe, and then do nothing but read lines. A subscribed connection is never dropped for being idle. Keep it separate from one you send calls on.

```python
import json, os

path = r"\\.\pipe\seam-" + os.environ["USERNAME"].lower()
events = open(path, "r+b", buffering=0)

def send(id, method, **params):
    request = {"jsonrpc": "2.0", "id": id, "method": method, "params": params}
    events.write((json.dumps(request) + "\n").encode("utf-8"))

send(1, "hello", client="event-example", version="1.0")
send(2, "events.subscribe", topics=["state", "workspace", "desk", "lineup", "update"])

while True:
    line = events.readline()
    if not line:
        break  # Seam quit
    message = json.loads(line)
    if message.get("method") == "event":
        kind, data = message["params"]["type"], message["params"]["data"]
        print(kind, json.dumps(data)[:120])
```

The first two lines out are the replies to `hello` and `events.subscribe`. After that, every line is an event.

## MCP

`seam-mcp.exe` lets an AI assistant that speaks the [Model Context Protocol](https://modelcontextprotocol.io) use Seam. It sits next to `seam.exe` (the installer and the zip both put it there), speaks newline-delimited JSON-RPC 2.0 over stdio, and turns each tool call into a call on this API. It is a separate program, so Seam itself stays as it was: no new port, and no network.

**Turning it on.** **Let assistants use Seam (MCP)** on the General page is off by default, and so is the switch under it, **Let assistants change workspaces**. Both take effect at once, with no restart, and **Let other apps control Seam** has to be on too. General shows whether an assistant is connected, and the path to `seam-mcp.exe` with a **Copy** button. While the first switch is off, Seam refuses `seam-mcp.exe` at `hello`, and every tool answers with an error result saying "Assistant access is off in Seam, General".

**Adding it to an assistant.** Give the assistant a stdio server that runs the path from General, with no arguments. For Claude Desktop that is an entry under `mcpServers` in its config file:

```json
{ "mcpServers": { "seam": { "command": "C:\\Users\\you\\AppData\\Local\\Programs\\Seam\\seam-mcp.exe" } } }
```

**What it speaks.** `initialize` (it answers with the client's `protocolVersion` when it knows it: 2025-11-25, 2025-06-18, 2025-03-26 or 2024-11-05, and its newest otherwise), `notifications/initialized`, `ping`, `tools/list` and `tools/call`. A tool answers with `content` text and `isError`. An unknown tool or a bad `arguments` is a JSON-RPC error. Nothing but protocol lines goes to stdout.

| Tool | Does | API method |
|---|---|---|
| `seam_state` | What Seam is doing now | `state.get` |
| `seam_workspaces` | The desk's workspaces | `workspaces.list` |
| `seam_workspace` | One workspace's rows | `workspace.get` |
| `seam_workspace_status` | How each app of a workspace is doing | `workspace.status` |
| `seam_switch_workspace` | Switch, always with `alongside: true` | `workspace.switch` |
| `seam_snap_back` | Put the windows of the workspace in use back | `workspace.snap_back` |
| `seam_switch_back` | Go back to the workspace before | `workspace.switch_back` |
| `seam_quick_save` | Save the windows on screen | `workspace.quick_save` |
| `seam_create_workspace` | Make an empty workspace | `workspace.create` |
| `seam_rename_workspace` | Rename one | `workspace.rename` |
| `seam_edit_row` | Change a row's place or how it is shown, or turn its launch off | `workspace.edit_row` |
| `seam_remove_row` | Remove a row | `workspace.remove_row` |
| `seam_set_show_all` | Show every window on or off | `workspace.set_show_all` |

`seam_quick_save` and the last five need **Let assistants change workspaces**, because a quick save adds a row for every window on screen. Snap back and Switch back from an assistant leave the other windows alone too, whatever the workspace's Other windows says. Each shows the notification that undoes the change, and Seam's log (with **Debug log** on) names the assistant that made it.

**What it can't do.** An assistant can't close a window, change a setting, set which program a workspace starts or its shortcut, turn launch on, or use anything not in the table, even if it talks to the pipe itself: Seam holds any `seam-mcp.exe` to the list in [Security model](#security-model), by the process, not by the name it gives. Switching can still start the apps a workspace lists that aren't running, because that is what the workspace says. `workspace.update` stays with the Seam window.

**Your own server.** The pipe is open to your own tools as always, and a server you write yourself follows the same [Transport](#transport) and [Lifecycle](#lifecycle). It is held only by **Let other apps control Seam**, and has the methods marked **Any**.

## Versioning

`hello` answers with `api_version` (1 today) and `seam_version`. `app.info` and the state's `seam_version` give the version too, and `app.info` adds `build` (a short commit id on CI builds, `dev` otherwise) and the process id. Send your own `version` in `hello`, which helps when you read Seam's log.

Changes within an API version are additive: new methods, new optional params, new fields in results, State and event data, and new events and topics. Seam ignores params it doesn't know, and a client should ignore fields it doesn't know. Something that would break a client, such as renaming a method or changing what a field means, would come with a new `api_version`, and I would say so in the [changelog](../CHANGELOG.md). Expect new methods and fields from release to release, and check `seam_version` before relying on one.
