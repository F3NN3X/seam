//! What the Help page says, kept as data so search can find it. `**` marks the
//! exact label of something in Seam, which shows in bold.

use crate::panel::Page;

pub enum Block {
    Text(&'static str),
    List(&'static [&'static str]),
    /// A step, with a button that opens the page that does it.
    Step {
        title: &'static str,
        text: &'static str,
        page: Page,
    },
}

pub struct Topic {
    pub title: &'static str,
    pub blocks: &'static [Block],
    /// The page the button under the text opens.
    pub opens: Option<Page>,
}

impl Topic {
    /// Whether its title or text holds `query`, which is already lower case.
    pub fn mentions(&self, query: &str) -> bool {
        let has = |text: &str| plain(text).to_lowercase().contains(query);
        has(self.title)
            || self.blocks.iter().any(|block| match block {
                Block::Text(text) => has(text),
                Block::List(items) => items.iter().any(|item| has(item)),
                Block::Step { title, text, .. } => has(title) || has(text),
            })
    }
}

/// The text without its bold marks.
pub fn plain(text: &str) -> String {
    text.replace("**", "")
}

use Block::{List, Step, Text};

pub const TOPICS: &[Topic] = &[
    Topic {
        title: "Getting started",
        blocks: &[
            Text(
                "Seam makes the pointer cross where your screens physically meet, and puts your windows where you want them. Three steps get you there.",
            ),
            Step {
                title: "Line up your screens",
                text: "Drag two lines across each border until the stripes run straight over the bezel",
                page: Page::Displays,
            },
            Step {
                title: "Pick zone layouts",
                text: "Choose Columns, Rows, Grid, Priority, Main + stack or Stack + main for each screen",
                page: Page::Zones,
            },
            Step {
                title: "Capture a workspace",
                text: "Arrange your windows, then save which app is where",
                page: Page::Workspaces,
            },
        ],
        opens: None,
    },
    Topic {
        title: "Pointer and borders",
        blocks: &[
            Text(
                "Windows keeps the same pixel row when the pointer crosses to another screen, so between screens of different sizes it jumps. Seam puts it where the screens physically meet instead.",
            ),
            Text(
                "Press **Line up screens** on the Displays page, or Ctrl+Shift+Alt+L. Every screen shows its number, and for each border:",
            ),
            List(&[
                "Grab a line to move just that line. This sets the scale.",
                "Drag anywhere else to move both lines. This sets the height.",
                "Arrow keys nudge the selected line, Tab switches lines and R goes back to the guess from your screens' sizes",
                "Enter goes to the next border or finishes, Backspace goes back and Esc cancels",
            ]),
            Text(
                "When you finish, Seam saves the borders and turns on **Remap pointer at borders**. Ctrl+Shift+Alt+P pauses and resumes it. With **Also align Windows arrangement** on, Seam also moves your screens in Display settings to match.",
            ),
            Text(
                "Each set of screens is a desk, with its own borders, zones and workspaces. The desk menu under the page title lists them, with when each was last here, in your local time. It has **Rename this desk** and **Forget desks not seen in 30 days**.",
            ),
            Text(
                "**Edge resistance**, on Displays, holds the pointer at the edge to another screen until you push that many pixels past it, so you can grab a window's edge or corner there. It's off at 0, works at every border, and gives way while a mouse button is down, so dragging a window across is never slowed. It stays off while the remap is paused and during Line Up.",
            ),
            Text(
                "Seam relies on Windows' **Ease cursor movement between displays**, so the pointer can always leave a screen toward its neighbour. When it's off, the Displays page says so, and **Turn it on** turns it on for you and checks that Windows kept it. Seam never turns it off.",
            ),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Line Up aids",
        blocks: &[
            Text(
                "Each step names both screens with their models, such as \"Line up screen 3 (HDMI) with screen 4 (U3477)\", and says which way to move each line, such as straight below or level with the line on the reference screen.",
            ),
            List(&[
                "A band of faint stripes runs along the shared edge on both screens, 10 mm apart on your desk. If they run on straight across the edge, the screens are lined up. If they step, the offset is off, and if the spacing changes, the scale is off.",
                "A screen that reports no size gets no stripes",
                "With **Type measurements in Line Up** on, on the Displays page, M types the offset instead of dragging. It asks how many cm one screen's top or left edge is below or to the right of the other's, and moves the lines to match. Tab flips the direction, Enter applies and Esc closes the field.",
                "Typing needs both screens to report their size",
            ]),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Glue and Not on my desk",
        blocks: &[
            Text("Windows doesn't always have your screens as they stand on your desk. Line Up handles two cases."),
            Text(
                "**Glue** is for screens that touch on your desk but have a gap in Windows. Line Up offers each pair after the touching borders. Press G to glue it, then line the two up as they stand on your desk. The gap doesn't matter. A pair found from the screens' sizes starts glued, and a pair found only from the pixel layout starts off, so Enter skips it.",
            ),
            List(&[
                "The pointer rests at a glued edge until you push past it, 60 px unless you change **Glued edge push** on Displays, so an auto-hide taskbar there still shows. A quick flick crosses at once.",
                "Past the end of a glue, the edge is a dead edge, and Seam holds the pointer there",
                "**Match Windows to this layout** keeps glued screens apart, and refuses a move that would break a glue",
            ]),
            Text(
                "**Not on my desk** is for two screens that touch in Windows but not on your desk. Press N in Line Up, or choose **Not on my desk** in the border's **...** menu on Displays. The pointer stops at that edge both ways, or crosses to a glued screen past it. **On my desk** takes it back.",
            ),
            Text("Displays describes these borders as glued or \"Not on your desk\"."),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Layout views",
        blocks: &[
            Text(
                "The **Physical layout** card on Displays shows your screens two ways. Switch between them above the map.",
            ),
            List(&[
                "**Seam** is your desk, at the screens' real sizes. A lined up border is a solid line, a glued one is dashed and one that isn't on your desk is dotted.",
                "**Windows** is the arrangement as Windows has it, in pixels. A gap shows a note such as \"18 px gap\".",
            ]),
            Text(
                "Click a screen to flash its number on it. **Show on screens** draws the chosen view on every real screen for a few seconds, with each screen's number and model and a band along each edge that has a neighbour. Esc or a click closes it.",
            ),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Zones",
        blocks: &[
            Text(
                "Pick a layout for each screen: Columns, Rows, Grid, Priority, Main + stack or Stack + main. Then hold Shift while you drag a window. The zones show on the screen under the pointer, and letting go drops the window into the one that's lit.",
            ),
            List(&[
                "Hold Ctrl as well to cover several zones with one window",
                "**Show zones while dragging** and **Span zones** change the two keys",
                "**Gap between zones** sets the space between zones and the screen edge",
                "**Restore size when dragged out** gives a window its old size back when you drop it outside a zone",
                "**Swap with the window already in a zone** makes a window you drop on a held zone trade places with it; off lets them overlap",
            ]),
            Text(
                "While zones show, the lit zone is where the window will land, and thin grey outlines show where any window it pushes aside will go. Until you let go:",
            ),
            List(&[
                "**Scroll the mouse wheel** to step through the whole zone, 1/2, 1/3, 2/3, the zone with its neighbour, and the whole screen. It stops at either end. A share sits on the side of the zone nearest the pointer and follows it",
                "**Right-click** drops the window where you let go, without a zone",
                "**Tab** steps through the zone, the zone with its neighbour on the side you lean to, and the whole screen. **Shift+Tab** goes back, unless Shift is your show key. With Alt held, Tab is Windows' Alt+Tab",
                "**1**, **2** or **3** give the window a half, a third or two thirds of it, and **0** all of it. The rest becomes a zone of its own, and a window that was there moves into it. The layout is saved as **Changed**, and a notification offers Undo for 8 seconds",
                "The **arrows** pick the side the part sits on, until the pointer moves to another zone",
                "**Esc** drops the window where you let go, without a zone",
            ]),
            Text(
                "Hold Ctrl as you let go outside the zones to take a window out of the layout; the others close the gap. It works for a snapped window that is alone in its zone, and a notification offers Undo for 8 seconds.",
            ),
            Text(
                "Other keys, clicks and moves reach your apps as usual. **Pause**, in **Show zones while dragging**, shows the zones when you hold the pointer still for a moment while dragging, with no key. **Show keys while dragging** puts a strip on the screen that names the wheel, right-click and these keys.",
            ),
            Text(
                "Drag the edge of a window in a zone and the windows in the zones on the other side follow it. No window goes below the smallest size its app allows: the edge beyond it moves instead. When you let go, the window fits its zone again and the screen's layout shows as **Changed**. Esc puts everything back. **Resize neighbours with a window** turns this off.",
            ),
            Text(
                "Between two snapped windows, the gap has a handle you can't see until the pointer is on it, then a slim bar shows. Drag it and the windows on both sides follow, with the same smallest sizes, and the layout is saved as **Changed** when you let go. Esc puts everything back. Right-click it to give the left or top window 1/2, 1/3, 2/3, 1/4 or 3/4 of the two, or to take one of them out: it leaves its zones with its old size, and the others stay. A gap under 4 px has no handles, so **Size menu** on Shortcuts opens the same choices for the focused window. It has no key until you set one. **Handles between snapped windows** turns the handles off.",
            ),
            Text(
                "**Snap beside a window**: while zones show, or once the pointer rests with **Pause**, point at the edge of another window and the outline shows the dragged window taking half of that window's place on that side, with the other window in the rest. The wheel, or 1, 2 and 3, change the share to 1/2, 1/3 or 2/3. On a screen with zones, the drop splits the zone that window holds, the layout shows as **Changed**, and a notification offers Undo for 8 seconds. On a screen with no zones, the two windows become a group: dragging the edge between them resizes both, the gap has a handle, and dropping a third window beside one of them adds it. A window you drag away, minimize, maximize or close leaves the group. Groups last until Seam restarts. The middle of a held zone still swaps. Turn off **Snap beside a window** on Zones to drop over windows as before.",
            ),
            Text(
                "**Stacks** put several windows in one zone, one on top of the others. Press **S** while zones show in a drag, and the drop stacks the window on the one in the zone instead of swapping with them; the strip says so, and S again goes back to swapping. While a window of a stack is the one you're using, a slim row of bars at the bottom of its zone shows how many windows the stack has, the one on top brighter. **Next window in stack** and **Previous window in stack** on Shortcuts put another one on top, and a window you click, or pick with Alt+Tab or the taskbar, is on top too. The others stay in the zone behind it. A notification offers Undo for 8 seconds after a drop makes a stack. A window you drag out, close, minimize, maximize or move or swap to another zone with a key leaves its stack, and a swap by dragging, an edge or a handle moves and resizes the whole stack. Windows behind the top are only behind it, so one smaller than the top's zone, or a top that can't fill it, leaves them showing round its edges. A workspace keeps its stacks, and a restart of Seam finds them again.",
            ),
            Text(
                "**Arrange new windows automatically** on Zones turns smart tiling on for the chosen screen, on the virtual desktop you are on. It needs zones on, and is off on every screen and desktop until you turn it on. Each window that opens there takes a share of the screen: with **Smart**, the largest window's space is split along its longer side; **Columns**, **Main + stack** and **Grid** share the screen evenly in that shape. A window that can't shrink to its share joins a stack in that space, and takes a share of its own once there is room. A window you close leaves a gap the others fill. A minimized window's space waits a moment, then closes too, and restoring it gives it its space back, or a share; windows minimized together, as by Win+D, keep their spaces until they come back, or for 30 seconds with none back. A window a workspace switch minimizes gives its space up a moment after the switch, as one you minimize does. While Seam is paused, a full-screen app is in front, zones are off or a workspace switches, nothing moves, and the gaps close afterwards. Dialogs, small helper windows, windows that can't be resized, apps under **Left alone** and windows a workspace or an app rule places are never tiled, nor is an app whose rule has **Leave out of smart tiling**. A tiled window you saved in a workspace is tiled again when you restore it after minimizing. Dragging works as with any zones: drop in the middle of a window to swap, or press S to stack; drop on its edge to split its space; drag a gap's handle to resize both sides. A tiled window you drag away floats where you drop it, and the others fill its space. One you move with Win+arrow keys or resize by hand, one its app moves, and one a workspace switch moves, floats where it is and is never moved back, and the others fill its space too. A maximized one keeps its space. Main + stack keeps the main window's share when you drag its handle; Columns and Grid share the screen evenly again when a window opens or closes. **Float or tile this window** on Shortcuts takes the focused window out of tiling, or puts it back in. **Free all windows** in the tray menu, or its shortcut, turns tiling off on the screen and puts every window back where it was before it was tiled. A notification offers Undo for 8 seconds: it turns tiling on again and puts the windows you haven't moved since back in their places. Each virtual desktop keeps its own arrangement, and deleting one ends its arrangement, and a restart of Seam tiles the windows again. **Save from windows** keeps an arrangement as a layout.",
            ),
            Text(
                "**Scrolling**, under smart tiling on Zones, makes each new window a column on a strip that can be wider than the screen. A new column is half the screen wide and opens after the focused one, and the screen shows whole columns from one column's left edge. The columns out of view wait outside every screen, so none ever shows on a screen next to it; they are still in Alt+Tab, the taskbar and the overview, and the one you go to scrolls into view. **Scroll left** and **Scroll right** on Shortcuts focus the next column and scroll to it, **Move column left** and **Move column right** swap the focused window's column with its neighbour, and **Column width** gives it the next width: a third, a half, two thirds or the full screen. With Win held, the mouse wheel over a screen that scrolls on the desktop you are on scrolls it a column at a time. A drag, the gap handles and the Float key work as on any tiled screen, and a handle drag gives a column the nearest of those widths. An app that moves a waiting window back onto a screen leaves the strip and floats there. While Seam is paused, steps aside or has zones off, when a screen is plugged in or out, and when Scrolling turns off or Seam quits, every waiting window comes back onto its screen. If Seam crashes, the waiting windows come back the next time Seam starts. A strip holds up to 32 columns, and a window that opens or is dropped past that floats. Rotate windows and Reset sizes don't apply to a Scrolling screen.",
            ),
            Text(
                "**Put windows back when Seam quits**, under smart tiling on Zones, is off until you turn it on. With it on, **Quit Seam** in the tray menu, or a quit from the Seam window, frees every tiled screen on every virtual desktop first, as **Free all windows** does, and waits a moment for the windows to move. An update, a restart Seam did for you, a crash, signing out or shutting down keeps the arrangement, as it does for pinned windows.",
            ),
            Text(
                "**Rotate windows** on Shortcuts moves every window snapped in the zones of the screen under the pointer to the next zone in zone order, and the last to the first; the same key with Shift added goes the other way. A stack goes as one. Windows that float, pinned and sticky windows, windows of apps running as administrator stay where they are, and a zone one of them holds is stepped over. A minimized window, or one on another virtual desktop, keeps its zone for when it is back, so nothing rotates into it. Empty zones are open steps. A tiled screen rotates its tiles the same way, and a window you moved by hand floats first. **Reset sizes** puts a screen whose layout shows **Changed** back to the layout it came from, with that layout's proportions, and its windows follow; a tiled screen's splits even out, and Main + stack goes back to its default main share. Seam finds the layout by how it is cut, so a layout with a zone added or removed can't be reset. Both offer Undo for 8 seconds, and neither has a key until you set one.",
            ),
            Text(
                "**Fit group to screen** stretches the focused window's group to fill its screen, keeping each window's share and order. A window in zones already fills its screen, and a window in no group takes the whole work area without being maximized. A notification offers Undo for 8 seconds. The shortcut has no key until you set one.",
            ),
            Text(
                "**Edit layout** opens the editor on the screen itself. Click a zone to split it side by side or top and bottom, drag a divider, or merge two neighbours, then press **Done**. Esc throws the changes away and Ctrl+Z undoes the last one.",
            ),
            Text(
                "A layout you make is for the screen you made it on, and that screen's tab shows it under **Your layouts**. **Used on...** in its **...** menu ties it to other screens or to any screen, and **Show all screens** lists every layout, each tied one saying which screen it's for.",
            ),
            Text(
                "Your own layouts are kept in a library that every desk and screen shares, each with a tile under the built-in ones. Click one to give it to the screen: the windows on it move into the new zones, one to a zone and snapped ones first, and when any moved, a notification offers Undo for 8 seconds. Or use its **...** menu to edit, rename, duplicate or delete it. Editing one changes every screen that uses it, and deleting one leaves those screens with their zones. **New layout** starts one from a single zone, and **Edit layout** on a template saves what you make as a new one, leaving the template as it was.",
            ),
            Text(
                "**Save from windows** makes a library layout from where the windows on the screen are now, a zone for each, and snaps them in. They need to line up: no overlaps, and straight lines across the screen that separate them. A layout that is none of the tiles shows as **Changed**, and **Save as layout** keeps it in the library.",
            ),
            Text(
                "**Screen margins** keep room free on each side of the chosen screen, in pixels at 100% scale: they grow with the screen's scaling, and no side takes more than a quarter of the screen. Zones, Tile and Fit group, Quick layout, smart tiling, windows snapped beside each other and the overview's drop targets all stay inside what is left. Maximized and full-screen windows, Line Up and the pointer are not affected, and neither are the places a workspace saves. Each screen on each desk has its own margins, and windows smart tiling holds move when you let go of a slider; others move the next time they are snapped.",
            ),
            Text(
                "**Keep windows in their zones** is experimental. It moves snapped windows back after a screen, resolution or scaling change, or after the PC wakes.",
            ),
            Text(
                "**Tile this screen's windows** puts the loose windows on a screen into its zones in one go: the one you are using gets the biggest zone, and the others take the nearest free one. A screen with no layout is given one that fits the number of windows, from two columns up to a 3x3 grid. It takes up to nine windows and leaves pinned, sticky, minimized and administrator windows alone. Tile windows in the tray menu does it for a screen too, and a notification offers Undo for 8 seconds. The shortcut has no key until you set one.",
            ),
            Text(
                "**Quick layout** opens a small panel on the screen under the pointer with that screen's layouts as numbered thumbnails: the built-in ones, the custom layouts made for it, then None. Press 1 to 9 to pick one, or use the arrows and Enter (the arrows scroll past nine). Ctrl+Z undoes the last pick made there, Esc or a click outside closes it, and the panel stays open after a pick so you can try another. A pick moves the windows on that screen as clicking a tile does. With zones off the panel says so, and a pick is still saved. The shortcut has no key until you set one.",
            ),
            Text("Layouts belong to the desk, so each set of screens keeps its own."),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "Moving windows",
        blocks: &[
            Text("Left means left on your desk, as the screens physically stand, not in Windows' pixel layout."),
            List(&[
                "Shift+Win+Alt+arrows move the focused window. A window in a zone goes to the next zone that way, then onto the screen next to it.",
                "Ctrl+Shift+Alt+arrows focus the nearest window that way",
                "Swap trades places with the nearest window that way. It has no key until you set one.",
            ]),
            Text(
                "A window in a zone can take room from its neighbours. **Grow window into its neighbour** and **Shrink window** move the divider beside it, towards the bigger neighbour first. **Give window two thirds of its group** makes it the main window on its screen, and pressing it again puts the zones back. The windows beside it follow, and no window goes below the smallest size its app allows. A changed layout shows as Custom on the Zones page. These have no key until you set one.",
            ),
            Text(
                "**When a window moves to another screen**, on the Zones page, sets the size a window gets there: **Let Windows scale it**, **Keep its pixel size** or **Keep its real size**. **Also when I drag a window to another screen** does the same for a window you drop with the mouse.",
            ),
            Text(
                "**Move and resize from anywhere in a window**, on the Zones page, lets you hold Alt and drag anywhere in a window to move it, or right-drag to resize it from the nearest edge or corner. A maximized window comes back to its normal size under the pointer. Hold the show key as well and the window snaps into zones as with any drag. Some apps use Alt+click themselves, such as VS Code for an extra cursor, so you can pick Win or Ctrl+Alt as the key instead. Full-screen apps, excluded apps and apps running as administrator get the click as usual.",
            ),
            Text(
                "**Show overview** fills every screen with the windows on it: a live preview of each, or its app's icon when Windows has no preview to give, as for a minimized window or one on another virtual desktop. Windows on other desktops show under that desktop's name. Type to search by title or app, use the arrows, Tab or the mouse to pick one, and press Enter or click it to go to it; a window on another desktop takes you to that desktop. Drag a window onto a zone, or onto another screen with no zones, to move it there. On a screen smart tiling arranges, the window floats over that zone. Windows only lets an app move its own windows between desktops, so the overview can't. Esc closes it and goes back to the window you were using. Open it from the tray menu too. The shortcut has no key until you set one.",
            ),
            Text(
                "**Open the overview when the pointer rests in a corner**, in Windows on the Zones page, is off until you turn it on. Pick the corner beside it: top left, top right, bottom left or bottom right. Push the pointer into that corner of the whole desk and leave it there a moment, with no mouse button down, and the overview opens. It opens once per visit: move away and back to open it again. Only a corner on the outside of the desk counts, never one where two screens meet, so on two screens side by side, top left is the left screen's. Nothing opens during a drag, while Seam is paused or steps aside, with a full-screen app in front, or while the overview is already open.",
            ),
            Text(
                "In **Show overview**, Ctrl+Tab and Page Down go to the first window of the next virtual desktop's group, and Ctrl+Shift+Tab and Page Up to the previous one, wrapping around. The mouse wheel over a group's label does the same. With a search typed, only groups with a match count.",
            ),
            Text(
                "Seam leaves alone the windows of apps running as administrator, windows that aren't responding, and its own.",
            ),
            Text(
                "If komorebi, GlazeWM or LeopardWM is running, Seam leaves placing windows to it. **Another window manager handles my windows** does the same for any other one.",
            ),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Focus and spotlight",
        blocks: &[
            List(&[
                "**Focus on this window** minimizes the other windows on its screen, and pressing it again brings them back. Windows pinned on top or kept out of workspace switches stay, and so do the zones' excluded apps.",
                "**Spotlight** dims everything but the window you're using, on every screen. Press it again to turn it off. With the desktop or taskbar in front, the dim is hidden.",
            ]),
            Text(
                "Neither has a key until you set one on the Shortcuts page. Focus on this window leaves placing windows to another window manager if one is running, and Spotlight keeps working next to it.",
            ),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Apps running as administrator",
        blocks: &[
            Text(
                "Windows doesn't let an app move the windows of an app that runs with more rights. That's why an elevated app's window can't move with Seam.",
            ),
            List(&[
                "To manage them, quit Seam and start it with Run as administrator. Seam then manages both, but its Start with Windows entry still starts it normally.",
                "Or leave those windows alone. The pointer remap and Line Up work either way.",
            ]),
        ],
        opens: None,
    },
    Topic {
        title: "Pins and borders",
        blocks: &[
            List(&[
                "**Pin window on top** keeps the focused window above the others until you press it again",
                "**Keep window out of workspace switches** makes it sticky: a switch never moves, minimizes or closes it, and capture leaves it out",
                "**Release pinned windows**, in the tray menu, takes back every pin and border Seam set. Quitting Seam does the same.",
            ]),
            Text("The pin keys have no key until you set one on the Shortcuts page."),
            Text(
                "**Focus border** colours the border of the window that has focus. A pinned window gets an amber border, which wins over the focus border. Both colours are picked from the swatches on the Zones page, with **None** for no border and **Custom colour** for any other.",
            ),
            Text(
                "**Tell me when a pinned window closes** tells you after the app closes. Click the notification to start it again and pin its new window.",
            ),
            Text(
                "**Pinned windows stay when I switch workspaces** makes a switch leave a window pinned on top alone, like a sticky one.",
            ),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "Workspaces",
        blocks: &[
            Text(
                "A workspace saves which app is on which screen and in which zone, how each window shows, and which one had the focus, for the desk you have connected.",
            ),
            List(&[
                "**Capture current windows** saves the windows open now. Only app windows with a taskbar button count.",
                "**Switch now**, the workspace's shortcut or **Workspaces** in the tray menu puts each app back. Windows already in place aren't touched.",
                "**Open alongside**, on a workspace's menu, switches once and leaves the other windows alone, whatever **Other windows** says",
                "**Start missing apps** starts the ticked apps that aren't running, and **Other windows** leaves, minimizes or closes the rest. Close only asks, so an app with unsaved work stays open.",
                "**Show every window** brings back a window that was minimized at capture, onto its place or maximized. It counts as moved while it's minimized, so Snap back and auto-apply restore it.",
                "**Update from screen** saves the windows as they are now, and keeps your edits and the apps that aren't open, unless **Update from screen removes apps that aren't open** is on",
            ]),
            Text(
                "The table is an editor, and each change saves at once with Undo. Pick where an app goes and how it shows. The rows are the start order: drag one by its grip, or press Alt+Up and Alt+Down. Click an app's name for a delay and **Wait for this app's window before opening the next**. **Add an app** picks one from the open windows or the Start menu.",
            ),
            Text(
                "**Zones**, under the table, gives each screen the layout a switch sets before it places the apps: **Keep as is**, **No zones**, a built-in layout or one from your library. Capture and **Update from screen** record the layout each screen has. A library layout you edit changes here too.",
            ),
            Text(
                "The **Now** column shows how each app is doing. When something moved or was minimized, or a screen's layout isn't the workspace's, **Put windows back** puts it back. That's Snap back, which also has a shortcut, **Snap back to this workspace**.",
            ),
            Text(
                "**Switch back to the previous workspace** is a shortcut, and the first item of **Workspaces** in the tray menu. It goes to the workspace that was in use before this one, and pressing it again goes back.",
            ),
            Text(
                "**Put apps in place when they open** sends an app of the workspace in use to its place once, a moment after you open it yourself.",
            ),
            Text(
                "**Remember folders and pages** is off until you turn it on. Then a capture also saves the folder each Explorer window shows and the page each browser shows, and a switch that starts the app opens it there. An app that's already open is never sent anywhere. Only folders on this PC's own drives (not mapped ones) and http or https pages are kept. Private windows are skipped when their title says so. Export leaves them out, and import drops them. With it off, nothing is read, and **Forget saved folders and pages** removes the ones saved.",
            ),
            Text(
                "**Open with**, under an app's name, makes a switch that starts the app open something I type: a full path on this PC's own drives, which goes to the app as its one argument, or a link that starts with http, https, obsidian, vscode, vscode-insiders or spotify, which opens in whatever app handles it. A path is also refused for a script host such as cmd, PowerShell or Python, which would run it, though that list is no guarantee. Anything else is refused and shows under the app when the switch ends, as does an app that can't start there. It is never captured, Export leaves it out, import drops it, and only this window sees it. An app that's already open is never sent anywhere.",
            ),
            Text(
                "**Update from screen removes apps that aren't open** is off until you turn it on. Then **Update from screen** removes the apps with no window open instead of keeping them, and a notification offers Undo for 8 seconds, which puts them back in the same place, as long as you haven't changed the workspace since. An app stays when it has a window Seam didn't take, such as on another desktop, off every screen or already held by another row (a suspended app's window can count too), and nothing is removed when no app of the workspace is open. **Save windows to the active workspace** keeps them either way.",
            ),
            Text(
                "**Save windows to the active workspace** is a shortcut that saves into the workspace in use without opening Seam. Click its notification within 8 seconds to undo it.",
            ),
            Text(
                "**Add window to workspace 1** to **9** on Shortcuts add the focused window to that workspace, counted in the order of the list. A window with no row there gets a new one at the end with where it is now and Launch off, so a switch places it but never starts it. A window that already has a row there updates that row's place and keeps its launch details and order. It never takes the window out of another workspace. Seam's own windows, apps under Left alone, apps running as administrator, windows kept out of workspace switches and windows with no taskbar button are refused, and a notice says why. A notification offers Undo for 8 seconds. None has a key until you set one.",
            ),
            Text(
                "The workspace in use shows as Active in the list, and as Active, moved once one of its windows moved. **Set as default**, on a workspace's menu, makes it the one in use when Seam starts and none is remembered.",
            ),
        ],
        opens: Some(Page::Workspaces),
    },
    Topic {
        title: "Where apps open",
        blocks: &[
            Text(
                "**Where apps open**, on the Zones page, sends an app's window to a place whenever it opens, with or without a workspace. **Add an app**, then pick its screen and zone, or the full screen.",
            ),
            Text(
                "**Put apps in place when they open** on Workspaces handles the workspace in use: a window goes where that workspace's row for the app says. An app rule is for any other time. When the workspace in use has a row for an app, it owns every window of that app and the rule never applies to it.",
            ),
            Text(
                "A rule moves a window once, a moment after it opens. Dialogs, windows that open while a workspace switches, apps that run as administrator and apps under **Left alone** stay where they open, and so does a window whose rule's screen isn't connected.",
            ),
            Text(
                "**...**, **Keep on top** on an app's row pins its windows as you would by hand, once, a moment after they open. Choose **Doesn't move** as its place to only pin it. **Release pinned windows** takes the pin back, and the rule leaves that window alone until it's opened again.",
            ),
            Text("**Left alone** lists the apps Seam never manages. Remove an app there to manage it again."),
            Text(
                "**...**, **Leave out of smart tiling** on an app's row keeps its windows floating on a screen where **Arrange new windows automatically** is on. Choose **Doesn't move** as its place to only do that.",
            ),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "When you sign in",
        blocks: &[
            Text(
                "**When I sign in**, on General, picks what Seam does on this desk when it starts at sign-in: **Do nothing** (the default), **Offer my last session** or **Open a workspace**. Starting Seam by hand does none of them.",
            ),
            Text(
                "**Offer my last session** keeps a hidden last session for each desk: which apps are where, and which are pinned. At sign-in Seam asks whether to restore it. A click puts the windows back in their zones and pins them again. Nothing starts without the click.",
            ),
            Text(
                "**Open a workspace** switches to the workspace you pick, like any workspace switch. **Ask first** is on by default: Seam shows \"Open <workspace>? Click to open\" and waits for the click. Off, it switches about 10 seconds after the taskbar is up. If a full-screen app is in front it waits, up to 3 minutes after Seam starts. A pause by hand cancels it. It acts on the desk that is current then.",
            ),
            Text(
                "**Who reopens the apps** picks **Windows reopens them**, where Seam puts each window back as it appears, or **Seam reopens them**, where Seam starts the missing apps itself.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Pause and full-screen apps",
        blocks: &[
            Text(
                "**Step aside for games and full-screen apps**, on General, is on by default. While a game, a video or a slide show fills a screen, Seam gives its shortcuts back to that app, shows no zones when you drag, and turns Spotlight and the focus border off. Everything comes back when you leave it.",
            ),
            Text(
                "**Pause Seam**, in the tray menu or on a shortcut you set on Shortcuts, does the same by hand and also turns the pointer remap off and leaves new windows where they open. It isn't saved, so Seam is running again after a restart.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Shortcuts",
        blocks: &[
            Text("Shortcuts work anywhere in Windows, even with the Seam window closed."),
            List(&[
                "Ctrl+Shift+Alt+L lines up your screens",
                "Ctrl+Shift+Alt+P pauses or resumes the remap",
                "Pause Seam has no key until you set one, and pauses or resumes everything",
                "Shift+Win+Alt+arrows move the focused window, and Ctrl+Shift+Alt+arrows focus the window that way",
                "Swap, grow, shrink, two thirds, tile, next and previous zone, the pins, capture, Snap back, Switch back, quick save, Focus on this window and Spotlight have no key until you set one",
            ]),
            Text(
                "Each workspace can have its own shortcut too. Press **Change** on a row, then the keys. Esc keeps the old ones and Backspace clears them. Ctrl+Shift+Alt plus a number is a good choice, since Ctrl+Alt alone types characters on many keyboards.",
            ),
            Text("If another app already holds a shortcut, its keys turn red and Seam keeps running without it."),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Stream Deck",
        blocks: &[
            Text(
                "The Seam plugin puts Seam on Stream Deck keys. It talks to Seam through the local API, so **Let other apps control Seam** on General has to stay on.",
            ),
            List(&[
                "Switch workspace, Snap back and Save windows",
                "Line up screens, Pause or resume remap, and Zones on or off",
                "Next zone, Previous zone, Move window and Focus window",
                "Pin on top, Release pinned windows, and Desk, which shows the desk's name",
            ]),
            Text(
                "To install it, open the co.f3nn3x.seam.streamDeckPlugin file, and the Stream Deck app adds a Seam group to its actions. When Seam isn't running, every key says so and reconnects by itself.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Local API",
        blocks: &[
            Text(
                "Tools on this PC can control Seam through its local API, the way the Stream Deck plugin does. They can read status, switch workspaces, line up screens and move windows.",
            ),
            Text(
                "**Let other apps control Seam**, on General, turns it on or off, and says how many apps are connected. The Seam window has its own connection, so it keeps working either way.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Assistants (MCP)",
        blocks: &[
            Text(
                "An AI assistant that speaks MCP, such as Claude, can use Seam through seam-mcp.exe, which sits next to seam.exe. Add its path to the assistant's MCP settings as a server that runs over stdio. **Copy** on General gives you the path.",
            ),
            Text(
                "**Let assistants use Seam (MCP)**, on General, is off until you turn it on, and takes effect at once. **Let other apps control Seam** has to be on too. While it is off, the assistant is told so and can't do anything.",
            ),
            List(&[
                "It can read status and your workspaces, switch workspaces, Snap back and Switch back",
                "A switch from an assistant never closes windows",
                "**Let assistants change workspaces** lets it also Quick save, create, rename and remove rows of a workspace, change where a row goes or how it is shown, and turn on **Show every window**. Each change shows a notice you can click to undo.",
                "It can't change which programs a workspace starts, its shortcut, or what happens to other windows, and it can't close windows or change your settings",
            ]),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Updates",
        blocks: &[
            Text("About shows your version and the result of each update check."),
            List(&[
                "**Check for updates every day** looks once a day, and **Check now** looks straight away",
                "**Install updates on their own** installs only while you're away from the computer",
                "**Include pre-releases** gets new features early. They may have rough edges.",
            ]),
            Text(
                "The updater only installs releases signed with Seam's key. The Store copy is updated by the Microsoft Store.",
            ),
        ],
        opens: Some(Page::About),
    },
];
