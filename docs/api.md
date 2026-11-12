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

**Let other apps control Seam** on the General page of the Seam window switches the API. It is on by default, and General shows how many apps are connected.

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

`state.get` and `config.get` read a snapshot, so they answer at once even while Seam is busy. `config.patch` is validated: a patch that makes an invalid config, or comes from a different config version, answers `3` and changes nothing. `autoapply.set` is "Put apps in place when they open". `config.patch` can also set `app_rules`, the list behind "Where apps open": each entry is a workspace row with `exe` (or `aumid`), `monitor` (the screen's device path) and `target` (`{zone}`, `{state: "max"}` for the full screen). A patch replaces the whole list, and an entry this build can't read is kept and ignored. A screen's layout on a desk or in a workspace's `zones` with that entry's `custom` id takes the entry's new tree, and keeps its tree without the id when the entry is deleted. `config.patch` also sets the pointer's push past an edge: `pointer.edge_resistance_px` (0 to 100, 0 is off) holds the pointer at the edge to any other screen until it has pushed that many pixels past it, and `pointer.glue_push_px` (0 to 200, default 60) does the same at a glued edge. Larger values are read as the top of the range. `cursor_easing.set` only accepts `true`: Seam never turns Ease cursor movement off, and `false` answers `3`.

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
| `screen.tile` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true, tiled, left}` | Any | 1, 2, 3 | `{"method":"screen.tile","params":{"number":2}}` |
| `zones.from_windows` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true, id}` | Any | 1, 2, 3 | `{"method":"zones.from_windows","params":{"number":1}}` |
| `zones.quick` | `number?`: Line Up number, from 1; the screen under the pointer when left out | `{ok: true}` | Any | 1, 3 | `{"method":"zones.quick","params":{"number":2}}` |
| `zones.pick` | `number`: Line Up number, from 1; `layout`: a screen layout, or null for no zones | `{ok: true}` | Panel | 1, 2, 3, 4, `-32602` | `{"method":"zones.pick","params":{"number":1,"layout":null}}` |

Error `3` carries the reason in `message`: no window has focus, nothing in that direction, the window can't be moved, runs as administrator or isn't responding. `window.resize` and `window.priority` act on the zones of the focused window's screen and every window in them, and also answer `3` when the window isn't in a zone, when no divider beside it can move, or when one of the windows that would move can't be. A second `window.priority` on the same layout puts it back as it was. A changed layout is saved, as Custom unless it is back to its template. `window.move`, `window.swap`, `window.resize` and `window.priority` also answer `3` while komorebi, GlazeWM, LeopardWM or "Another window manager handles my windows" has Seam leaving placement alone. `window.focus` still works then. Error `2` means a newer window command replaced this one before it ran.

`window.share` moves the divider beside a window in a zone so the window gets `fraction` of the span of itself and the zones beside it along `axis`, as the right-click menu of a divider handle does. Of the dividers on its two sides along that axis, it takes the one towards the bigger neighbour. The windows on both sides follow, none goes below the smallest size its app allows, and the layout is saved as for `window.resize`, with the same errors. `window.ungroup` takes a window out of its zones: it gets its size from before it was snapped, centred on those zones, and the layout and the other windows stay. A window snapped before the tray started, or maximized before it was snapped, gets two thirds of its zones. It answers `3` when the window isn't in a zone, with zones off, for the same windows Seam can't move, and while another window manager has Seam leaving placement alone. Both act on `hwnd` when it is given, and on the focused window otherwise.

`screen.tile` puts the loose windows of one screen into its zones, up to nine, and answers how many it placed (`tiled`) and how many found no zone or were past the ninth (`left`). A window already in a zone keeps it. A screen with no layout is given one for the number of windows and keeps it. It answers `3` with "Nothing to tile" when there is nothing to place, and while zones are off or another window manager has the windows, and `1` for a number that isn't a screen. Pinned, sticky, minimized and administrator windows are left alone.

`zones.from_windows` is **Save from windows** on the Zones page. It turns where the windows on one screen are now into a layout with a zone for each: it looks for a straight cut across the screen with windows on both sides and none across it, side by side first, then does the same on each side. A cut within a few pixels of a quarter, a third, a half, two thirds or three quarters goes there. The layout is added to the library of custom layouts as "Custom N", the screen is given it, and each window snaps into its zone. It answers with the new layout's `id`. Windows that overlap, or that no straight cut separates, answer `3` with "These windows don't line up into zones", and nothing changes. It also answers `3` while zones are off or another window manager has the windows, and `1` for a number that isn't a screen. The same windows as for `screen.tile` are taken into account. Windows may overlap by up to 16 px, and each window's centre has to fall in a zone of its own.

`zones.quick` opens the quick layout panel on one screen, the one a Quick layout shortcut opens. The panel shows that screen's layouts as numbered thumbnails and takes focus, and each pick it makes is a `zones.pick`. It answers once the panel is open, and `1` for a number that isn't a screen and `3` when the desk isn't saved yet. With zones off, or another window manager in charge, it still opens and says so, since a pick is only saved.

`zones.pick` is what a layout tile on the Zones page does. It saves the screen's layout, and with zones on the windows `screen.tile` would take on that screen each get a zone of their own. Windows snapped in the old zones go first: the closest pair of a window's old zones and a free new zone is matched first, then the closest of the rest, so no two share a zone. Then the others, the focused one first, each take the free zone nearest their own middle. Windows past the last zone stay where they are. A notification offers Undo for 8 seconds, which gives the screen its old layout back and puts each moved window back where it was, in its old zones if it had them. Picking no zones (`layout` null) moves nothing. With zones off or another window manager in charge it only saves the layout. Picking the layout the screen already has does nothing.

### Custom layouts

`config.patch` can also set `custom_layouts`, the library of custom layouts shared by every desk and screen. Each entry is `{id, name, tree, screens}`: a short id, the name on its tile ("Custom 1" unless renamed), a split tree in the same form as a screen's layout, and optionally the device paths of the screens whose tabs offer it. With no `screens`, or an empty list, it's offered on any screen. A layout made in the panel or with `zones.from_windows` starts with the screen it was made on. A patch replaces the whole list, and an entry this build can't read is kept and ignored.

A screen's layout, `desks[sig].zones[screen]`, is `{template, tree}`. One given a library layout is `{template: "custom", custom: id, tree}`. When a patch changes a library layout's tree, Seam gives the new tree to every screen whose `custom` is that id, on every desk. When a patch deletes a library layout, those screens keep their tree and lose `custom`. A screen whose layout is changed on its own, by dragging a window's edge, `window.resize` or `window.priority`, keeps its tree and loses `custom` too. A second `window.priority` that puts the zones back links the screen again, if the library layout still has that tree. A patch that gives a screen a layout with no link has to say `"custom": null`, because a merge patch keeps keys it doesn't mention. An older Seam ignores `custom` and reads the screen's tree as Custom.

### Focus and spotlight

| Method | Params | Result | Who | Errors | Example |
|---|---|---|---|---|---|
| `window.focus_only` | `scope?`: `"screen"` (default) or `"workspace"` | `{ok: true, minimized}` or `{ok: true, restored}` | Any | 2, 3, -32602 | `{"method":"window.focus_only","params":{"scope":"screen"}}` |
| `window.spotlight` | `on?`: bool | `{on}` | Any | -32602 | `{"method":"window.spotlight","params":{"on":true}}` |

`window.focus_only` minimizes every other window on the focused window's screen. With `scope` `"workspace"` it minimizes every window outside the workspace in use, on all of the desk's screens, using the matching a workspace switch uses. Seam's own, administrator, hung, excluded, sticky and pinned-on-top windows are left alone. Calling it again while any window it minimized still exists restores those that are still minimized, bottom of the stack first, and gives focus back to the window that had it. Error `3` carries the reason: no window has focus, no other window to minimize, or no workspace in use. It also answers `3` while another window manager has Seam leaving placement alone. `window.spotlight` toggles when `on` is left out, and answers the new state. It dims every screen except the focused window, and works next to another window manager. The state's `spotlight` says whether it is on.

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
| `workspace.snap_back` | none | `{ok: true}` | Any | 1, 3 | `{"method":"workspace.snap_back"}` |
| `workspace.switch_back` | none | `{ok: true}` | Any | 1, 3 | `{"method":"workspace.switch_back"}` |
| `workspace.status` | `id` | `[{rule_index, state}]` | Any | 1 | `{"method":"workspace.status","params":{"id":"ws_7f3a"}}` |
| `workspace.update` | `id`, `workspace`: the whole workspace object | `{ok: true}` | Panel | 1, 3, 4 | `{"method":"workspace.update","params":{"id":"ws_7f3a","workspace":{...}}}` |
| `workspace.set_default` | `id`: a workspace id, or `null` to clear | `{id}` | Panel or Seam | 1, 3, 4 | `{"method":"workspace.set_default","params":{"id":"ws_7f3a"}}` |
| `session.restore` | none | `{ok: true}` | Panel or Seam | 1, 3, 4 | `{"method":"session.restore"}` |

- `workspaces.list`: `hotkey` is the workspace's shortcut, or `null`. `windows` is how many apps it holds. `zones` maps a screen's device path to the layout a switch gives it, in the same form as a screen's layout on a desk, with `{"template": "none", "tree": "leaf"}` for no zones. A screen with no entry keeps its layout, and so does one whose entry this build can't read, or that isn't connected. `default` marks the one taken as in use at start when none is remembered. Names are matched case-insensitively by `workspace.switch`, and `id` wins when both are sent.
- `workspace.switch` answers as soon as the switch has started. When it ends you get a `workspace.switched` event, so wait for that if you need the result. Error `3` is "Couldn't start the switch". With `alongside` true (default false) the switch leaves the other windows alone this once, whatever the workspace's **Other windows** says. It doesn't change the saved setting. Before it places anything, a switch (Snap back and `alongside` too) saves the workspace's layouts for the connected screens, as a pick on Zones does but without moving snapped windows, and `state.changed` follows. A row whose zone the new layout doesn't have fills the screen's work area. `session.restore` changes no layout.
- `workspace.capture` saves the windows on screen into a new workspace, or into `into` (like **Update from screen**). Both record every screen of the desk in `zones`: its layout now, or no zones. With `row`, `zones` stays as it was. With `row` and `into`, only that row of the workspace is updated, from its open window, like **Where it is now**. `row` without `into` is `-32602`. Error `2` is "Already capturing", and `3` includes a row whose app isn't open. With `workspaces_remember_places` on in the config (off by default), a captured row of an Explorer window gets `place` `{"folder": "C:\\..."}` and one of a browser `{"url": "https://..."}`: only a folder on a fixed or removable drive that exists (no mapped drives), or an http or https page with no user name in it, and never for the last session. A page's whole query is dropped when any parameter's name, percent-decoded and lowercased, contains `token`, `secret`, `key`, `pass`, `sig`, `auth`, `code`, `sess`, `cred`, `otp`, `jwt`, `sid` or `state`, and its fragment when it holds `=`. An address bar showing no scheme gets `http://` for `localhost`, an IPv4 address or a dotless name with a port, `https://` for a name with a dot, and other dotless text isn't kept. The last session never gets `place`, and a last session restore opens none. Private windows are skipped on a best-effort basis, by words in the title, so one whose title doesn't say so may be saved. A capture that reads none keeps the row's saved `place`. With `hwnd`, `exe` and `class` as well as `into` and `row`, only that one window is read and the row takes nothing but its `place`, when the row still has that `exe` and `class`, has no `place` yet, and the window is of that app and class: this is what **Add an app** does for a window picked from Open now. It answers `3` when the setting is off or the window has no folder or page. While the setting is on, Seam remembers, in memory only, the window each row with a `place` was captured or added from, or that a switch started or first matched for it. A switch, Snap back, `workspace.status`, the drift check, auto-apply and `window.focus_only` then give such a row that window only, wherever it has browsed since, and no other row of the workspace takes it. A row not bound yet (after Seam restarts) matches by app once, but not a window another workspace's row holds. Once its window is closed the row gets none, so a switch starts the app there: a folder only for an Explorer row, a page only for a browser row. `config.get` shows `place` on rows. A workspace's Export and the settings export on General leave it out, and both kinds of import drop it.
- `workspace.quick_save` saves into the workspace in use, or into a new "Workspace N" that becomes the one in use (`created` is then true). Seam shows a notification that undoes it for 8 seconds.
- `workspace.snap_back` switches to the workspace in use again, so windows that moved or were minimized go back. Error `1` means no workspace is in use.
- `workspace.switch_back` switches to the workspace that was in use on this desk before the current one, as `workspace.switch` does, and a second call goes back again. Error `1` ("No workspace to switch back to") when there is none, or it was deleted.
- `workspace.status` gives each app's state, by its row number from 0: `in_place`, `moved`, `minimized`, `not_running`, `background` (the app runs with no window, as one closed to the tray does) or `cant_move` (it runs as administrator). Error `1` when the id isn't on this desk.
- `workspace.update` saves one edited workspace of the current desk, the way the editor does. Without a `zones` key the workspace keeps the layouts it has, and with one, `zones` replaces them. A new or changed entry must be a layout of at most 128 zones nested at most 12 deep, and is stored as read, without fields Seam doesn't know. It answers `3` when a new program isn't safe to launch, a chosen zone isn't on its screen, or a layout can't be used. A row's `place` can be kept or removed, but one that no row of the same app (same `exe`, and same `class` when set) already had is dropped.
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
  "clients": ["panel", "streamdeck"]
}
```

| Field | Meaning |
|---|---|
| `seam_version` | Seam's version, such as `0.6.0` |
| `packaged` | True for the Microsoft Store build |
| `remap` | The pointer remap: `on`, `paused`, or `off_no_borders` when no border is lined up on this desk |
| `zones` | `on` or `off`: whether drag snapping is enabled |
| `spotlight` | Spotlight is dimming everything but the focused window |
| `wm.other_wm` | `komorebi`, `glazewm` or `leopardwm` while that window manager is running, else `null`. Seam leaves placing windows to it |
| `wm.manual_other_wm` | "Another window manager handles my windows" is on |
| `wm.focus_border` | "Focus border" is on |
| `wm.focus_color`, `wm.pin_color` | The border colours as stored: `none`, a swatch name, or `#RRGGBB` |
| `desk` | The connected set of screens. `sig` is its stable signature, and `name` is what you called it |
| `screens` | One entry per connected screen. `number` is the Line Up number from 1, `id` the device path, `name` the model name, and `primary` whether it is the Windows main screen. `rect` is its place in the Windows arrangement, in pixels: `x` and `y` of its top left corner, then `w` and `h`. It is zero in an older Seam |
| `borders` | The borders between screens, by Line Up number. `a` and `b` are the two screens, and `side` is which side of `a` the screen `b` is on (`Left`, `Right`, `Top` or `Bottom`). `state` is `lined_up` or `not_lined_up`. `glued` is true when the two screens touch on the desk across a gap in the Windows arrangement, and false when an older Seam leaves it out. `not_on_desk` is true when the two screens touch in Windows but not on the desk, so the pointer stops at that border, and false when an older Seam leaves it out |
| `workspace` | The workspace in use on this desk, or `null`. `drifted` is true once one of its windows was moved or minimized, so Snap back has work to do |
| `session` | The last session saved for this desk, or `null`. `saved_at` is Unix seconds and `windows` how many windows it holds |
| `restart_apps` | Windows' "Restart apps" sign-in option was on when Seam last looked |
| `windows_cursor_easing` | Windows' "Ease cursor movement between displays": `true`, `false`, or `null` on a Windows without the setting |
| `hotkey_errors` | Shortcuts Seam couldn't register because another app holds them. `action` names the shortcut and `combo` is its keys |
| `last_align` | The result of the last change to Windows' arrangement, `{ok, message}`, or `null` |
| `update` | An update waiting to install, `{available, ready}`, or `null`. `ready` means it is downloaded and checked |
| `clients` | The names of the connected API clients. The settings window shows as `panel` |

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
| `5` | disabled | **Let other apps control Seam** is off |

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

An MCP server for Seam isn't built yet. It is something I plan to add on top of this API, so an AI assistant could switch workspaces or line things up the way a Stream Deck key does. There is no date. Follow the [issues](https://github.com/F3NN3X/seam/issues) and the [changelog](../CHANGELOG.md).

You don't have to wait, because the API is enough to wrap yourself. The shape is simple:

1. Write a small MCP server (the official SDKs are on [modelcontextprotocol.io](https://modelcontextprotocol.io)). It runs as a local process, started by your MCP client, on the same Windows account as Seam.
2. Keep one pipe connection to Seam inside it, the way the Node example does. Call `hello` once, and connect again if the pipe closes or a call finds it gone.
3. Turn each Seam method you want to expose into an MCP tool, with a name, a description and an input schema. `state.get` and `workspaces.list` make good read tools, and `workspace.switch`, `workspace.snap_back` and `window.move` good action tools.
4. Keep the tool set small and think about what you allow: an assistant that can switch workspaces can also close other windows through a workspace's Close setting. Leave out what you wouldn't press yourself.

An outline in TypeScript. It is a sketch, so check the names against the SDK's own docs:

```ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { call } from "./seam.js"; // the pipe client from the Node example

const server = new McpServer({ name: "seam", version: "0.1.0" });

server.registerTool(
  "seam_state",
  { description: "What Seam is doing: remap, zones, desk, active workspace" },
  async () => ({ content: [{ type: "text", text: JSON.stringify(await call("state.get")) }] }),
);

server.registerTool(
  "seam_switch_workspace",
  { description: "Switch to a workspace on the current desk", inputSchema: { name: z.string() } },
  async ({ name }) => {
    await call("workspace.switch", { name });
    return { content: [{ type: "text", text: `Switching to ${name}` }] };
  },
);

await server.connect(new StdioServerTransport());
```

## Versioning

`hello` answers with `api_version` (1 today) and `seam_version`. `app.info` and the state's `seam_version` give the version too, and `app.info` adds `build` (a short commit id on CI builds, `dev` otherwise) and the process id. Send your own `version` in `hello`, which helps when you read Seam's log.

Changes within an API version are additive: new methods, new optional params, new fields in results, State and event data, and new events and topics. Seam ignores params it doesn't know, and a client should ignore fields it doesn't know. Something that would break a client, such as renaming a method or changing what a field means, would come with a new `api_version`, and I would say so in the [changelog](../CHANGELOG.md). Expect new methods and fields from release to release, and check `seam_version` before relying on one.
